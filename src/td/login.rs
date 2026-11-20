//! Máy trạng thái đăng nhập TDLib, chạy theo `updateAuthorizationState` (giống `tdclient.py` + `login.py` bản Python):
//!
//! waitTdlibParameters → `setTdlibParameters` (tham số PHẲNG, y hệt bản Python) → waitPhoneNumber → số cấu hình →
//! waitCode → mã từ [`TdClient::submit_code`] hoặc tệp `<data>/login_code.txt` (đọc mỗi giây) → waitPassword →
//! [`TdClient::submit_password`] hoặc `<data>/login_password.txt` → ready.
//!
//! Không bao giờ in mã, mật khẩu, api_hash hay khoá DB. Không tự thử lại số điện thoại (FLOOD_WAIT có thể khoá tài khoản).

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::watch;

use super::{Secret, TdClient};
use crate::contract::TdLink;

/// Cấu hình đăng nhập. `Debug` che api_hash và số điện thoại.
#[derive(Clone)]
pub struct LoginConfig {
    /// Thư mục dữ liệu (`TGGW_DATA`). Phiên TDLib ở `<data>/tdlib` (cùng bố cục một thư mục phiên của bản Python).
    pub data_dir: PathBuf,
    pub api_id: i32,
    pub api_hash: String,
    /// Rỗng được nếu phiên đã đăng nhập sẵn (TDLib đi thẳng tới ready).
    pub phone: String,
}

impl fmt::Debug for LoginConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginConfig")
            .field("data_dir", &self.data_dir)
            .field("api_id", &self.api_id)
            .field("api_hash", &"***")
            .field("phone", &if self.phone.is_empty() { "" } else { "***" })
            .finish()
    }
}

impl LoginConfig {
    /// Đọc `TGGW_DATA` (mặc định `/data` như `DATA_DIR` của bản Python), `TG_API_ID`, `TG_API_HASH`, `TG_PHONE`.
    pub fn from_env() -> Result<Self, String> {
        let data_dir = PathBuf::from(std::env::var("TGGW_DATA").unwrap_or_else(|_| "/data".into()));
        let api_id = std::env::var("TG_API_ID").map_err(|_| "thiếu TG_API_ID".to_owned())?;
        let api_id = api_id.trim().parse::<i32>().map_err(|_| "TG_API_ID phải là số nguyên".to_owned())?;
        let api_hash = std::env::var("TG_API_HASH").unwrap_or_default().trim().to_owned();
        if api_hash.is_empty() {
            return Err("thiếu TG_API_HASH".into());
        }
        let phone = std::env::var("TG_PHONE").unwrap_or_default().trim().to_owned();
        Ok(Self { data_dir, api_id, api_hash, phone })
    }

    pub fn session_dir(&self) -> PathBuf {
        self.data_dir.join("tdlib")
    }
    pub fn files_dir(&self) -> PathBuf {
        self.session_dir().join("files")
    }
    pub fn key_path(&self) -> PathBuf {
        self.session_dir().join("khoa")
    }
    pub fn code_file(&self) -> PathBuf {
        self.data_dir.join("login_code.txt")
    }
    pub fn password_file(&self) -> PathBuf {
        self.data_dir.join("login_password.txt")
    }
}

/// Tạo `<data>/tdlib/files` (0700) và khoá DB `<data>/tdlib/khoa` (0600, 16 byte ngẫu nhiên dạng hex — như
/// `store.ensure_session_dir` bản Python) nếu chưa có. KHÔNG BAO GIỜ ghi đè khoá có sẵn. Trả khoá dạng byte.
pub fn ensure_session(cfg: &LoginConfig) -> io::Result<Vec<u8>> {
    let files = cfg.files_dir();
    fs::create_dir_all(&files)?;
    fs::set_permissions(cfg.session_dir(), fs::Permissions::from_mode(0o700))?;
    fs::set_permissions(&files, fs::Permissions::from_mode(0o700))?;
    let key_path = cfg.key_path();
    if !key_path.exists() {
        let mut raw = [0u8; 16];
        fs::File::open("/dev/urandom")?.read_exact(&mut raw)?;
        let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&key_path)?;
        f.write_all(hex(&raw).as_bytes())?;
    }
    Ok(decode_key(fs::read_to_string(&key_path)?.trim()))
}

/// Giống `TdlibClient._read_key` bản Python: chuỗi hex → byte; không phải hex → byte UTF-8 nguyên văn.
pub fn decode_key(raw: &str) -> Vec<u8> {
    let b = raw.as_bytes();
    if b.len() % 2 == 0 && b.iter().all(u8::is_ascii_hexdigit) {
        b.chunks(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    } else {
        b.to_vec()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `setTdlibParameters` — trường PHẲNG, cùng giá trị với `TdlibClient.open()` bản Python.
pub fn tdlib_parameters(cfg: &LoginConfig, key: &[u8]) -> Value {
    let files = cfg.files_dir().to_string_lossy().into_owned();
    json!({
        "@type": "setTdlibParameters",
        "use_test_dc": false,
        "database_directory": files,
        "files_directory": files,
        "database_encryption_key": base64::engine::general_purpose::STANDARD.encode(key),
        "use_file_database": true,
        "use_chat_info_database": true,
        "use_message_database": true,
        "use_secret_chats": false,
        "api_id": cfg.api_id,
        "api_hash": cfg.api_hash,
        "system_language_code": "en",
        "device_model": "telegram-gateway",
        "system_version": "Linux",
        "application_version": "0.1.0",
    })
}

/// Chạy đăng nhập tới `authorizationStateReady`. `Err` khi TDLib từ chối tham số/số điện thoại, bị FLOOD_WAIT,
/// thiếu cấu hình, hoặc gặp trạng thái chưa hỗ trợ (QR, email, đăng ký mới, đóng phiên).
pub async fn run(client: &TdClient, cfg: &LoginConfig) -> Result<(), String> {
    let mut rx = client.auth_watch();
    loop {
        let state = rx.borrow_and_update().clone();
        match state.as_str() {
            "" => {} // TDLib chưa báo trạng thái đầu tiên
            "authorizationStateWaitTdlibParameters" => {
                let key = ensure_session(cfg)
                    .map_err(|e| format!("không tạo được phiên ở {}: {e}", cfg.session_dir().display()))?;
                client
                    .request(tdlib_parameters(cfg, &key))
                    .await
                    .map_err(|e| format!("setTdlibParameters bị từ chối: {}", e.replace(&cfg.api_hash, "***")))?;
            }
            "authorizationStateWaitPhoneNumber" => {
                if cfg.phone.is_empty() {
                    return Err("chưa có phiên đăng nhập và chưa đặt TG_PHONE".into());
                }
                // Một lần duy nhất: lỗi (kể cả FLOOD_WAIT) thì dừng, không tự thử lại.
                client
                    .request(json!({"@type": "setAuthenticationPhoneNumber", "phone_number": cfg.phone}))
                    .await
                    .map_err(|e| format!("Telegram từ chối số điện thoại: {e}"))?;
            }
            "authorizationStateWaitCode" => {
                eprintln!("[td] chờ mã đăng nhập: submit_code() hoặc ghi vào {}", cfg.code_file().display());
                let input = Input { slot: &client.code, file: cfg.code_file(), what: "mã" };
                if !submit(client, &mut rx, input, "checkAuthenticationCode", "code").await? {
                    continue; // trạng thái đã đổi trong lúc chờ → xử lý ngay trạng thái mới
                }
            }
            "authorizationStateWaitPassword" => {
                eprintln!("[td] chờ mật khẩu 2FA: submit_password() hoặc ghi vào {}", cfg.password_file().display());
                let input = Input { slot: &client.password, file: cfg.password_file(), what: "mật khẩu" };
                if !submit(client, &mut rx, input, "checkAuthenticationPassword", "password").await? {
                    continue;
                }
            }
            "authorizationStateReady" => return Ok(()),
            "authorizationStateClosing" | "authorizationStateClosed" | "authorizationStateLoggingOut" => {
                return Err(format!("TDLib đóng phiên ({state})"));
            }
            other => return Err(format!("trạng thái đăng nhập chưa hỗ trợ: {other}")),
        }
        // Đã xử lý trạng thái này → chờ trạng thái KHÁC (đừng gửi lại cùng yêu cầu).
        rx.changed().await.map_err(|_| "mất kết nối TDLib".to_owned())?;
    }
}

struct Input<'a> {
    slot: &'a Secret,
    file: PathBuf,
    what: &'static str,
}

/// Chờ mã/mật khẩu rồi gửi `kind`. `Ok(true)` khi TDLib nhận; `Ok(false)` khi trạng thái đổi trong lúc chờ.
/// Bị từ chối (vd PHONE_CODE_INVALID) thì chờ giá trị MỚI — không gửi lại giá trị cũ. FLOOD_WAIT → `Err`.
async fn submit(
    client: &TdClient,
    rx: &mut watch::Receiver<String>,
    input: Input<'_>,
    kind: &str,
    field: &str,
) -> Result<bool, String> {
    let entered = SystemTime::now();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        let value = input.slot.take().or_else(|| {
            read_input_file(&input.file, entered, input.what).map(|v| {
                if field == "code" { v.trim().to_owned() } else { v }
            }).filter(|v| !v.is_empty())
        });
        if let Some(v) = value {
            // TDLib có thể nhắc lại input trong message lỗi; che trước khi log hoặc trả lên CLI.
            match client.request(json!({"@type": kind, (field): v})).await.map_err(|e| e.replace(&v, "***")) {
                Ok(_) => return Ok(true),
                Err(e) if e.contains("FLOOD_WAIT") => return Err(format!("{kind}: {e}")),
                Err(e) => eprintln!("[td] {} bị từ chối ({e}); chờ {} mới", input.what, input.what),
            }
            continue;
        }
        tokio::select! {
            _ = input.slot.notified() => {}
            _ = tick.tick() => {}
            r = rx.changed() => {
                r.map_err(|_| "mất kết nối TDLib".to_owned())?;
                return Ok(false);
            }
        }
    }
}

/// Đọc rồi XOÁ tệp nhập (để không gửi lại). Tệp có trước lúc bắt đầu chờ (quá 2 giây — bù độ thô của mtime) là của lần
/// trước → xoá, bỏ qua, tránh đốt một lượt thử. Tệp rỗng (có thể đang được ghi) để lại cho lần đọc sau.
fn read_input_file(path: &Path, entered: SystemTime, what: &str) -> Option<String> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    if modified + Duration::from_secs(2) < entered {
        eprintln!("[td] bỏ qua tệp {what} cũ {} (có trước lúc chờ)", path.display());
        let _ = fs::remove_file(path);
        return None;
    }
    // Giữ khoảng trắng của mật khẩu, chỉ bỏ ký tự xuống dòng do trình soạn thảo thêm.
    let v = fs::read_to_string(path).ok()?.trim_end_matches(['\r', '\n']).to_owned();
    if v.is_empty() {
        return None;
    }
    let _ = fs::remove_file(path);
    eprintln!("[td] đã đọc {what} từ {}", path.display());
    Some(v)
}
