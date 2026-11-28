//! Test module `td` bằng libtdjson GIẢ (không cần thư viện thật).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};

use super::login::{self, LoginConfig};
use super::{TdApi, TdClient};
use crate::contract::TdLink;

type Responder = Box<dyn Fn(&Value) -> Option<Value> + Send + Sync>;

/// libtdjson giả: ghi lại mọi yêu cầu gửi đi; `receive` chặn trên condvar tới khi có bản tin hoặc hết giờ.
#[derive(Default)]
struct FakeApi {
    next_id: AtomicI32,
    sent: Mutex<Vec<Value>>,
    inbox: Mutex<VecDeque<Value>>,
    cv: Condvar,
    /// Tự trả lời (nếu đặt): nhận yêu cầu, trả câu trả lời (chưa có @extra — FakeApi tự chép vào).
    responder: Mutex<Option<Responder>>,
}

impl FakeApi {
    fn push(&self, v: Value) {
        self.inbox.lock().unwrap().push_back(v);
        self.cv.notify_all();
    }
    fn sent(&self) -> Vec<Value> {
        self.sent.lock().unwrap().clone()
    }
    /// Yêu cầu đã gửi có `@type` = `kind`.
    fn sent_of(&self, kind: &str) -> Vec<Value> {
        self.sent().into_iter().filter(|v| v["@type"] == kind).collect()
    }
    /// Chờ (tối đa 5 giây) tới khi đã gửi ít nhất `n` yêu cầu loại `kind`.
    async fn wait_sent(&self, kind: &str, n: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let v = self.sent_of(kind);
            if v.len() >= n {
                return v;
            }
            assert!(Instant::now() < deadline, "không thấy {n} yêu cầu {kind}; đã gửi: {:?}", self.types());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    fn types(&self) -> Vec<String> {
        self.sent().iter().map(|v| v["@type"].as_str().unwrap_or("").to_owned()).collect()
    }
    fn auth(&self, state: &str) {
        self.push(json!({"@type": "updateAuthorizationState", "authorization_state": {"@type": state}, "@client_id": 1}));
    }
}

impl TdApi for FakeApi {
    fn create_client_id(&self) -> i32 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }
    fn send(&self, client_id: i32, request: &Value) {
        self.sent.lock().unwrap().push(request.clone());
        let reply = self.responder.lock().unwrap().as_ref().and_then(|f| f(request));
        if let Some(mut r) = reply {
            r["@extra"] = request["@extra"].clone();
            r["@client_id"] = json!(client_id);
            self.push(r);
        }
    }
    fn receive(&self, timeout: f64) -> Option<Value> {
        let q = self.inbox.lock().unwrap();
        let (mut q, _) = self
            .cv
            .wait_timeout_while(q, Duration::from_secs_f64(timeout), |q| q.is_empty())
            .unwrap();
        q.pop_front()
    }
    fn execute(&self, _request: &Value) -> Option<Value> {
        None
    }
}

fn start() -> (Arc<FakeApi>, Arc<TdClient>) {
    static NEXT: AtomicI32 = AtomicI32::new(0);
    let dir = TmpDir::new(&format!("receiver-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    let api = Arc::new(FakeApi::default());
    let client = TdClient::start(super::Receiver::start(api.clone()), &dir.0).unwrap();
    (api, client)
}

fn extra_of(v: &Value) -> Value {
    v["@extra"].clone()
}

#[tokio::test]
async fn hai_client_khong_lan_phan_hoi_va_giu_khoa_phien_toi_khi_dong() {
    let dir = TmpDir::new("multi");
    let api = Arc::new(FakeApi::default());
    let receiver = super::Receiver::start(api.clone());
    let p = dir.0.join("mot");
    let a = TdClient::start(receiver.clone(), &p).unwrap();
    let b = TdClient::start(receiver.clone(), &dir.0.join("hai")).unwrap();
    assert!(TdClient::start(receiver.clone(), &p.join(".")).is_err());

    let mut au = a.updates();
    let mut bu = b.updates();
    let fetch = |c: Arc<TdClient>| tokio::spawn(async move { c.request(json!({"@type":"getMe"})).await.unwrap() });
    let (ax, bx) = (fetch(a.clone()), fetch(b.clone()));
    let sent = api.wait_sent("getMe", 2).await;
    assert_eq!(sent[0]["@extra"], sent[1]["@extra"]);
    for (client, value) in [(&b, 22), (&a, 11)] {
        api.push(json!({"@client_id":client.client_id(), "@extra":sent[0]["@extra"], "id":value}));
        api.push(json!({"@client_id":client.client_id(), "@type":"updateNewMessage", "id":value}));
    }
    let wait = Duration::from_secs(2);
    assert_eq!(tokio::time::timeout(wait, ax).await.unwrap().unwrap()["id"], 11);
    assert_eq!(tokio::time::timeout(wait, bx).await.unwrap().unwrap()["id"], 22);
    assert_eq!(tokio::time::timeout(wait, au.recv()).await.unwrap().unwrap()["id"], 11);
    assert_eq!(tokio::time::timeout(wait, bu.recv()).await.unwrap().unwrap()["id"], 22);

    let closing = a.clone();
    let task = tokio::spawn(async move { closing.close(wait).await });
    api.wait_sent("close", 1).await;
    assert!(TdClient::start(receiver.clone(), &p).is_err());
    api.auth("authorizationStateClosed");
    task.await.unwrap();
    assert!(TdClient::start(receiver, &p).is_ok());
    assert_ne!(b.auth_state(), "authorizationStateClosed");
}

#[tokio::test]
async fn khoi_dong_gui_yeu_cau_danh_thuc_td() {
    let (api, _client) = start();
    // TDLib chỉ dựng client sau td_send đầu tiên.
    api.wait_sent("getAuthorizationState", 1).await;
}

#[tokio::test]
async fn ghep_extra_dung_khi_tra_loi_lech_thu_tu_va_update_khong_lan() {
    let (api, client) = start();
    let mut updates = client.updates();
    let (c1, c2) = (client.clone(), client.clone());
    let a = tokio::spawn(async move { c1.request(json!({"@type": "getUser", "user_id": 111})).await });
    let b = tokio::spawn(async move { c2.request(json!({"@type": "getUser", "user_id": 222})).await });
    let sent = api.wait_sent("getUser", 2).await;
    let ea = extra_of(sent.iter().find(|v| v["user_id"] == 111).unwrap());
    let eb = extra_of(sent.iter().find(|v| v["user_id"] == 222).unwrap());
    assert_ne!(ea, eb);
    // Trả lời cho B trước, xen một update và một câu trả lời mồ côi (của yêu cầu khởi động), rồi mới tới A.
    api.push(json!({"@type": "user", "id": 222, "@extra": eb, "@client_id": 1}));
    api.push(json!({"@type": "updateOption", "name": "x", "@client_id": 1}));
    api.push(json!({"@type": "authorizationStateWaitTdlibParameters", "@extra": "khoi-dong", "@client_id": 1}));
    api.push(json!({"@type": "updateOption", "name": "khac-client", "@client_id": 99}));
    api.push(json!({"@type": "user", "id": 111, "@extra": ea, "@client_id": 1}));
    let ra = a.await.unwrap().unwrap();
    let rb = b.await.unwrap().unwrap();
    assert_eq!(ra["id"], 111);
    assert_eq!(rb["id"], 222);
    assert!(ra.get("@extra").is_none(), "câu trả lời không mang @extra nội bộ");
    // Dòng update chỉ có đúng MỘT bản tin: updateOption "x" (không câu trả lời nào, không bản tin client khác).
    let u = tokio::time::timeout(Duration::from_secs(2), updates.recv()).await.unwrap().unwrap();
    assert_eq!(u["@type"], "updateOption");
    assert_eq!(u["name"], "x");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(matches!(updates.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)));
}

#[tokio::test]
async fn loi_tdlib_thanh_err_message() {
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(Box::new(|req| {
        (req["@type"] == "getMe").then(|| json!({"@type": "error", "code": 401, "message": "Unauthorized"}))
    }));
    let r = client.request(json!({"@type": "getMe"})).await;
    assert_eq!(r, Err(crate::contract::TdError::telegram(401, "Unauthorized")));
}

#[tokio::test]
async fn dong_tdlib_ngat_yeu_cau_dang_cho_va_tu_choi_yeu_cau_moi() {
    let (api, client) = start();
    let mut first = client.updates();
    let mut second = client.updates();
    let c = client.clone();
    let task = tokio::spawn(async move { c.request(json!({"@type": "getMe"})).await });
    api.wait_sent("getMe", 1).await;
    api.auth("authorizationStateClosed");
    let error = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap_err();
    assert_eq!(error.code, None);
    assert!(error.message.contains("mất kết nối TDLib"));
    let update = tokio::time::timeout(Duration::from_secs(2), first.recv()).await.unwrap().unwrap();
    assert_eq!(update["authorization_state"]["@type"], "authorizationStateClosed");
    assert_eq!(second.recv().await.unwrap(), update, "mọi subscriber nhận một bản sao");
    assert_eq!(client.auth_state(), "authorizationStateClosed");
    assert!(client.request(json!({"@type": "getMe"})).await.is_err());
    assert_eq!(api.sent_of("getMe").len(), 1);
}

#[tokio::test]
async fn huy_yeu_cau_go_nguoi_cho_va_khong_phat_tra_loi_muon() {
    let (api, client) = start();
    let mut updates = client.updates();
    let c = client.clone();
    let task = tokio::spawn(async move { c.request(json!({"@type": "getMe"})).await });
    let sent = api.wait_sent("getMe", 1).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(client.shared.pending.lock().unwrap().as_ref().unwrap().is_empty());
    api.push(json!({"@type": "user", "@extra": extra_of(&sent[0]), "@client_id": 1}));
    api.push(json!({"@type": "updateOption", "name": "sau-huy", "@client_id": 1}));
    let update = tokio::time::timeout(Duration::from_secs(2), updates.recv()).await.unwrap().unwrap();
    assert_eq!(update["@type"], "updateOption");
    assert_eq!(update["name"], "sau-huy");
}

#[tokio::test]
async fn het_gio_tra_err_va_tra_loi_muon_khong_lan_vao_update() {
    let (api, client) = start();
    let mut updates = client.updates();
    let r = client.request_timeout(json!({"@type": "getMe"}), Duration::from_millis(100)).await;
    assert!(r.unwrap_err().message.contains("getMe"));
    let e = extra_of(&api.sent_of("getMe")[0]);
    api.push(json!({"@type": "user", "@extra": e, "@client_id": 1}));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(updates.try_recv().is_err());
}

#[tokio::test]
async fn trang_thai_dang_nhap_doc_duoc() {
    let (api, client) = start();
    assert_eq!(client.auth_state(), "");
    api.auth("authorizationStateWaitPhoneNumber");
    let mut w = client.auth_watch();
    tokio::time::timeout(Duration::from_secs(2), w.wait_for(|s| s == "authorizationStateWaitPhoneNumber"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(client.auth_state(), "authorizationStateWaitPhoneNumber");
}

#[tokio::test]
async fn close_chi_gui_mot_lan_va_cho_trang_thai_closed() {
    let (api, client) = start();
    let closing = client.clone();
    let task = tokio::spawn(async move { closing.close(Duration::from_secs(2)).await; });
    api.wait_sent("close", 1).await;
    assert_eq!(api.sent_of("close").len(), 1);
    api.auth("authorizationStateClosed");
    tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
    assert_eq!(client.auth_state(), "authorizationStateClosed");
    assert_eq!(api.sent_of("close").len(), 1);
}

struct TmpDir(PathBuf);
impl TmpDir {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("tggw-td-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cfg(dir: &TmpDir) -> LoginConfig {
    LoginConfig { data_dir: dir.0.clone(), api_id: 12345, api_hash: "hash-bi-mat".into(), phone: "+84900000000".into() }
}

#[test]
fn cau_hinh_thieu_tu_choi_bang_tieng_viet_khong_lo_gia_tri_bi_mat() {
    let old_api_id = std::env::var_os("TG_API_ID");
    let old_api_hash = std::env::var_os("TG_API_HASH");
    std::env::remove_var("TG_API_ID");
    std::env::set_var("TG_API_HASH", "api-hash-khong-lo");

    let error = LoginConfig::from_env().unwrap_err();

    match old_api_id {
        Some(value) => std::env::set_var("TG_API_ID", value),
        None => std::env::remove_var("TG_API_ID"),
    }
    match old_api_hash {
        Some(value) => std::env::set_var("TG_API_HASH", value),
        None => std::env::remove_var("TG_API_HASH"),
    }
    assert!(error.contains("TG_API_ID") && error.contains("thiếu"), "{error}");
    assert!(!error.contains("api-hash-khong-lo"), "{error}");
}

/// Trả lời `ok` cho mọi yêu cầu, trừ mã "11111" (sai) → PHONE_CODE_INVALID.
fn ok_responder() -> Responder {
    Box::new(|req| {
        if req["@type"] == "checkAuthenticationCode" && req["code"] == "11111" {
            return Some(json!({"@type": "error", "code": 400, "message": "PHONE_CODE_INVALID"}));
        }
        (req["@extra"] != "khoi-dong").then(|| json!({"@type": "ok"}))
    })
}

#[tokio::test]
async fn dang_nhap_gui_dung_yeu_cau_tung_trang_thai() {
    let dir = TmpDir::new("tung-buoc");
    let cfg = cfg(&dir);
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(ok_responder());
    let (c, k) = (client.clone(), cfg.clone());
    let task = tokio::spawn(async move { login::run(&c, &k).await });

    // 1) waitTdlibParameters → setTdlibParameters phẳng, giống bản Python.
    api.auth("authorizationStateWaitTdlibParameters");
    let p = api.wait_sent("setTdlibParameters", 1).await.remove(0);
    let files = dir.0.join("tdlib/files").to_string_lossy().into_owned();
    assert_eq!(p["api_id"], json!(12345));
    assert!(p["api_id"].is_i64(), "api_id phải là số");
    assert_eq!(p["api_hash"], "hash-bi-mat");
    assert_eq!(p["database_directory"], files);
    assert_eq!(p["files_directory"], files);
    for k in ["use_file_database", "use_chat_info_database", "use_message_database"] {
        assert_eq!(p[k], true, "{k}");
    }
    assert_eq!(p["use_secret_chats"], false);
    assert_eq!(p["use_test_dc"], false);
    assert_eq!(p["system_language_code"], "en");
    assert_eq!(p["device_model"], "telegram-gateway");
    assert_eq!(p["system_version"], "Linux");
    assert_eq!(p["application_version"], "0.1.0");
    assert!(p.get("parameters").is_none(), "không lồng `parameters`");
    let khoa = std::fs::read_to_string(dir.0.join("tdlib/khoa")).unwrap();
    assert_eq!(khoa.len(), 32);
    use base64::Engine as _;
    let key = base64::engine::general_purpose::STANDARD.encode(login::decode_key(&khoa));
    assert_eq!(p["database_encryption_key"], key);
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.0.join("tdlib/khoa")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // 2) waitPhoneNumber → số cấu hình.
    api.auth("authorizationStateWaitPhoneNumber");
    let s = api.wait_sent("setAuthenticationPhoneNumber", 1).await;
    assert_eq!(s[0]["phone_number"], "+84900000000");

    // 3) waitCode → mã từ tệp được đọc và xoá.
    api.auth("authorizationStateWaitCode");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(api.sent_of("checkAuthenticationCode").is_empty());
    let code_file = dir.0.join("login_code.txt");
    std::fs::write(&code_file, "22222\n").unwrap();
    let s = api.wait_sent("checkAuthenticationCode", 1).await;
    assert_eq!(s[0]["code"], "22222");
    assert!(!code_file.exists(), "tệp mã phải bị xoá sau khi đọc");

    // 4) waitPassword → tệp mật khẩu.
    api.auth("authorizationStateWaitPassword");
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::write(dir.0.join("login_password.txt"), " mk-tep \r\n").unwrap();
    let s = api.wait_sent("checkAuthenticationPassword", 1).await;
    assert_eq!(s[0]["password"], " mk-tep ");

    // 5) ready → xong.
    api.auth("authorizationStateReady");
    let r = tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    assert_eq!(r, Ok(()));
    assert_eq!(client.auth_state(), "authorizationStateReady");
    // Mỗi bước đúng MỘT yêu cầu (không lặp setTdlibParameters / số điện thoại).
    assert_eq!(api.sent_of("setTdlibParameters").len(), 1);
    assert_eq!(api.sent_of("setAuthenticationPhoneNumber").len(), 1);
}

#[tokio::test]
async fn ma_sai_ket_thuc_dang_nhap_va_khong_gui_lai() {
    let dir = TmpDir::new("ma-sai");
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(ok_responder());
    let (c, k) = (client.clone(), cfg(&dir));
    let task = tokio::spawn(async move { login::run(&c, &k).await });
    api.auth("authorizationStateWaitCode");
    client.submit_code("11111");
    api.wait_sent("checkAuthenticationCode", 1).await;
    let result = tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    assert!(result.unwrap_err().contains("PHONE_CODE_INVALID"));
    assert_eq!(api.sent_of("checkAuthenticationCode").len(), 1, "mã sai không bị gửi lại");
}

#[tokio::test]
async fn mat_khau_qua_submit_password() {
    let dir = TmpDir::new("mat-khau");
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(ok_responder());
    let (c, k) = (client.clone(), cfg(&dir));
    let task = tokio::spawn(async move { login::run(&c, &k).await });
    api.auth("authorizationStateWaitPassword");
    client.submit_password(" mk-ham ");
    let s = api.wait_sent("checkAuthenticationPassword", 1).await;
    assert_eq!(s[0]["password"], " mk-ham ");
    api.auth("authorizationStateReady");
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap(), Ok(()));
}

#[tokio::test]
async fn doi_trang_thai_khi_cho_ma_va_khong_gui_lai_so_dien_thoai() {
    let dir = TmpDir::new("doi-trang-thai");
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(ok_responder());
    let (c, k) = (client.clone(), cfg(&dir));
    let task = tokio::spawn(async move { login::run(&c, &k).await });
    api.auth("authorizationStateWaitPhoneNumber");
    api.wait_sent("setAuthenticationPhoneNumber", 1).await;
    api.auth("authorizationStateWaitPhoneNumber");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(api.sent_of("setAuthenticationPhoneNumber").len(), 1);
    api.auth("authorizationStateWaitCode");
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Phiên được xác thực từ nơi khác trong lúc chờ mã: không được treo trong submit().
    api.auth("authorizationStateReady");
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap(), Ok(()));
    assert!(api.sent_of("checkAuthenticationCode").is_empty());
}

#[tokio::test]
async fn phien_da_dang_nhap_khong_can_gui_lai_thong_tin() {
    let dir = TmpDir::new("phien-san");
    let (api, client) = start();
    api.auth("authorizationStateReady");
    assert_eq!(tokio::time::timeout(Duration::from_secs(2), login::run(&client, &cfg(&dir))).await.unwrap(), Ok(()));
    assert_eq!(api.types(), vec!["getAuthorizationState"]);
}

#[tokio::test]
async fn tep_ma_cu_bi_bo_qua() {
    let dir = TmpDir::new("ma-cu");
    let code_file = dir.0.join("login_code.txt");
    std::fs::write(&code_file, "99999").unwrap();
    let f = std::fs::File::options().write(true).open(&code_file).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
    drop(f);
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(ok_responder());
    let (c, k) = (client.clone(), cfg(&dir));
    let task = tokio::spawn(async move { login::run(&c, &k).await });
    api.auth("authorizationStateWaitCode");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(api.sent_of("checkAuthenticationCode").is_empty(), "tệp mã cũ không được gửi");
    assert!(!code_file.exists());
    client.submit_code("33333");
    assert_eq!(api.wait_sent("checkAuthenticationCode", 1).await[0]["code"], "33333");
    task.abort();
}

#[tokio::test]
async fn so_dien_thoai_bi_tu_choi_thi_dung_khong_thu_lai() {
    let dir = TmpDir::new("flood");
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(Box::new(|req| {
        (req["@type"] == "setAuthenticationPhoneNumber")
            .then(|| json!({"@type": "error", "code": 429, "message": "FLOOD_WAIT_300"}))
    }));
    api.auth("authorizationStateWaitPhoneNumber");
    let r = tokio::time::timeout(Duration::from_secs(3), login::run(&client, &cfg(&dir))).await.unwrap();
    assert!(r.unwrap_err().contains("FLOOD_WAIT_300"));
    assert_eq!(api.sent_of("setAuthenticationPhoneNumber").len(), 1);
}

#[tokio::test]
async fn thieu_so_dien_thoai_thi_err() {
    let dir = TmpDir::new("thieu-so");
    let (api, client) = start();
    api.auth("authorizationStateWaitPhoneNumber");
    let mut k = cfg(&dir);
    k.phone.clear();
    let r = tokio::time::timeout(Duration::from_secs(3), login::run(&client, &k)).await.unwrap();
    assert!(r.unwrap_err().contains("TG_PHONE"));
    assert!(api.sent_of("setAuthenticationPhoneNumber").is_empty());
}

#[tokio::test]
async fn loi_dang_nhap_khong_lo_api_hash() {
    let dir = TmpDir::new("loi-bi-mat");
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(Box::new(|req| {
        (req["@type"] == "setTdlibParameters").then(|| json!({
            "@type": "error", "code": 400, "message": "invalid api_hash: hash-bi-mat"
        }))
    }));
    api.auth("authorizationStateWaitTdlibParameters");
    let error = tokio::time::timeout(Duration::from_secs(3), login::run(&client, &cfg(&dir)))
        .await.unwrap().unwrap_err();
    assert!(error.contains("setTdlibParameters bị từ chối"));
    assert!(!error.contains("hash-bi-mat"), "{error}");
}

#[test]
fn khoa_giai_ma_nhu_python() {
    assert_eq!(login::decode_key("00ff10"), vec![0x00, 0xff, 0x10]);
    assert_eq!(login::decode_key("khong-hex"), b"khong-hex".to_vec());
    assert_eq!(login::decode_key("abc"), b"abc".to_vec()); // lẻ ký tự → không phải hex
    assert!(login::decode_key("").is_empty());
}

#[test]
fn khoa_co_san_khong_bi_ghi_de() {
    let dir = TmpDir::new("khoa-san");
    let k = cfg(&dir);
    std::fs::create_dir_all(k.session_dir()).unwrap();
    std::fs::write(k.key_path(), "0102").unwrap();
    assert_eq!(login::ensure_session(&k).unwrap(), vec![1, 2]);
    assert_eq!(std::fs::read_to_string(k.key_path()).unwrap(), "0102");
}

#[test]
fn debug_cau_hinh_khong_lo_bi_mat() {
    let dir = TmpDir::new("debug");
    let s = format!("{:?}", cfg(&dir));
    assert!(!s.contains("hash-bi-mat") && !s.contains("84900000000"), "{s}");
}

#[tokio::test]
async fn verify_http_waits_past_stale_password_state_and_fails_fast_on_closed() {
    let (api, client) = start();
    api.auth("authorizationStateWaitPassword");
    let mut w = client.auth_watch();
    tokio::time::timeout(Duration::from_secs(2), w.wait_for(|s| s == "authorizationStateWaitPassword"))
        .await
        .unwrap()
        .unwrap();
    let api_clone = api.clone();
    *api.responder.lock().unwrap() = Some(Box::new(move |req| {
        if req["@extra"] == "khoi-dong" {
            return None;
        }
        match req["@type"].as_str() {
            Some("checkAuthenticationPassword") => {
                let api_delayed = api_clone.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(50));
                    api_delayed.auth("authorizationStateReady");
                });
                Some(json!({"@type": "ok"}))
            }
            Some("getAuthorizationState") => Some(json!({"@type": "authorizationStateWaitPassword"})),
            _ => None,
        }
    }));
    let state = tokio::time::timeout(
        Duration::from_secs(2),
        login::verify_http(&client, "checkAuthenticationPassword", "password", "pw"),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(state, "authorizationStateReady");

    let (api2, client2) = start();
    *api2.responder.lock().unwrap() = Some(Box::new(|req| {
        if req["@extra"] == "khoi-dong" {
            return None;
        }
        match req["@type"].as_str() {
            Some("checkAuthenticationCode") => Some(json!({"@type": "ok"})),
            Some("getAuthorizationState") => Some(json!({"@type": "authorizationStateClosed"})),
            _ => None,
        }
    }));
    let res = tokio::time::timeout(
        Duration::from_secs(1),
        login::verify_http(&client2, "checkAuthenticationCode", "code", "11111"),
    )
    .await
    .unwrap();
    assert!(res.is_err());
}

#[tokio::test]
async fn run_in_does_not_resend_phone_after_code_expires() {
    let dir = TmpDir::new("resend-phone");
    let (api, client) = start();
    *api.responder.lock().unwrap() = Some(ok_responder());
    let (c, k) = (client.clone(), cfg(&dir));
    let task = tokio::spawn(async move { login::run(&c, &k).await });
    api.auth("authorizationStateWaitPhoneNumber");
    api.wait_sent("setAuthenticationPhoneNumber", 1).await;
    api.auth("authorizationStateWaitCode");
    tokio::time::sleep(Duration::from_millis(50)).await;
    api.auth("authorizationStateWaitPhoneNumber");
    let res = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
    assert!(res.is_err(), "{res:?}");
    assert_eq!(api.sent_of("setAuthenticationPhoneNumber").len(), 1);
}

#[test]
fn ensure_session_dir_is_safe_under_concurrent_creation() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TmpDir::new("concurrent-key");
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8_usize)
        .map(|_| {
            let path = dir.0.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                login::ensure_session_dir(&path)
            })
        })
        .collect();
    let keys: Vec<Vec<u8>> = handles.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
    assert_eq!(keys[0].len(), 16);
    for key in &keys[1..] {
        assert_eq!(key, &keys[0]);
    }
    let mode = std::fs::metadata(dir.0.join("khoa")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    for entry in std::fs::read_dir(&dir.0).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(!name.to_string_lossy().starts_with(".khoa.tmp."), "{name:?}");
    }
}
