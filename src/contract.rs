//! HỢP ĐỒNG giữa các module (chốt trước khi chia việc — đổi tệp này phải báo người điều phối).
//!
//! - `td`    : nói chuyện với TDLib (libtdjson 1.8.69 qua JSON), đăng nhập, phân phát bản tin → hiện thực [`TdLink`].
//! - `calls` : cuộc gọi P2P bằng ntgcalls 3.0.7, chỉ dùng TDLib qua [`TdLink`] (test bằng TdLink giả) → hiện thực [`CallControl`].
//! - `rest`  : HTTP giống hệt gateway Python (cùng đường, thân JSON, mã lỗi) cho tích hợp HA, chỉ gọi [`CallControl`]/[`ChatControl`].
//!
//! Ngôn ngữ chung: JSON là `serde_json::Value`; lỗi nghiệp vụ là [`GwError`] (rest đổi thành HTTP giống `gateway/errors.py`).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Nội dung hữu hạn và metadata công khai. Không truyền đường dẫn tệp tới REST.
#[derive(Debug)]
pub struct Binary {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub cache_control: &'static str,
    pub disposition: Option<String>,
}

/// Lỗi TDLib: giữ nguyên mã HTTP của TDLib và message để tầng API ánh xạ đúng
/// hợp đồng Python. Lỗi vận chuyển không có mã.
#[derive(Debug, Clone, PartialEq)]
pub struct TdError {
    pub code: Option<i64>,
    pub message: String,
}

impl TdError {
    pub fn telegram(code: i64, message: impl Into<String>) -> Self {
        Self { code: Some(code), message: message.into() }
    }

    pub fn transport(message: impl Into<String>) -> Self {
        Self { code: None, message: message.into() }
    }
}

/// Lỗi nghiệp vụ, mang sẵn mã HTTP + mã chữ như bản Python (`ApiError(status, code, message)`).
#[derive(Debug, Clone, PartialEq)]
pub struct GwError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after_seconds: Option<u64>,
}

impl GwError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self { status, code: code.to_owned(), message: message.into(), retry_after_seconds: None }
    }

    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }

    pub fn from_tdlib(error: TdError) -> Self {
        let message = if error.message.is_empty() {
            error.code.map_or_else(|| "Unknown error".to_owned(), |code| code.to_string())
        } else {
            error.message
        };
        if error.code == Some(429) {
            if let Some(seconds) = flood_wait_seconds(&message) {
                return Self::new(429, "FLOOD_WAIT", "Telegram yêu cầu chờ trước khi thử lại.")
                    .with_retry_after(seconds);
            }
        }
        match error.code {
            Some(code @ 400..=499) => Self::new(code as u16, "TELEGRAM_ERROR", format!("Telegram từ chối: {message}")),
            _ => Self::new(502, "MEDIA_ENGINE_ERROR", format!("Lỗi phía Telegram: {message}")),
        }
    }
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self::new(400, "BAD_REQUEST", m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(404, "NOT_FOUND", m)
    }
}

/// Đường tới TDLib cho các module khác (module `td` hiện thực; test dùng bản giả).
pub trait TdLink: Send + Sync + 'static {
    /// Gửi một yêu cầu TDLib (JSON có `@type`), chờ đúng câu trả lời của nó (ghép bằng `@extra`).
    /// `Err` khi TDLib trả `{"@type":"error"}` (giữ cả code/message) hoặc mất kết nối.
    fn request(&self, req: Value) -> BoxFut<'_, Result<Value, TdError>>;
    /// Mọi bản tin KHÔNG phải câu trả lời (update*) — mỗi người đăng ký nhận một bản sao.
    fn updates(&self) -> tokio::sync::broadcast::Receiver<Value>;
}

fn flood_wait_seconds(message: &str) -> Option<u64> {
    message.match_indices("FLOOD_WAIT_").find_map(|(offset, marker)| {
        let suffix = &message[offset + marker.len()..];
        let digits = suffix.split(|c: char| !c.is_ascii_digit()).next()?;
        digits.parse().ok()
    })
}

/// Điều khiển cuộc gọi — đúng các đường `/api/live/*` mà tích hợp HA gọi (xem gateway Python `chat_routes.py`).
/// Thân yêu cầu và JSON trả về GIỮ NGUYÊN hình dạng bản Python (`LiveCall.info()` cho mỗi cuộc gọi).
pub trait CallControl: Send + Sync + 'static {
    /// GET /api/live/calls → `{"calls":[info…]}` như Python.
    fn calls(&self) -> BoxFut<'_, Result<Value, GwError>>;
    /// POST /api/live/call (gọi đi), /accept, /hangup, /settings, /media, /playback, /say — `body` là JSON nguyên văn.
    fn call_out(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
    fn accept(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
    fn hangup(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
    fn settings(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
    fn media(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
    fn playback(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
    fn say(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
}

/// Tài khoản, chat, tin nhắn — các đường còn lại (`/api/accounts`, `/api/chats…`, `/api/messages/*`, `/api/feed`, …).
/// Giai đoạn đầu được phép trả `GwError::new(501, "NOT_IMPLEMENTED", …)`.
pub trait ChatControl: Send + Sync + 'static {
    /// `method` ("GET"/"POST"), `path` (vd "/api/chats/123/send"), `query` (chuỗi query thô), `body` JSON (Null nếu không có).
    fn handle(&self, method: &str, path: &str, query: &str, body: Value) -> BoxFut<'_, Result<Value, GwError>>;
    /// GET ảnh/tệp hoặc nhật ký. Backend chỉ JSON có thể giữ mặc định 501.
    fn bytes(&self, _path: &str, _query: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        Box::pin(async { Err(GwError::new(501, "NOT_IMPLEMENTED", "Chưa có phản hồi nhị phân cho đường này.")) })
    }
}
