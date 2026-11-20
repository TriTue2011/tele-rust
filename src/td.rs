//! Cầu nối tới TDLib qua giao diện JSON của `libtdjson` (giống `gateway/tdclient.py` của bản Python).
//!
//! Không dùng crate `tdlib-rs` vì nó chỉ hỗ trợ TDLib 1.8.61, cũ hơn 1.8.69 của bản Python: phiên TDLib tạo bằng bản mới
//! có thể không mở được bằng bản cũ. Thư viện được mở lúc chạy (dlopen) từ đường dẫn `TDJSON_LIB`.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int};

use libloading::Library;
use serde_json::Value;

type CreateClientId = unsafe extern "C" fn() -> c_int;
type Send = unsafe extern "C" fn(c_int, *const c_char);
type Receive = unsafe extern "C" fn(c_double) -> *const c_char;
type Execute = unsafe extern "C" fn(*const c_char) -> *const c_char;

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

    pub fn create_client_id(&self) -> i32 {
        unsafe { (self.create_client_id)() }
    }

    pub fn send(&self, client_id: i32, request: &Value) {
        let s = CString::new(request.to_string()).expect("JSON không chứa byte 0");
        unsafe { (self.send)(client_id, s.as_ptr()) }
    }

    /// Chờ tối đa `timeout` giây một bản tin bất kỳ (của mọi client). `None` khi hết giờ.
    pub fn receive(&self, timeout: f64) -> Option<Value> {
        let p = unsafe { (self.receive)(timeout) };
        if p.is_null() {
            return None;
        }
        // SAFETY: chuỗi do TDLib giữ, hợp lệ tới lần gọi td_receive kế tiếp → chép ra ngay.
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        serde_json::from_str(&s).ok()
    }

    /// Gọi đồng bộ các hàm TDLib cho phép (getOption, setLogVerbosityLevel…).
    pub fn execute(&self, request: &Value) -> Option<Value> {
        let s = CString::new(request.to_string()).expect("JSON không chứa byte 0");
        let p = unsafe { (self.execute)(s.as_ptr()) };
        if p.is_null() {
            return None;
        }
        let out = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        serde_json::from_str(&out).ok()
    }

    /// Phiên bản TDLib của thư viện đã mở (vd "1.8.69").
    pub fn version(&self) -> Option<String> {
        let r = self.execute(&serde_json::json!({"@type": "getOption", "name": "version"}))?;
        r.get("value").and_then(|v| v.as_str()).map(str::to_owned)
    }
}
