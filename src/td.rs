//! Cầu nối tới TDLib qua giao diện JSON của `libtdjson` (giống `gateway/tdclient.py` của bản Python).
//!
//! Không dùng crate `tdlib-rs` vì nó chỉ hỗ trợ TDLib 1.8.61, cũ hơn 1.8.69 của bản Python: phiên TDLib tạo bằng bản mới
//! có thể không mở được bằng bản cũ. Thư viện được mở lúc chạy (dlopen) từ đường dẫn `TDJSON_LIB`.
//!
//! Cấu trúc:
//! - [`TdApi`]   : bốn hàm C thô (create_client_id/send/receive/execute). [`TdJson`] là bản thật; test dùng bản giả.
//! - [`TdClient`]: bộ nhận chung gọi `td_receive`, phân phối theo `@client_id`, ghép câu trả lời theo `@extra`,
//!   theo dõi trạng thái đăng nhập → hiện thực [`crate::contract::TdLink`].
//! - [`login`]   : máy trạng thái đăng nhập (setTdlibParameters → số điện thoại → mã → mật khẩu → ready).

pub mod login;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use libloading::Library;
use serde_json::{json, Value};
use tokio::sync::{broadcast, oneshot, watch, Notify};

use crate::contract::{BoxFut, TdError, TdLink};

type CreateClientId = unsafe extern "C" fn() -> c_int;
type Send = unsafe extern "C" fn(c_int, *const c_char);
type Receive = unsafe extern "C" fn(c_double) -> *const c_char;
type Execute = unsafe extern "C" fn(*const c_char) -> *const c_char;

/// Bốn hàm C thô của libtdjson. Tách thành trait để test không cần thư viện thật.
/// `receive` chỉ được gọi từ MỘT luồng tại một thời điểm (TdClient bảo đảm điều đó).
trait TdApi: std::marker::Send + Sync + 'static {
    fn create_client_id(&self) -> i32;
    fn send(&self, client_id: i32, request: &Value);
    /// Chờ tối đa `timeout` giây một bản tin bất kỳ (của mọi client). `None` khi hết giờ.
    fn receive(&self, timeout: f64) -> Option<Value>;
    fn execute(&self, request: &Value) -> Option<Value>;
}

/// Thư viện libtdjson đã mở. TDLib cho phép gọi các hàm này từ nhiều luồng, trừ `td_receive` chỉ một luồng tại một thời điểm.
pub struct TdJson {
    _lib: Library,
    create_client_id: CreateClientId,
    send: Send,
    receive: Receive,
    execute: Execute,
}

impl TdJson {
    /// Mở thư viện ở `path`. Lỗi khi không mở được hoặc thiếu hàm.
    pub fn open(path: &str) -> Result<Self, String> {
        // SAFETY: nạp thư viện chia sẻ do người vận hành chỉ định (biến môi trường, không đến từ mạng).
        let lib = unsafe { Library::new(path) }.map_err(|e| format!("không mở được {path}: {e}"))?;
        unsafe {
            let create_client_id = *lib
                .get::<CreateClientId>(b"td_create_client_id\0")
                .map_err(|e| e.to_string())?;
            let send = *lib.get::<Send>(b"td_send\0").map_err(|e| e.to_string())?;
            let receive = *lib.get::<Receive>(b"td_receive\0").map_err(|e| e.to_string())?;
            let execute = *lib.get::<Execute>(b"td_execute\0").map_err(|e| e.to_string())?;
            Ok(Self { _lib: lib, create_client_id, send, receive, execute })
        }
    }

    /// Phiên bản TDLib của thư viện đã mở (vd "1.8.69").
    pub fn version(&self) -> Option<String> {
        let r = TdApi::execute(self, &json!({"@type": "getOption", "name": "version"}))?;
        r.get("value").and_then(|v| v.as_str()).map(str::to_owned)
    }

    /// Gọi đồng bộ các hàm TDLib cho phép (getOption, setLogVerbosityLevel…).
    pub fn execute(&self, request: &Value) -> Option<Value> {
        TdApi::execute(self, request)
    }
}

impl TdApi for TdJson {
    fn create_client_id(&self) -> i32 {
        unsafe { (self.create_client_id)() }
    }

    fn send(&self, client_id: i32, request: &Value) {
        let s = CString::new(request.to_string()).expect("JSON không chứa byte 0");
        unsafe { (self.send)(client_id, s.as_ptr()) }
    }

    fn receive(&self, timeout: f64) -> Option<Value> {
        let p = unsafe { (self.receive)(timeout) };
        if p.is_null() {
            return None;
        }
        // SAFETY: chuỗi do TDLib giữ, hợp lệ tới lần gọi td_receive kế tiếp → chép ra ngay.
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        serde_json::from_str(&s).ok()
    }

    /// Gọi đồng bộ các hàm TDLib cho phép (getOption, setLogVerbosityLevel…).
    fn execute(&self, request: &Value) -> Option<Value> {
        let s = CString::new(request.to_string()).expect("JSON không chứa byte 0");
        let p = unsafe { (self.execute)(s.as_ptr()) };
        if p.is_null() {
            return None;
        }
        let out = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        serde_json::from_str(&out).ok()
    }
}

/// Thời gian chờ một câu trả lời (bằng bản Python `TdlibClient.call(timeout=30)`).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Sức chứa hàng update; người nhận chậm quá sẽ gặp `RecvError::Lagged`.
const UPDATES_CAPACITY: usize = 4096;
/// Mỗi lần `td_receive` chờ tối đa bao lâu (giây) — cũng là thời gian tối đa để luồng nhận thấy lệnh dừng.
const RECEIVE_TIMEOUT: f64 = 1.0;

type Pending = Mutex<Option<HashMap<String, oneshot::Sender<Value>>>>;

/// Huỷ future (ví dụ REST ngắt kết nối) cũng phải gỡ người chờ khỏi bảng.
struct PendingRequest<'a> {
    pending: &'a Pending,
    key: String,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.lock().unwrap().as_mut() {
            pending.remove(&self.key);
        }
    }
}

/// Phần dùng chung giữa TdClient và luồng nhận.
struct Shared {
    session: PathBuf,
    /// `@extra` → người đang chờ. `None` = luồng nhận đã dừng (yêu cầu mới trả Err ngay).
    pending: Pending,
    updates: broadcast::Sender<Value>,
    /// `@type` của trạng thái đăng nhập mới nhất (rỗng khi TDLib chưa báo).
    auth: watch::Sender<String>,
}

impl Shared {
    /// Phân loại MỘT bản tin từ td_receive: câu trả lời (có `@extra`) → đúng người chờ; còn lại là update → broadcast.
    fn route(&self, mut msg: Value, closed: bool) {
        if let Some(extra) = msg.get("@extra").cloned() {
            let key = extra_key(&extra);
            let waiter = self.pending.lock().unwrap().as_mut().and_then(|m| m.remove(&key));
            if let Some(tx) = waiter {
                if let Some(o) = msg.as_object_mut() {
                    o.remove("@extra");
                    o.remove("@client_id");
                }
                let _ = tx.send(msg);
            }
            // Câu trả lời không còn ai chờ (đã hết giờ) → bỏ, KHÔNG lẫn vào dòng update.
            return;
        }
        if msg.get("@type").and_then(Value::as_str) == Some("updateAuthorizationState") {
            let state = msg
                .pointer("/authorization_state/@type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if closed {
                self.pending.lock().unwrap().take();
            }
            self.auth.send_if_modified(|current| {
                if *current == state { return false; }
                *current = state;
                true
            });
        }
        let _ = self.updates.send(msg); // không ai đăng ký thì thôi
    }

}

fn extra_key(extra: &Value) -> String {
    match extra {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

static RECEIVER: Mutex<Option<(String, Arc<Receiver>)>> = Mutex::new(None);

#[derive(Default)]
struct Dispatch {
    clients: Mutex<HashMap<i64, Arc<Shared>>>,
    stop: AtomicBool,
}

struct Receiver {
    api: Arc<dyn TdApi>,
    dispatch: Arc<Dispatch>,
    thread: Option<JoinHandle<()>>,
}

impl Receiver {
    fn start(api: Arc<dyn TdApi>) -> Arc<Self> {
        // Tắt nhật ký TDLib: lỗi gốc có thể chứa dữ liệu đăng nhập.
        api.execute(&json!({"@type": "setLogVerbosityLevel", "new_verbosity_level": 0}));
        let dispatch = Arc::new(Dispatch::default());
        let d = dispatch.clone();
        let a = api.clone();
        let thread = std::thread::Builder::new().name("td-receive".into()).spawn(move || {
            while !d.stop.load(Ordering::Relaxed) {
                let Some(msg) = a.receive(RECEIVE_TIMEOUT) else { continue };
                let Some(id) = msg["@client_id"].as_i64() else { continue };
                let closed = msg["@type"] == "updateAuthorizationState"
                    && msg.pointer("/authorization_state/@type").and_then(Value::as_str) == Some("authorizationStateClosed");
                let client = {
                    let mut clients = d.clients.lock().unwrap();
                    if closed { clients.remove(&id) } else { clients.get(&id).cloned() }
                };
                if let Some(client) = client { client.route(msg, closed); }
            }
        }).expect("không tạo được luồng nhận TDLib");
        Arc::new(Self { api, dispatch, thread: Some(thread) })
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.dispatch.stop.store(true, Ordering::Relaxed);
        let _ = self.thread.take().unwrap().join();
    }
}

/// Một client TDLib cho một thư mục phiên, dùng chung luồng nhận của tiến trình.
pub struct TdClient {
    receiver: Arc<Receiver>,
    shared: Arc<Shared>,
    client_id: i32,
    next_extra: AtomicU64,
    closing: OnceLock<()>,
    pub(crate) code: Secret,
    pub(crate) password: Secret,
}

/// Ô chứa mã/mật khẩu do người dùng gửi (không bao giờ in ra).
#[derive(Default)]
pub(crate) struct Secret {
    slot: Mutex<Option<String>>,
    notify: Notify,
}

impl Secret {
    fn put(&self, v: &str) {
        *self.slot.lock().unwrap() = Some(v.to_owned());
        self.notify.notify_one();
    }
    pub(crate) fn take(&self) -> Option<String> {
        self.slot.lock().unwrap().take().filter(|s| !s.is_empty())
    }
    pub(crate) async fn notified(&self) {
        self.notify.notified().await
    }
}

impl TdClient {
    /// Mở một phiên. Không được mở lại cùng thư mục trước khi TDLib xác nhận đã đóng.
    pub fn open(path: &str, session: &Path) -> Result<Arc<Self>, String> {
        let receiver = {
            let mut slot = RECEIVER.lock().unwrap();
            match slot.as_ref() {
                Some((loaded, r)) if loaded == path => r.clone(),
                Some(_) => return Err("tiến trình đã nạp một thư viện TDLib khác".into()),
                None => {
                    let r = Receiver::start(Arc::new(TdJson::open(path)?));
                    *slot = Some((path.to_owned(), r.clone()));
                    r
                }
            }
        };
        Self::start(receiver, session)
    }

    /// Tạo client, khởi động luồng nhận và «đánh thức» TDLib (client chỉ thật sự chạy sau lần td_send đầu tiên).
    fn start(receiver: Arc<Receiver>, session: &Path) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(session).map_err(|e| format!("không tạo được thư mục phiên: {e}"))?;
        let session = session.canonicalize().map_err(|e| format!("không đọc được thư mục phiên: {e}"))?;
        let mut clients = receiver.dispatch.clients.lock().unwrap();
        if clients.values().any(|c| c.session == session) {
            return Err("đã mở thư mục phiên TDLib này".into());
        }
        let client_id = receiver.api.create_client_id();
        let (updates, _) = broadcast::channel(UPDATES_CAPACITY);
        let (auth, _) = watch::channel(String::new());
        let shared = Arc::new(Shared {
            session,
            pending: Mutex::new(Some(HashMap::new())),
            updates,
            auth,
        });
        clients.insert(i64::from(client_id), shared.clone());
        drop(clients);
        // Yêu cầu vô hại để TDLib dựng client → phát updateAuthorizationState đầu tiên. Không dùng câu trả lời của nó
        // để đặt trạng thái (trả lời có thể đến sau update mới hơn → trạng thái lùi).
        receiver.api.send(client_id, &json!({"@type": "getAuthorizationState", "@extra": "khoi-dong"}));
        Ok(Arc::new(Self {
            receiver,
            shared,
            client_id,
            next_extra: AtomicU64::new(1),
            closing: OnceLock::new(),
            code: Secret::default(),
            password: Secret::default(),
        }))
    }

    pub fn client_id(&self) -> i32 {
        self.client_id
    }

    /// `@type` của trạng thái đăng nhập mới nhất (vd "authorizationStateWaitCode"); rỗng khi TDLib chưa báo.
    pub fn auth_state(&self) -> String {
        self.shared.auth.borrow().clone()
    }

    /// Theo dõi thay đổi trạng thái đăng nhập (không bao giờ lỡ giá trị mới nhất).
    pub fn auth_watch(&self) -> watch::Receiver<String> {
        self.shared.auth.subscribe()
    }

    /// Nộp mã đăng nhập (máy trạng thái dùng khi đang ở waitCode).
    pub fn submit_code(&self, code: &str) {
        self.code.put(code.trim());
    }

    /// Nộp mật khẩu 2FA (dùng khi đang ở waitPassword).
    pub fn submit_password(&self, password: &str) {
        self.password.put(password);
    }

    /// Gửi yêu cầu, chờ câu trả lời cùng `@extra` (tối đa `timeout`). `Err` khi TDLib trả error, hết giờ hoặc luồng nhận dừng.
    pub async fn request_timeout(&self, mut req: Value, timeout: Duration) -> Result<Value, TdError> {
        let kind = req.get("@type").and_then(Value::as_str).unwrap_or("?").to_owned();
        let Some(obj) = req.as_object_mut() else {
            return Err(TdError::transport("yêu cầu TDLib phải là đối tượng JSON"));
        };
        let extra = format!("r{}", self.next_extra.fetch_add(1, Ordering::Relaxed));
        obj.insert("@extra".into(), Value::String(extra.clone()));
        let (tx, rx) = oneshot::channel();
        match self.shared.pending.lock().unwrap().as_mut() {
            Some(m) => m.insert(extra.clone(), tx),
            None => return Err(TdError::transport("mất kết nối TDLib (luồng nhận đã dừng)")),
        };
        let _pending = PendingRequest { pending: &self.shared.pending, key: extra };
        self.receiver.api.send(self.client_id, &req);
        let reply = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => return Err(TdError::transport("mất kết nối TDLib (luồng nhận đã dừng)")),
            Err(_) => {
                return Err(TdError::transport(format!("TDLib không trả lời {kind} sau {} giây", timeout.as_secs())));
            }
        };
        if reply.get("@type").and_then(Value::as_str) == Some("error") {
            let code = reply.get("code").and_then(Value::as_i64).unwrap_or(400);
            let msg = reply.get("message").and_then(Value::as_str).unwrap_or_default();
            return Err(TdError::telegram(code, msg));
        }
        Ok(reply)
    }

    /// Đóng client và chờ `authorizationStateClosed` (TDLib ghi xong DB, nhả khoá td.binlog), tối đa `wait`.
    pub async fn close(&self, wait: Duration) {
        let mut rx = self.auth_watch();
        self.send_close();
        let _ = tokio::time::timeout(wait, rx.wait_for(|s| s == "authorizationStateClosed")).await;
    }

    fn send_close(&self) {
        self.closing.get_or_init(|| {
            if self.auth_state() != "authorizationStateClosed" {
                self.receiver.api.send(self.client_id, &json!({"@type": "close", "@extra": "dong"}));
            }
        });
    }
}

impl Drop for TdClient {
    fn drop(&mut self) {
        self.send_close();
    }
}

impl TdLink for TdClient {
    fn request(&self, req: Value) -> BoxFut<'_, Result<Value, TdError>> {
        let timeout = if req["@type"] == "downloadFile" { Duration::from_secs(60) } else { REQUEST_TIMEOUT };
        Box::pin(self.request_timeout(req, timeout))
    }

    fn updates(&self) -> broadcast::Receiver<Value> {
        self.shared.updates.subscribe()
    }
}
