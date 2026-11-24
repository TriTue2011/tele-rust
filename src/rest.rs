//! HTTP API — GIỐNG HỆT gateway Python (`gateway/routes.py`, `gateway/chat_routes.py`, `gateway/errors.py`) để tích hợp HA
//! `telegram_call` dùng được mà không sửa. Module này chỉ lo phần vận chuyển; nghiệp vụ nằm sau [`CallControl`]/[`ChatControl`].
//!
//! Hành vi đối chiếu với mã Python trong `tham-khao/python-gateway/`:
//! - Auth `Authorization: Bearer <GATEWAY_TOKEN>` (phân biệt hoa thường, so sánh thời gian hằng); sai/thiếu → 401 `UNAUTHORIZED`.
//!   Riêng `GET /health` không cần token.
//! - 120 yêu cầu/phút cho token (cửa sổ trượt 60 giây, `RATE_LIMIT_PER_MINUTE`), chỉ đếm yêu cầu đã qua auth → 429 `RATE_LIMITED`.
//!   Snapshot có hạn mức riêng bằng 3 lần giá trị trên để tải ảnh không chiếm lượt điều khiển cuộc gọi.
//! - Thân JSON: rỗng = `{}`; > 400 000 byte → 400 «vượt 400 KB»; JSON hỏng → 400; không phải object → 400.
//! - Lỗi nghiệp vụ [`GwError`] → `{"error":{"code","message"}}` với đúng mã HTTP.
//! - Kiểm tra TRƯỚC auth như FastAPI: đường lạ → 404 `{"detail":"Not Found"}`, sai phương thức → 405
//!   `{"detail":"Method Not Allowed"}`, tham số đường dẫn số (`chat_id`, `file_id`) sai → 422 `{"detail":[…int_parsing…]}`.
//!
//! Quy ước riêng của bản Rust (hợp đồng `contract.rs` cố định, không chở được các thứ này):
//! - `POST /api/chats/{id}/send_file`: thân HTTP là tệp thô (tối đa 64 MB, không phải JSON) → [`ChatControl::handle`] nhận
//!   `{"data_b64": "<base64 của tệp>"}`; `kind`/`name`/`caption` vẫn ở query.
//! - `/api/live/*` (trừ `GET /api/live/calls`): `?account=` không có chỗ trong [`CallControl`] nên được chép vào thân là
//!   `"account"` (bản Python chọn tài khoản theo query).
//! - `GET /api/live/settings` không có hàm trong [`CallControl`] → chuyển cho [`ChatControl::handle`].
//! - Đường tải tệp/nhật ký dùng `ChatControl::bytes`.
//! - `/health` lấy số tài khoản từ `ChatControl::handle("GET", "/api/accounts", …)` (`{"tai_khoan":[{status}]}`); lỗi = 0.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{on, MethodFilter, MethodRouter};
use axum::Router;
use base64::Engine as _;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;

use crate::contract::{Binary, BoxFut, CallControl, ChatControl, GwError, VideoFrames};
use crate::events::EventStore;
use crate::ui_auth::{AuthError, UiAuth};

/// Như `MAX_BODY_BYTES` của routes.py.
const MAX_BODY_BYTES: usize = 400_000;
/// Như `MAX_UPLOAD_BYTES` của chat_routes.py.
const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;
const FILE_KINDS: [&str; 5] = ["photo", "video", "voice", "audio", "document"];

pub struct RestConfig {
    pub token: String,
    pub rate_limit_per_minute: usize,
    pub version: String,
}

struct Ctx {
    call: Arc<dyn CallControl>,
    chat: Arc<dyn ChatControl>,
    events: Option<EventStore>,
    accounts: bool,
    token: String,
    ui_auth: Arc<UiAuth>,
    limit: usize,
    /// Hạn mức THEO TỪNG NGƯỜI GỌI (khoá = «bearer» hoặc «cookie:<12 ký tự đầu của phiên>»): trước đây MỘT xô chung cho cả gateway nên
    /// Home Assistant (hỏi mỗi 1,5 s) + giao diện web cộng dồn vượt 120/phút làm giao diện báo «Quá 120 request/phút cho token này» và khoá (22/11).
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
    snapshot_hits: Mutex<HashMap<String, VecDeque<Instant>>>,
    /// Tải tệp (avatar, ảnh, thumb): mỗi trang tải hàng chục tệp nhỏ nên có xô riêng, rộng hơn xô điều khiển.
    media_hits: Mutex<HashMap<String, VecDeque<Instant>>>,
    version: String,
    started: Instant,
    audio_slots: tokio::sync::Semaphore,
}

impl Ctx {
    /// `_rate_limited` của routes.py: bỏ mốc cũ hơn 60 giây; đủ hạn mức thì từ chối (không ghi thêm mốc).
    fn rate_limited(hits: &Mutex<HashMap<String, VecDeque<Instant>>>, key: &str, limit: usize) -> bool {
        let now = Instant::now();
        let mut map = hits.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() > 256 {
            map.retain(|_, dq| dq.back().is_some_and(|t| now.duration_since(*t) <= Duration::from_secs(60)));
        }
        let dq = map.entry(key.to_owned()).or_default();
        while dq.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
            dq.pop_front();
        }
        if dq.len() >= limit {
            return true;
        }
        dq.push_back(now);
        false
    }

    /// `_auth` của routes.py.
    async fn auth(&self, parts: &axum::http::request::Parts, kind: Kind) -> Result<Option<String>, GwError> {
        let (refreshed_cookie, principal) = if self.bearer(&parts.headers).is_ok() {
            (None, "bearer".to_owned())
        } else if let Some(token) = cookie_token(&parts.headers) {
            let principal = format!("cookie:{}", token.chars().take(12).collect::<String>());
            if matches!(parts.method, Method::POST | Method::PUT | Method::DELETE | Method::PATCH)
                && !csrf_valid(&parts.headers, self.ui_auth.secure_cookie() || parts.uri.scheme_str() == Some("https")) {
                return Err(GwError::new(403, "CSRF_FORBIDDEN", "Yêu cầu ghi bằng cookie thiếu kiểm tra CSRF."));
            }
            let secure = request_is_secure(parts, self.ui_auth.secure_cookie());
            let auth = self.ui_auth.clone();
            let session = tokio::task::spawn_blocking(move || auth.validate_session(&token, secure))
                .await.map_err(|_| auth_task_error())?.map_err(auth_error)?;
            (session.refreshed_cookie, principal)
        } else {
            return Err(unauthorized());
        };
        let limited = |message: String| {
            let mut error = GwError::new(429, "RATE_LIMITED", message);
            error.retry_after_seconds = Some(5);
            error
        };
        if matches!(kind, Kind::AudioIn | Kind::VideoIn) {
            // Audio PCM / khung webcam có giới hạn riêng theo cuộc gọi, không tiêu ngân sách điều khiển.
        } else if matches!(kind, Kind::Snapshot) {
            let limit = self.limit.saturating_mul(3);
            if Self::rate_limited(&self.snapshot_hits, &principal, limit) {
                return Err(limited(format!("Quá {limit} yêu cầu ảnh/phút cho token này.")));
            }
        } else if matches!(kind, Kind::ChatBytes) {
            let limit = self.limit.saturating_mul(5);
            if Self::rate_limited(&self.media_hits, &principal, limit) {
                return Err(limited(format!("Quá {limit} yêu cầu tệp/phút cho token này.")));
            }
        } else if Self::rate_limited(&self.hits, &principal, self.limit) {
            return Err(limited(format!("Quá {} request/phút cho token này.", self.limit)));
        }
        Ok(refreshed_cookie)
    }

    fn bearer(&self, headers: &HeaderMap) -> Result<(), GwError> {
        let h = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
        let token = h.strip_prefix("Bearer ").unwrap_or("");
        if self.token.is_empty() || !ct_eq(token.as_bytes(), self.token.as_bytes()) {
            return Err(GwError::new(401, "UNAUTHORIZED", "Thiếu hoặc sai token."));
        }
        Ok(())
    }
}

/// So sánh thời gian hằng (chỉ lộ độ dài, như `secrets.compare_digest`).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

#[derive(Clone, Copy)]
enum LiveOp {
    Call,
    Accept,
    Hangup,
    Settings,
    Media,
    Playback,
    Say,
    Devices,
}

#[derive(Clone, Copy)]
enum Kind {
    /// GET /api/live/calls → `CallControl::calls`.
    LiveCalls,
    LiveDevices,
    /// POST /api/live/* → `CallControl`.
    Live(LiveOp),
    /// GET → `ChatControl::handle(GET, …, Null)`.
    ChatGet,
    /// POST thân JSON → `ChatControl::handle(POST, …, body)`.
    ChatPost,
    Ui,
    /// POST tệp thô → `ChatControl::handle(POST, …, {"data_b64"})`.
    SendFile,
    WallpaperUpload,
    ChatBytes,
    Snapshot,
    LiveView,
    AudioIn,
    VideoIn,
    Events,
    Auth,
}

#[derive(Clone, Copy)]
struct Spec {
    kind: Kind,
    /// Tham số đường dẫn kiểu `int` của FastAPI: (tên, vị trí đoạn khi tách path theo '/').
    int_param: Option<(&'static str, usize)>,
    int_param2: Option<(&'static str, usize)>,
}

fn json_resp(status: StatusCode, v: &Value) -> Response {
    let body = serde_json::to_vec(v).unwrap_or_default();
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn bytes_resp(result: Result<Binary, GwError>) -> Response {
    match result {
        Err(error) => error_resp(&error),
        Ok(content) => {
            // Untrusted Telegram files may be navigated to with the web session cookie.
            // Only passive media gets its original MIME type; unknown types are downloads.
            let mime = content.content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            let inline = matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp")
                || mime.starts_with("audio/") || mime.starts_with("video/");
            let content_type = if inline { content.content_type } else { "application/octet-stream".into() };
            let mut response = ([(header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, content.cache_control.to_owned()),
                (header::CONTENT_LENGTH, content.bytes.len().to_string())], content.bytes).into_response();
            if !inline {
                let disposition = content.disposition.filter(|value| value.starts_with("attachment;"))
                    .and_then(|value| value.parse().ok()).unwrap_or_else(|| "attachment".parse().unwrap());
                response.headers_mut().insert(header::CONTENT_DISPOSITION, disposition);
            } else if let Some(disposition) = content.disposition {
                if let Ok(value) = disposition.parse() { response.headers_mut().insert(header::CONTENT_DISPOSITION, value); }
            }
            isolate_binary(response)
        }
    }
}

/// Phản hồi tệp/nhị phân của người dùng (cần cookie/Bearer): chỉ cùng nguồn được nhúng.
fn isolate_binary(response: Response) -> Response { isolate_with(response, "same-origin") }

/// Luồng âm thanh dành cho THIẾT BỊ NGOÀI trình duyệt cùng nguồn (loa Google Cast, loa R1): đường này không dùng cookie mà dùng token riêng
/// từng cuộc gọi trong URL, và bộ nhận của loa tải theo kiểu khác nguồn — `same-origin` sẽ làm loa im. Vẫn giữ nosniff + CSP sandbox.
fn isolate_media(response: Response) -> Response { isolate_with(response, "cross-origin") }

fn isolate_with(mut response: Response, corp: &'static str) -> Response {
    for (name, value) in [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox"),
        (header::HeaderName::from_static("cross-origin-resource-policy"), corp),
    ] { response.headers_mut().insert(name, value.parse().unwrap()); }
    response
}

/// `ApiError` → JSON như `_api_error_handler` (errors.py).
fn error_resp(e: &GwError) -> Response {
    let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut error = json!({"code": e.code, "message": e.message});
    if let Some(seconds) = e.retry_after_seconds { error["retry_after_seconds"] = json!(seconds); }
    let mut response = json_resp(status, &json!({"error": error}));
    if status == StatusCode::TOO_MANY_REQUESTS {
        if let Some(value) = e.retry_after_seconds.and_then(|seconds| seconds.to_string().parse().ok()) {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
    }
    response
}

async fn not_found() -> Response {
    json_resp(StatusCode::NOT_FOUND, &json!({"detail": "Not Found"}))
}

async fn method_not_allowed() -> Response {
    json_resp(StatusCode::METHOD_NOT_ALLOWED, &json!({"detail": "Method Not Allowed"}))
}

/// `_body_dict` của routes.py.
async fn body_dict(body: Body) -> Result<Value, GwError> {
    let raw = to_bytes(body, MAX_BODY_BYTES).await.map_err(|_| GwError::bad_request("Thân request vượt 400 KB."))?;
    if raw.is_empty() {
        return Ok(json!({}));
    }
    let v: Value = serde_json::from_slice(&raw).map_err(|_| GwError::bad_request("Thân request không phải JSON hợp lệ."))?;
    if !v.is_object() {
        return Err(GwError::bad_request("Thân POST phải là JSON object."));
    }
    Ok(v)
}

/// `request.query_params.get(name)` của Starlette (giá trị cuối cùng nếu lặp).
fn query_get(query: &str, name: &str) -> Option<String> {
    form_urlencoded::parse(query.as_bytes()).filter(|(k, _)| k == name).last().map(|(_, v)| v.into_owned())
}

async fn handle(ctx: Arc<Ctx>, spec: Spec, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path().to_owned();
    let query = parts.uri.query().unwrap_or("").to_owned();
    // FastAPI kiểm kiểu tham số đường dẫn trước khi vào hàm (tức trước auth).
    for (name, idx) in [spec.int_param, spec.int_param2].into_iter().flatten() {
        let seg = path.split('/').nth(idx).unwrap_or("");
        if seg.parse::<i64>().is_err() {
            let detail = json!({"detail": [{"type": "int_parsing", "loc": ["path", name],
                "msg": "Input should be a valid integer, unable to parse string as an integer", "input": seg}]});
            return json_resp(StatusCode::UNPROCESSABLE_ENTITY, &detail);
        }
    }
    let mut since = 0;
    if matches!(spec.kind, Kind::Events) {
        if let Some(raw) = query_get(&query, "since") {
            match raw.trim().parse() {
                Ok(cursor) => since = cursor,
                Err(_) => return json_resp(StatusCode::UNPROCESSABLE_ENTITY, &json!({"detail":[{
                    "type":"int_parsing", "loc":["query", "since"],
                    "msg":"Input should be a valid integer, unable to parse string as an integer", "input":raw
                }]})),
            }
        }
    }
    // Mic ~10 POST/s có hạn mức riêng theo cuộc gọi; không tiêu hạn mức điều khiển 120/phút.
    if matches!(spec.kind, Kind::Auth) {
        let mut response = auth_handle(ctx.ui_auth.clone(), parts, body).await;
        response.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        return response;
    }
    let refreshed_cookie = match ctx.auth(&parts, spec.kind).await {
        Ok(cookie) => cookie,
        Err(error) => return error_resp(&error),
    };
    let out = match spec.kind {
        Kind::AudioIn => {
            let Ok(_permit) = ctx.audio_slots.try_acquire() else {
                return with_cookie(error_resp(&GwError::new(429, "AUDIO_BACKPRESSURE", "Quá nhiều khối mic đang chờ.")), refreshed_cookie.as_deref());
            };
            let raw = match tokio::time::timeout(Duration::from_secs(5), to_bytes(body, 64 * 1024)).await {
                Err(_) => return with_cookie(error_resp(&GwError::new(408, "AUDIO_TIMEOUT", "Quá thời gian nhận PCM.")), refreshed_cookie.as_deref()),
                Ok(Err(_)) => return with_cookie(error_resp(&GwError::new(413, "AUDIO_TOO_LARGE", "PCM vượt 64 KiB.")), refreshed_cookie.as_deref()),
                Ok(Ok(raw)) => raw,
            };
            ctx.call.audio_in(path.rsplit('/').next().unwrap(), raw.to_vec()).await
        }
        Kind::VideoIn => {
            let Ok(_permit) = ctx.audio_slots.try_acquire() else {
                return with_cookie(error_resp(&GwError::new(429, "VIDEO_BACKPRESSURE", "Quá nhiều khung camera đang chờ.")), refreshed_cookie.as_deref());
            };
            let raw = match tokio::time::timeout(Duration::from_secs(5), to_bytes(body, 256 * 1024)).await {
                Err(_) => return with_cookie(error_resp(&GwError::new(408, "VIDEO_TIMEOUT", "Quá thời gian nhận khung camera.")), refreshed_cookie.as_deref()),
                Ok(Err(_)) => return with_cookie(error_resp(&GwError::new(413, "VIDEO_TOO_LARGE", "Khung JPEG vượt 256 KiB.")), refreshed_cookie.as_deref()),
                Ok(Ok(raw)) => raw,
            };
            ctx.call.video_in(path.rsplit('/').next().unwrap(), raw.to_vec()).await
        }
        Kind::ChatBytes => return with_cookie(bytes_resp(ctx.chat.bytes(&path, &query).await), refreshed_cookie.as_deref()),
        Kind::Snapshot if ctx.accounts => return with_cookie(bytes_resp(ctx.chat.bytes(&path,&query).await), refreshed_cookie.as_deref()),
        Kind::Snapshot => return with_cookie(bytes_resp(ctx.call.snapshot(path.rsplit('/').next().unwrap()).await), refreshed_cookie.as_deref()),
        Kind::Events if ctx.accounts => ctx.chat.handle("GET", "/api/events", &query, Value::Null).await,
        Kind::LiveView => return with_cookie(match ctx.call.video_frames(path.rsplit('/').next().unwrap()).await {
            Ok(frames) => live_view(frames, Duration::from_secs(8), Duration::from_secs(30)).await,
            Err(e) => error_resp(&e),
        }, refreshed_cookie.as_deref()),
        Kind::Events => match &ctx.events {
            Some(store) => store.poll(since).await,
            None => Err(GwError::new(501, "NOT_IMPLEMENTED", "Chưa có bộ lưu sự kiện.")),
        },
        Kind::LiveCalls if ctx.accounts => ctx.chat.handle("GET", "/api/live/calls", &query, Value::Null).await,
        Kind::LiveCalls => ctx.call.calls().await,
        Kind::LiveDevices if ctx.accounts => ctx.chat.handle("GET", "/api/live/devices", &query, Value::Null).await,
        Kind::LiveDevices => ctx.call.devices(Value::Null).await,
        Kind::Ui => match if parts.method == axum::http::Method::GET || parts.method == axum::http::Method::DELETE {
            Ok(json!({}))
        } else { body_dict(body).await } {
            Err(e) => Err(e),
            Ok(b) => ctx.chat.handle(parts.method.as_str(), &path, &query, b).await,
        },
        Kind::Live(op) => match body_dict(body).await {
            Err(e) => Err(e),
            Ok(mut b) => {
                if let Some(acc) = query_get(&query, "account").filter(|a| !a.is_empty()) {
                    b["account"] = Value::String(acc);
                }
                let c = &ctx.call;
                let fut: BoxFut<'_, Result<Value, GwError>> = match op {
                    LiveOp::Call => c.call_out(b),
                    LiveOp::Accept => c.accept(b),
                    LiveOp::Hangup => c.hangup(b),
                    LiveOp::Settings => c.settings(b),
                    LiveOp::Media => c.media(b),
                    LiveOp::Playback => c.playback(b),
                    LiveOp::Say => c.say(b),
                    LiveOp::Devices => c.devices(b),
                };
                fut.await
            }
        },
        Kind::ChatGet => ctx.chat.handle("GET", &path, &query, Value::Null).await,
        Kind::ChatPost => match body_dict(body).await {
            Err(e) => Err(e),
            Ok(b) => ctx.chat.handle("POST", &path, &query, b).await,
        },
        Kind::SendFile => send_file(&ctx, &path, &query, body).await,
        Kind::WallpaperUpload => upload_wallpaper(&ctx, &path, &query, body).await,
        Kind::Auth => unreachable!(),
    };
    with_cookie(match out {
        Ok(v) => json_resp(StatusCode::OK, &v),
        Err(e) => error_resp(&e),
    }, refreshed_cookie.as_deref())
}

fn unauthorized() -> GwError { GwError::new(401, "UNAUTHORIZED", "Thiếu hoặc sai token.") }

fn auth_task_error() -> GwError {
    GwError::new(500, "AUTH_STORAGE", "Không xử lý được đăng nhập web.")
}

fn auth_error(error: AuthError) -> GwError {
    let mut out = GwError::new(error.status, error.code, error.message);
    out.retry_after_seconds = error.retry_after;
    out
}

fn with_cookie(mut response: Response, cookie: Option<&str>) -> Response {
    if let Some(cookie) = cookie {
        if let Ok(value) = cookie.parse() { response.headers_mut().insert(header::SET_COOKIE, value); }
    }
    response
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "tggw_session" && !value.is_empty()).then(|| value.to_owned())
    })
}

fn csrf_valid(headers: &HeaderMap, secure: bool) -> bool {
    // This header is a CSRF boundary only while CORS stays disabled.
    if headers.get("x-requested-with").and_then(|value| value.to_str().ok()) == Some("fetch") { return true; }
    let Some(host) = headers.get(header::HOST).and_then(|value| value.to_str().ok()) else { return false; };
    let Some(origin) = headers.get(header::ORIGIN).and_then(|value| value.to_str().ok()) else { return false; };
    let Some(authority) = origin.strip_prefix("https://").or_else(|| (!secure).then(|| origin.strip_prefix("http://")).flatten()) else { return false; };
    let authority = authority.strip_suffix('/').unwrap_or(authority);
    !authority.contains('/') && authority.eq_ignore_ascii_case(host)
}

fn request_is_secure(parts: &axum::http::request::Parts, configured: bool) -> bool {
    configured || parts.uri.scheme_str() == Some("https") || parts.headers.get(header::ORIGIN)
        .and_then(|value| value.to_str().ok()).is_some_and(|origin| origin.starts_with("https://"))
}

async fn auth_handle(auth: Arc<UiAuth>, parts: axum::http::request::Parts, body: Body) -> Response {
    let action = parts.uri.path().rsplit('/').next().unwrap_or_default();
    let payload = if parts.method == Method::POST {
        let raw = match tokio::time::timeout(Duration::from_secs(5), to_bytes(body, 8192)).await {
            Ok(Ok(raw)) => raw,
            Ok(Err(_)) => return error_resp(&GwError::bad_request("Thân đăng nhập web vượt 8 KB.")),
            Err(_) => return error_resp(&GwError::new(408, "AUTH_TIMEOUT", "Quá thời gian nhận yêu cầu đăng nhập web.")),
        };
        match serde_json::from_slice::<Value>(&raw) {
            Ok(body) if body.is_object() => body,
            _ if raw.is_empty() => json!({}),
            _ => return error_resp(&GwError::bad_request("Thân đăng nhập web phải là JSON object.")),
        }
    } else { Value::Null };
    // Slow request bodies must not occupy slots reserved for password hashing.
    let permit = if matches!(action, "setup" | "login" | "password") { Some(auth.hash_permit().await) } else { None };
    tokio::task::spawn_blocking(move || { let _permit = permit; auth_action(&auth, &parts, &payload) })
        .await.unwrap_or_else(|_| error_resp(&auth_task_error()))
}

fn auth_action(auth: &UiAuth, parts: &axum::http::request::Parts, payload: &Value) -> Response {
    let action = parts.uri.path().rsplit('/').next().unwrap_or_default();
    let secure = request_is_secure(parts, auth.secure_cookie());
    let current_cookie = cookie_token(&parts.headers);
    if action == "status" {
        let status = match auth.status(current_cookie.as_deref(), secure) {
            Ok(status) => status,
            Err(error) => return error_resp(&auth_error(error)),
        };
        let body = json!({"da_thiet_lap":status.configured, "dang_nhap":status.logged_in_as.is_some(),
            "nguoi_dung":status.logged_in_as});
        return with_cookie(json_resp(StatusCode::OK, &body), status.refreshed_cookie.as_deref());
    }
    if matches!(action, "setup" | "login") && !parts.headers.get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(';').next().unwrap().trim().eq_ignore_ascii_case("application/json")) {
        return error_resp(&GwError::bad_request("Yêu cầu đăng nhập web cần Content-Type application/json."));
    }
    let outcome = match action {
        "setup" => {
            let (Some(token), Some(user), Some(password)) = (str_field(payload, "token"), str_field(payload, "ten"), str_field(payload, "mat_khau")) else {
                return error_resp(&GwError::bad_request("Thiếu token, tên đăng nhập hoặc mật khẩu."));
            };
            auth.setup(token, user, password, secure).map(|grant| (json!({"ok":true,"nguoi_dung":grant.username}), Some(grant.cookie)))
        }
        "login" => {
            let (Some(user), Some(password)) = (str_field(payload, "ten"), str_field(payload, "mat_khau")) else {
                return error_resp(&GwError::bad_request("Thiếu tên đăng nhập hoặc mật khẩu."));
            };
            let remember = payload.get("ghi_nho").and_then(Value::as_bool).unwrap_or(true);
            let ip = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(addr)| addr.ip().to_string()).unwrap_or_else(|| "unknown".into());
            auth.login(user, password, remember, &ip, secure).map(|grant| (json!({"ok":true,"nguoi_dung":grant.username}), Some(grant.cookie)))
        }
        "logout" => {
            let Some(token) = current_cookie.as_deref() else { return error_resp(&auth_error(crate::ui_auth::AuthError {
                status:401,code:"UNAUTHORIZED",message:"Chưa đăng nhập hoặc phiên đã hết hạn.",retry_after:None,
            })); };
            if !csrf_valid(&parts.headers, auth.secure_cookie() || parts.uri.scheme_str() == Some("https")) { return error_resp(&GwError::new(403,"CSRF_FORBIDDEN","Yêu cầu ghi bằng cookie thiếu kiểm tra CSRF.")); }
            auth.logout(token).map(|_| (json!({"ok":true}), Some(clear_cookie(secure))))
        }
        "logout-all" => {
            let Some(token) = current_cookie.as_deref() else { return error_resp(&auth_error(crate::ui_auth::AuthError {
                status:401,code:"UNAUTHORIZED",message:"Chưa đăng nhập hoặc phiên đã hết hạn.",retry_after:None,
            })); };
            if !csrf_valid(&parts.headers, auth.secure_cookie() || parts.uri.scheme_str() == Some("https")) { return error_resp(&GwError::new(403,"CSRF_FORBIDDEN","Yêu cầu ghi bằng cookie thiếu kiểm tra CSRF.")); }
            auth.logout_all(token).map(|_| (json!({"ok":true}), Some(clear_cookie(secure))))
        }
        "password" => {
            let Some(token) = current_cookie.as_deref() else { return error_resp(&auth_error(crate::ui_auth::AuthError {
                status:401,code:"UNAUTHORIZED",message:"Chưa đăng nhập hoặc phiên đã hết hạn.",retry_after:None,
            })); };
            if !csrf_valid(&parts.headers, auth.secure_cookie() || parts.uri.scheme_str() == Some("https")) { return error_resp(&GwError::new(403,"CSRF_FORBIDDEN","Yêu cầu ghi bằng cookie thiếu kiểm tra CSRF.")); }
            let (Some(old), Some(new)) = (str_field(payload, "cu"), str_field(payload, "moi")) else {
                return error_resp(&GwError::bad_request("Thiếu mật khẩu cũ hoặc mới."));
            };
            auth.change_password(token, old, new).map(|_| (json!({"ok":true}), None))
        }
        _ => unreachable!("chỉ gọi cho đường dẫn đã đăng ký"),
    };
    match outcome {
        Ok((value, cookie)) => with_cookie(json_resp(StatusCode::OK, &value), cookie.as_deref()),
        Err(error) => {
            let error = auth_error(error);
            let mut response = error_resp(&error);
            if let Some(seconds) = error.retry_after_seconds {
                response.headers_mut().insert(header::RETRY_AFTER, seconds.to_string().parse().unwrap());
            }
            response
        }
    }
}

fn str_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> { value.get(name).and_then(Value::as_str) }

fn clear_cookie(secure: bool) -> String {
    format!("tggw_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}", if secure { "; Secure" } else { "" })
}

/// Phần vận chuyển của `send_file` (chat_routes.py): kiểm `kind` trước khi đọc thân, thân thô ≤ 64 MB, không rỗng.
async fn send_file(ctx: &Ctx, path: &str, query: &str, body: Body) -> Result<Value, GwError> {
    let kind = query_get(query, "kind").unwrap_or_else(|| "document".into());
    if !FILE_KINDS.contains(&kind.as_str()) {
        return Err(GwError::bad_request("kind phải là photo, video, voice, audio hoặc document."));
    }
    let raw = to_bytes(body, MAX_UPLOAD_BYTES).await.map_err(|_| GwError::bad_request("Tệp vượt 64 MB."))?;
    if raw.is_empty() {
        return Err(GwError::bad_request("Tệp rỗng."));
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    ctx.chat.handle("POST", path, query, json!({"data_b64": b64})).await
}

/// Tải ảnh nền: thân thô ≤ 3 MB (JPEG/PNG), gateway kiểm lại ruột tệp rồi lưu theo băm nội dung.
async fn upload_wallpaper(ctx: &Ctx, path: &str, query: &str, body: Body) -> Result<Value, GwError> {
    let raw = to_bytes(body, crate::chat::MAX_WALLPAPER_BYTES + 1).await.map_err(|_| GwError::bad_request("Ảnh nền tối đa 3 MB."))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    ctx.chat.handle("POST", path, query, json!({"data_b64": b64})).await
}

/// `/health` (không auth). Không bao giờ hỏng: `ChatControl` lỗi thì coi như 0 tài khoản.
async fn health(State(ctx): State<Arc<Ctx>>) -> Response {
    let (total, online) = match ctx.chat.handle("GET", "/api/accounts", "", Value::Null).await {
        Ok(v) => v.get("tai_khoan").and_then(Value::as_array).map_or((0, 0), |a| {
            let online = a.iter().filter(|x| matches!(x.get("status").and_then(Value::as_str), Some("online" | "mock"))).count();
            (a.len(), online)
        }),
        Err(_) => (0, 0),
    };
    json_resp(StatusCode::OK, &json!({
        "status": if online > 0 { "ok" } else { "degraded" },
        "version": ctx.version,
        "uptime_seconds": ctx.started.elapsed().as_secs(),
        "accounts_total": total,
        "accounts_online": online,
        "mock_mode": false,
    }))
}

/// Loa Cast không gửi Bearer. Token riêng của mỗi cuộc gọi hết hiệu lực khi luồng đóng.
async fn cast_stream(State(ctx): State<Arc<Ctx>>, req: Request) -> Response {
    let mut parts = req.uri().path().rsplit('/');
    let Some(token) = parts.next().and_then(|s| s.strip_suffix(".mp3")) else {
        return not_found().await;
    };
    let id = parts.next().unwrap();
    let Some(stream) = ctx.call.cast_audio(id, token).await else {
        return error_resp(&GwError::not_found("Không có luồng này."));
    };
    const CAST_LENGTH: usize = 16_000 * 4 * 3600;
    let chunks = futures_util::stream::unfold((Some(stream.prime), stream.audio, CAST_LENGTH), |(mut prime, mut audio, left)| async move {
        if left == 0 { return None; }
        let mut chunk = match prime.take().filter(|p| !p.is_empty()) {
            Some(bytes) => bytes,
            None => loop {
                match audio.recv().await {
                    Ok(bytes) => break bytes,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            },
        };
        chunk.truncate(left);
        let left = left - chunk.len();
        Some((Ok::<_, std::convert::Infallible>(chunk), (prime, audio, left)))
    });
    isolate_media(([(header::CONTENT_TYPE, "audio/mpeg".into()), (header::CACHE_CONTROL, "no-store".into()),
        (header::CONTENT_LENGTH, CAST_LENGTH.to_string())], Body::from_stream(chunks)).into_response())
}

/// Token media ngắn hạn, chỉ lấy từ POST /media đã qua Bearer. Không nhận Bearer trong query.
async fn audio_out_stream(State(ctx): State<Arc<Ctx>>, req: Request) -> Response {
    let id = req.uri().path().rsplit('/').next().unwrap();
    let token = query_get(req.uri().query().unwrap_or(""), "t").unwrap_or_default();
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)) {
        return error_resp(&GwError::new(403,"MEDIA_TOKEN","Thiếu, sai hoặc hết hạn token âm thanh."));
    }
    match ctx.call.audio_out(id, &token).await {
        Ok(stream) => audio_out_response(stream),
        Err(e) => error_resp(&e),
    }
}

fn audio_out_response(stream: crate::contract::AudioOut) -> Response {
    let chunks = futures_util::stream::unfold((Some(stream.prime), stream.audio, stream.stopped, stream.expires),
        |(mut prime, mut audio, mut stopped, expires)| async move {
            if stopped.has_changed().is_err() || Instant::now() >= expires { return None; }
            let chunk = if let Some(header) = prime.take() { header } else {
                tokio::select! { biased;
                    _ = stopped.changed() => return None,
                    _ = tokio::time::sleep_until(expires.into()) => return None,
                    data = audio.recv() => match data { Ok(bytes) => bytes, Err(_) => return None },
                }
            };
            Some((Ok::<_, std::convert::Infallible>(chunk), (prime, audio, stopped, expires)))
        });
    isolate_media(([(header::CONTENT_TYPE,"audio/wav"),(header::CACHE_CONTROL,"no-store"),
        (header::REFERRER_POLICY,"no-referrer")],Body::from_stream(chunks)).into_response())
}

async fn next_jpeg(frames: &mut VideoFrames, limit: Duration) -> Result<Option<Arc<[u8]>>, ()> {
    tokio::time::timeout(limit, async {
        while frames.has_changed().is_ok() && frames.changed().await.is_ok() {
            if let Some(jpeg) = frames.borrow_and_update().clone() { return Ok(Some(jpeg)); }
        }
        Err(())
    }).await.unwrap_or(Ok(None))
}

async fn live_view(mut frames: VideoFrames, first_wait: Duration, idle: Duration) -> Response {
    // Bản Python trả 409 trước khi gửi header nếu chưa có khung đầu tiên.
    frames.mark_changed();
    match next_jpeg(&mut frames, first_wait).await {
        Ok(Some(_)) => frames.mark_changed(),
        Ok(None) => return error_resp(&GwError::new(409, "NO_VIDEO", "Chưa có hình video của bên kia.")),
        Err(()) => return error_resp(&GwError::not_found("Cuộc gọi đã kết thúc.")),
    }
    let parts = futures_util::stream::unfold(frames, move |mut frames| async move {
        let jpeg = next_jpeg(&mut frames, idle).await.ok().flatten()?;
        let mut part = format!("--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len()).into_bytes();
        part.extend_from_slice(&jpeg);
        part.extend_from_slice(b"\r\n");
        Some((Ok::<_, std::convert::Infallible>(part), frames))
    });
    isolate_binary(([(header::CONTENT_TYPE, "multipart/x-mixed-replace; boundary=frame"),
        (header::CACHE_CONTROL, "no-store")], Body::from_stream(parts)).into_response())
}

/// Một phương thức → `spec`; phương thức khác → 405 kiểu FastAPI.
fn route(filter: MethodFilter, spec: Spec) -> MethodRouter<Arc<Ctx>> {
    add(MethodRouter::new(), filter, spec)
        .on(MethodFilter::HEAD, method_not_allowed)
        .fallback(method_not_allowed)
}

fn add(r: MethodRouter<Arc<Ctx>>, filter: MethodFilter, spec: Spec) -> MethodRouter<Arc<Ctx>> {
    r.on(filter, move |State(ctx): State<Arc<Ctx>>, req: Request| handle(ctx, spec, req))
}

const fn s(kind: Kind) -> Spec {
    Spec { kind, int_param: None, int_param2: None }
}

/// `/api/<a>/{chat_id|file_id}/…`: tham số số nằm ở đoạn thứ 3 (`["", "api", a, id, …]`).
const fn s_id(kind: Kind, name: &'static str) -> Spec {
    Spec { kind, int_param: Some((name, 3)), int_param2: None }
}

const fn s_ids(kind: Kind, first: (&'static str, usize), second: (&'static str, usize)) -> Spec {
    Spec { kind, int_param: Some(first), int_param2: Some(second) }
}

fn router_with(ctx: Arc<Ctx>) -> Router {
    use Kind::*;
    use LiveOp::*;
    let get = MethodFilter::GET;
    let post = MethodFilter::POST;
    let mut r = Router::new().route("/health", on(get, health).on(MethodFilter::HEAD, method_not_allowed).fallback(method_not_allowed));
    for path in ["/","/app.html","/ui/app.html"] {r=r.route(path,on(get,crate::web::app));}
    for path in ["/index.html","/ui/","/ui/index.html"] {r=r.route(path,on(get,crate::web::accounts));}
    r = r.route("/ui/qrcode.min.js", on(get, crate::web::qrcode))
        .route("/ui/emoji.js", on(get, crate::web::emoji_js)).route("/ui/emoji-data.js", on(get, crate::web::emoji_data_js)).route("/ui/wallpaper-art.js", on(get, crate::web::wallpaper_art_js));
    r = r.route("/ui/api/auth/status", route(get, s(Auth)));
    for action in ["setup", "login", "logout", "logout-all", "password"] {
        r = r.route(&format!("/ui/api/auth/{action}"), route(post, s(Auth)));
    }
    r=r.route("/ui/api/accounts",route(post,s(Ui)))
        .route("/ui/api/accounts/qr",route(post,s(Ui)))
        .route("/ui/api/accounts/:login_id",add(route(get,s(Ui)),MethodFilter::DELETE,s(Ui)));
    for action in ["code","password","qr-confirm"] {
        r=r.route(&format!("/ui/api/accounts/:login_id/{action}"),route(post,s(Ui)));
    }
    // Chỉ đọc → ChatControl (routes.py + chat_routes.py).
    for p in ["/api/accounts", "/api/me", "/api/chats", "/api/contacts", "/api/search", "/api/feed",
              "/api/calls", "/api/calltrace", "/api/contacts/blocked", "/api/storage/usage", "/api/storage/janitor"] {
        r = r.route(p, route(get, s(ChatGet)));
    }
    r = r
        .route("/api/settings/storage", add(route(get, s(ChatGet)), post, s(ChatPost)))
        .route("/api/settings/appearance", add(route(get, s(ChatGet)), post, s(ChatPost)))
        .route("/api/profile", add(route(get, s(ChatGet)), post, s(ChatPost)))
        .route("/api/events", route(get, s(Events)))
        .route("/api/chats/:chat_id/messages", route(get, s_id(ChatGet, "chat_id")))
        .route("/api/chats/:chat_id/messages/:message_id/comments", add(
            route(get, s_ids(ChatGet, ("chat_id", 3), ("message_id", 5))),
            post,
            s_ids(ChatPost, ("chat_id", 3), ("message_id", 5)),
        ))
        .route("/api/chats/:chat_id/info", route(get, s_id(ChatGet, "chat_id")))
        .route("/api/chats/:chat_id/members", route(get, s_id(ChatGet, "chat_id")))
        .route("/api/file/:file_id", route(get, s_id(ChatBytes, "file_id")))
        .route("/api/calltrace/:call_id", route(get, s(ChatBytes)));
    // Ghi, thân JSON → ChatControl.
    for p in ["/api/send_message", "/api/send_media", "/api/send_voice", "/api/call", "/api/answer_call",
              "/api/hangup_call", "/api/audio/in", "/api/audio/upload", "/api/messages/edit", "/api/messages/delete",
              "/api/messages/forward", "/api/messages/react", "/api/resolve", "/api/translate", "/api/contacts/open",
              "/api/contacts/add", "/api/contacts/edit", "/api/contacts/remove", "/api/contacts/block", "/api/contacts/unblock",
              "/api/do/:name", "/api/storage/clear"] {
        r = r.route(p, route(post, s(ChatPost)));
    }
    for p in ["/api/chats/:chat_id/send", "/api/chats/:chat_id/send_album", "/api/chats/:chat_id/read",
              "/api/chats/:chat_id/settings", "/api/chats/:chat_id/clear_history", "/api/chats/:chat_id/delete",
              "/api/chats/:chat_id/leave"] {
        r = r.route(p, route(post, s_id(ChatPost, "chat_id")));
    }
    r = r.route("/api/chats/:chat_id/send_file", route(post, s_id(SendFile, "chat_id")));
    r = r.route("/api/wallpapers/upload", route(post, s(WallpaperUpload)));
    r = r.route("/api/backgrounds", route(get, s(ChatGet))).route("/api/messages/reactions", route(get, s(ChatGet)))
        .route("/api/chats/:chat_id/background", add(route(get, s_id(ChatGet, "chat_id")), post, s_id(ChatPost, "chat_id")));
    // Cuộc gọi → CallControl.
    r = r
        .route("/api/live/calls", route(get, s(LiveCalls)))
        .route("/api/live/audio/in/:call_id", route(post, s(AudioIn)))
        .route("/api/live/video/in/:call_id", route(post, s(VideoIn)))
        .route("/api/live/audio/out/:call_id", on(get, audio_out_stream).on(MethodFilter::HEAD, method_not_allowed).fallback(method_not_allowed))
        .route("/api/live/settings", add(route(get, s(ChatGet)), post, s(Live(Settings))))
        .route("/api/live/devices", route(get, s(LiveDevices)))
        .route("/api/live/ha_devices", route(post, s(Live(Devices))))
        .route("/api/live/view/:call_id", route(get, s(LiveView)))
        .route("/api/live/snapshot/:call_id", route(get, s(Snapshot)))
        .route("/api/live/cast/:call_id/:file", on(get, cast_stream).on(MethodFilter::HEAD, method_not_allowed).fallback(method_not_allowed));
    for (p, op) in [("/api/live/call", Call), ("/api/live/accept", Accept), ("/api/live/hangup", Hangup),
                    ("/api/live/media", Media), ("/api/live/playback", Playback), ("/api/live/say", Say)] {
        r = r.route(p, route(post, s(Live(op))));
    }
    r.fallback(not_found).with_state(ctx)
}

/// Router HTTP hoàn chỉnh. Gọi với token rỗng là lỗi của người gọi (`serve` từ chối khởi động trước đó).
pub fn router(call: Arc<dyn CallControl>, chat: Arc<dyn ChatControl>, cfg: RestConfig) -> Router {
    router_with_events(call, chat, cfg, None)
}

pub fn router_with_ui_auth(call: Arc<dyn CallControl>, chat: Arc<dyn ChatControl>, cfg: RestConfig, ui_auth: Arc<UiAuth>) -> Router {
    router_with(Arc::new(Ctx {
        call, chat, events: None, accounts: false, token: cfg.token, ui_auth,
        limit: cfg.rate_limit_per_minute, hits: Mutex::new(HashMap::new()), media_hits: Mutex::new(HashMap::new()),
        snapshot_hits: Mutex::new(HashMap::new()), version: cfg.version,
        started: Instant::now(), audio_slots: tokio::sync::Semaphore::new(8),
    }))
}

pub(crate) fn router_accounts_with_ui_auth(accounts: Arc<crate::account_router::Accounts>, cfg: RestConfig, ui_auth: Arc<UiAuth>) -> Router {
    router_with(Arc::new(Ctx {
        call: accounts.clone(), chat: accounts, events: None, accounts: true, token: cfg.token,
        ui_auth, limit: cfg.rate_limit_per_minute, hits: Mutex::new(HashMap::new()), media_hits: Mutex::new(HashMap::new()),
        snapshot_hits: Mutex::new(HashMap::new()), version: cfg.version,
        started: Instant::now(), audio_slots: tokio::sync::Semaphore::new(8),
    }))
}

#[cfg(test)]
pub(crate) fn router_accounts(accounts: Arc<crate::account_router::Accounts>, cfg: RestConfig) -> Router {
    let ui_auth = Arc::new(UiAuth::in_memory(&cfg.token));
    router_accounts_with_ui_auth(accounts, cfg, ui_auth)
}

pub(crate) fn router_with_events(call: Arc<dyn CallControl>, chat: Arc<dyn ChatControl>, cfg: RestConfig, events: Option<EventStore>) -> Router {
    let ui_auth = Arc::new(UiAuth::in_memory(&cfg.token));
    router_with(Arc::new(Ctx {
        call,
        chat,
        events,
        accounts: false,
        token: cfg.token,
        ui_auth,
        limit: cfg.rate_limit_per_minute,
        hits: Mutex::new(HashMap::new()), media_hits: Mutex::new(HashMap::new()),
        snapshot_hits: Mutex::new(HashMap::new()),
        version: cfg.version,
        started: Instant::now(),
        audio_slots: tokio::sync::Semaphore::new(8),
    }))
}

/// `_int` của config.py: thiếu hoặc không phải số → mặc định.
fn env_num<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// `tggw serve`: nghe `GATEWAY_HOST` (mặc định 0.0.0.0) : `REST_PORT` (mặc định 3334). Thiếu `GATEWAY_TOKEN` → từ chối chạy
/// (như `load_config` của Python). Dừng êm khi nhận SIGTERM/SIGINT.
pub fn serve_from_env(call: Arc<dyn CallControl>, chat: Arc<dyn ChatControl>) -> Result<(), String> {
    let (addr, cfg) = serve_config_from_env()?;
    let app = router(call, chat, cfg);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("không tạo được tokio: {e}"))?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("không nghe được {addr}: {e}"))?;
        serve(listener, app, shutdown_signal()).await
    })
}

pub fn serve_config_from_env() -> Result<(SocketAddr, RestConfig), String> {
    let token = std::env::var("GATEWAY_TOKEN").unwrap_or_default();
    if token.is_empty() {
        return Err("GATEWAY_TOKEN chưa đặt. Sinh bằng 'openssl rand -hex 32'.".into());
    }
    let host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = env_num("REST_PORT", 3334);
    let addr: SocketAddr = format!("{host}:{port}").parse().map_err(|e| format!("địa chỉ nghe sai {host}:{port}: {e}"))?;
    let cfg = RestConfig {
        token,
        rate_limit_per_minute: env_num("RATE_LIMIT_PER_MINUTE", 120),
        version: env!("CARGO_PKG_VERSION").into(),
    };
    Ok((addr, cfg))
}

pub async fn serve<F>(listener: tokio::net::TcpListener, app: Router, shutdown: F) -> Result<(), String>
where
    F: Future<Output = ()> + Send + 'static,
{
    let addr = listener.local_addr().map_err(|e| format!("không đọc được địa chỉ HTTP: {e}"))?;
    if crate::tls_serve::enabled() {
        match crate::tls_serve::acceptor(&crate::tls_serve::tls_dir(), addr) {
            Ok(acceptor) => {
                log::info!("tggw serve: nghe http://{addr} và https://{addr} (cùng cổng, chứng chỉ tự ký)");
                return crate::tls_serve::serve_dual(listener, app, acceptor, shutdown).await;
            }
            Err(e) => log::warn!("HTTPS tắt vì không dựng được chứng chỉ ({e}); vẫn nghe HTTP"),
        }
    }
    log::info!("tggw serve: nghe http://{addr}");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown).await.map_err(|e| format!("HTTP lỗi: {e}"))
}

pub async fn shutdown_signal() {
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term => {},
    }
}

/// `CallControl`/`ChatControl` TẠM cho `tggw serve` khi chưa có module `calls`/`chat`: mọi việc → 501 `NOT_IMPLEMENTED`,
/// riêng danh sách cuộc gọi trả `{"calls":[]}`.
pub struct ChuaCo;

fn chua_co(what: &str) -> BoxFut<'static, Result<Value, GwError>> {
    let e = GwError::new(501, "NOT_IMPLEMENTED", format!("Bản Rust chưa có: {what}"));
    Box::pin(async move { Err(e) })
}

impl CallControl for ChuaCo {
    fn calls(&self) -> BoxFut<'_, Result<Value, GwError>> {
        Box::pin(async { Ok(json!({"calls": []})) })
    }
    fn call_out(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co("/api/live/call")
    }
    fn accept(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co("/api/live/accept")
    }
    fn hangup(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co("/api/live/hangup")
    }
    fn settings(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co("/api/live/settings")
    }
    fn media(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co("/api/live/media")
    }
    fn playback(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co("/api/live/playback")
    }
    fn say(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co("/api/live/say")
    }
}

impl ChatControl for ChuaCo {
    fn handle(&self, method: &str, path: &str, _: &str, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        chua_co(&format!("{method} {path}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::CastAudio;
    use futures_util::StreamExt;
    use tokio::sync::watch;
    use tower::ServiceExt;

    /// Ghi lại mọi lời gọi; trả lời theo `reply` (mặc định `{"ok":true,"from":…}`).
    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<(String, String, String, Value)>>,
        reply: Mutex<Option<Result<Value, GwError>>>,
        video: Mutex<Option<watch::Sender<Option<Arc<[u8]>>>>>,
    }

    impl Fake {
        fn rec(&self, a: &str, b: &str, c: &str, v: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.log.lock().unwrap().push((a.into(), b.into(), c.into(), v));
            let out = self.reply.lock().unwrap().clone().unwrap_or_else(|| Ok(json!({"ok": true, "from": a})));
            Box::pin(async move { out })
        }
        fn last(&self) -> (String, String, String, Value) {
            self.log.lock().unwrap().last().cloned().expect("không có lời gọi nào")
        }
        fn count(&self) -> usize {
            self.log.lock().unwrap().len()
        }
    }

    impl CallControl for Fake {
        fn calls(&self) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("calls", "", "", Value::Null)
        }
        fn call_out(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("call_out", "", "", b)
        }
        fn accept(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("accept", "", "", b)
        }
        fn hangup(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("hangup", "", "", b)
        }
        fn settings(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("settings", "", "", b)
        }
        fn devices(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("devices", "", "", b)
        }
        fn media(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("media", "", "", b)
        }
        fn playback(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("playback", "", "", b)
        }
        fn say(&self, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("say", "", "", b)
        }
        fn video_in(&self, id: &str, jpeg: Vec<u8>) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("video_in", id, "", json!(jpeg))
        }
        fn audio_in(&self, id: &str, pcm: Vec<u8>) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec("audio_in", id, "", json!(pcm))
        }
        fn snapshot(&self, id: &str) -> BoxFut<'_, Result<Binary, GwError>> {
            let reply = self.rec("snapshot", id, "", Value::Null);
            Box::pin(async move {
                reply.await?;
                Ok(Binary { bytes: vec![1, 2, 3], content_type: "image/jpeg".into(),
                    cache_control: "no-store", disposition: None })
            })
        }
        fn cast_audio(&self, id: &str, token: &str) -> BoxFut<'_, Option<CastAudio>> {
            let valid = id == "c1" && token == "test-cast-token";
            Box::pin(async move {
                if !valid { return None; }
                let (tx, audio) = tokio::sync::broadcast::channel(1);
                tx.send(vec![4, 5, 6]).unwrap();
                Some(CastAudio { prime: vec![1, 2, 3], audio })
            })
        }
        fn video_frames(&self, id: &str) -> BoxFut<'_, Result<VideoFrames, GwError>> {
            let video = self.video.lock().unwrap();
            let frames = video.as_ref().filter(|_| id == "c1").map(watch::Sender::subscribe);
            Box::pin(async { frames.ok_or_else(|| GwError::not_found("Không có cuộc gọi đang diễn ra này.")) })
        }
    }

    impl ChatControl for Fake {
        fn handle(&self, m: &str, p: &str, q: &str, b: Value) -> BoxFut<'_, Result<Value, GwError>> {
            self.rec(m, p, q, b)
        }
        fn bytes(&self, p: &str, q: &str) -> BoxFut<'_, Result<Binary, GwError>> {
            let reply = self.rec("GET", p, q, Value::Null);
            Box::pin(async move {
                reply.await?;
                Ok(Binary { bytes: vec![1, 2, 3], content_type: "application/octet-stream".into(),
                    cache_control: "no-store", disposition: None })
            })
        }
    }

    const TOK: &str = "bi-mat-thu";

    fn app_limit(call: &Arc<Fake>, chat: &Arc<Fake>, limit: usize) -> Router {
        let cfg = RestConfig { token: TOK.into(), rate_limit_per_minute: limit, version: "thu".into() };
        router(call.clone(), chat.clone(), cfg)
    }

    fn app() -> (Router, Arc<Fake>, Arc<Fake>) {
        let (call, chat) = (Arc::new(Fake::default()), Arc::new(Fake::default()));
        (app_limit(&call, &chat, 120), call, chat)
    }

    fn req(method: &str, uri: &str, auth: Option<&str>, body: impl Into<Body>) -> Request {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(a) = auth {
            b = b.header("Authorization", a);
        }
        b.body(body.into()).unwrap()
    }

    fn ok(method: &str, uri: &str, body: impl Into<Body>) -> Request {
        req(method, uri, Some(&format!("Bearer {TOK}")), body)
    }

    async fn send(app: &Router, r: Request) -> (u16, Value) {
        let resp = app.clone().oneshot(r).await.unwrap();
        let st = resp.status().as_u16();
        let raw = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (st, serde_json::from_slice(&raw).unwrap_or(Value::Null))
    }

    fn err(code: &str, msg: &str) -> Value {
        json!({"error": {"code": code, "message": msg}})
    }

    #[tokio::test]
    async fn health_khong_can_token_va_dung_hinh_dang() {
        let (app, _, chat) = app();
        *chat.reply.lock().unwrap() =
            Some(Ok(json!({"tai_khoan": [{"status": "online"}, {"status": "offline"}, {"status": "mock"}]})));
        let (st, v) = send(&app, req("GET", "/health", None, Body::empty())).await;
        assert_eq!(st, 200);
        assert_eq!(v["status"], "ok");
        assert_eq!(v["accounts_total"], 3);
        assert_eq!(v["accounts_online"], 2);
        assert_eq!(v["mock_mode"], false);
        assert!(v["version"].is_string() && v["uptime_seconds"].is_u64());
        assert_eq!(v.as_object().unwrap().len(), 6);
        assert_eq!(chat.last().1, "/api/accounts");
    }

    #[tokio::test]
    async fn health_khi_chat_loi_van_200_degraded() {
        let (app, _, chat) = app();
        *chat.reply.lock().unwrap() = Some(Err(GwError::new(501, "NOT_IMPLEMENTED", "x")));
        let (st, v) = send(&app, req("GET", "/health", None, Body::empty())).await;
        assert_eq!((st, v["status"].clone(), v["accounts_total"].clone()), (200, json!("degraded"), json!(0)));
    }

    #[tokio::test]
    async fn sai_hoac_thieu_token_401() {
        let (app, call, _) = app();
        let want = err("UNAUTHORIZED", "Thiếu hoặc sai token.");
        let bad_auth = [None, Some("Bearer sai"), Some("bearer bi-mat-thu"), Some("Bearer  bi-mat-thu"), Some(TOK),
                        Some("Bearer bi-mat-th")];
        for a in bad_auth {
            assert_eq!(send(&app, req("GET", "/api/live/calls", a, Body::empty())).await, (401, want.clone()), "{a:?}");
        }
        assert_eq!(send(&app, req("POST", "/api/live/hangup", None, "{bad")).await, (401, want));
        assert_eq!(call.count(), 0);
    }

    #[tokio::test]
    async fn qua_han_muc_429_va_401_khong_tinh() {
        let (call, chat) = (Arc::new(Fake::default()), Arc::new(Fake::default()));
        let app = app_limit(&call, &chat, 3);
        for _ in 0..5 {
            assert_eq!(send(&app, req("GET", "/api/live/calls", Some("Bearer sai"), Body::empty())).await.0, 401);
        }
        for _ in 0..3 {
            assert_eq!(send(&app, ok("GET", "/api/live/calls", Body::empty())).await.0, 200);
        }
        let got = send(&app, ok("GET", "/api/chats", Body::empty())).await;
        assert_eq!(got, (429, json!({"error": {"code": "RATE_LIMITED", "message": "Quá 3 request/phút cho token này.", "retry_after_seconds": 5}})));
        // /health không bị giới hạn.
        assert_eq!(send(&app, req("GET", "/health", None, Body::empty())).await.0, 200);
        assert_eq!(call.count(), 3);
    }

    #[tokio::test]
    async fn han_muc_mac_dinh_120_dung_chung_cac_duong() {
        let (app, call, chat) = app();
        for i in 0..120 {
            let path = if i % 2 == 0 { "/api/live/calls" } else { "/api/chats" };
            assert_eq!(send(&app, ok("GET", path, Body::empty())).await.0, 200);
        }
        assert_eq!(send(&app, ok("GET", "/api/accounts", Body::empty())).await,
                   (429, json!({"error": {"code": "RATE_LIMITED", "message": "Quá 120 request/phút cho token này.", "retry_after_seconds": 5}})));
        assert_eq!((call.count(), chat.count()), (60, 60));
    }

    #[test]
    fn cua_so_truot_bo_moc_cu_khong_gia_han_khi_tu_choi() {
        let fake = Arc::new(Fake::default());
        let ctx = Ctx {
            events: None,
            accounts: false,
            call: fake.clone(), chat: fake, token: TOK.into(), ui_auth: Arc::new(UiAuth::in_memory(TOK)), limit: 2,
            hits: Mutex::new(HashMap::from([("bearer".to_owned(), VecDeque::from([
                Instant::now() - Duration::from_secs(61), Instant::now(),
            ]))])), media_hits: Mutex::new(HashMap::new()),
            snapshot_hits: Mutex::new(HashMap::new()),
            version: "thu".into(), started: Instant::now(),
            audio_slots: tokio::sync::Semaphore::new(8),
        };
        assert!(!Ctx::rate_limited(&ctx.hits, "bearer", ctx.limit));
        let before = ctx.hits.lock().unwrap().clone();
        assert_eq!(before["bearer"].len(), 2);
        assert!(Ctx::rate_limited(&ctx.hits, "bearer", ctx.limit));
        assert_eq!(*ctx.hits.lock().unwrap(), before);
    }

    #[test]
    fn han_muc_tinh_rieng_cho_tung_nguoi_goi_va_xo_tep_khong_chiem_xo_dieu_khien() {
        // 22/11: một xô chung cho cả gateway ⇒ HA + giao diện web cộng dồn vượt 120/phút ⇒ giao diện báo «Quá 120 request/phút» và khoá.
        let hits = Mutex::new(HashMap::new());
        for _ in 0..3 { assert!(!Ctx::rate_limited(&hits, "bearer", 3)); }
        assert!(Ctx::rate_limited(&hits, "bearer", 3), "người gọi bearer hết hạn mức");
        assert!(!Ctx::rate_limited(&hits, "cookie:abc", 3), "phiên cookie KHÔNG bị ảnh hưởng bởi bearer");
        let (control, media) = (Mutex::new(HashMap::new()), Mutex::new(HashMap::new()));
        for _ in 0..2 { assert!(!Ctx::rate_limited(&control, "cookie:abc", 2)); }
        assert!(Ctx::rate_limited(&control, "cookie:abc", 2));
        assert!(!Ctx::rate_limited(&media, "cookie:abc", 10), "tải tệp có xô riêng, không dùng chung xô điều khiển");
    }

    #[tokio::test]
    async fn snapshot_van_can_bearer_va_khong_chiem_han_muc_cup_may() {
        let (app, _, _) = app();
        let path = "/api/live/snapshot/c1";
        assert_eq!(send(&app, req("GET", path, None, Body::empty())).await.0, 401);
        for _ in 0..360 {
            let response = app.clone().oneshot(ok("GET", path, Body::empty())).await.unwrap();
            assert_eq!(response.status(), 200);
        }
        assert_eq!(send(&app, ok("GET", path, Body::empty())).await.0, 429);
        assert_eq!(send(&app, ok("POST", "/api/live/hangup", Body::empty())).await.0, 200);
    }

    #[tokio::test]
    async fn token_cau_hinh_rong_khong_bo_qua_bearer() {
        let fake = Arc::new(Fake::default());
        let app = router(fake.clone(), fake, RestConfig {
            token: String::new(), rate_limit_per_minute: 120, version: "thu".into(),
        });
        assert_eq!(send(&app, req("GET", "/api/accounts", None, Body::empty())).await.0, 401);
        assert_eq!(send(&app, req("GET", "/api/accounts", Some("Bearer "), Body::empty())).await.0, 401);
    }

    #[tokio::test]
    async fn than_json_sai_400() {
        let (app, call, _) = app();
        let cases = [
            ("{bad", "Thân request không phải JSON hợp lệ."),
            ("   ", "Thân request không phải JSON hợp lệ."),
            ("[1]", "Thân POST phải là JSON object."),
            ("\"x\"", "Thân POST phải là JSON object."),
        ];
        for (body, msg) in cases {
            assert_eq!(send(&app, ok("POST", "/api/live/hangup", body)).await, (400, err("BAD_REQUEST", msg)), "{body}");
        }
        assert_eq!(call.count(), 0);
    }

    #[tokio::test]
    async fn than_qua_400kb_400_dung_400000_thi_qua() {
        let (app, call, chat) = app();
        // Đúng 400 000 byte: {"a":"xxx…"} → được nhận.
        let exact = format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_BODY_BYTES - 8));
        assert_eq!(exact.len(), MAX_BODY_BYTES);
        assert_eq!(send(&app, ok("POST", "/api/live/hangup", exact)).await.0, 200);
        assert_eq!(call.count(), 1);
        let over = format!("{{\"a\":\"{}\"}}", "x".repeat(MAX_BODY_BYTES - 7));
        let want = (400, err("BAD_REQUEST", "Thân request vượt 400 KB."));
        assert_eq!(send(&app, ok("POST", "/api/live/hangup", over.clone())).await, want);
        assert_eq!(send(&app, ok("POST", "/api/chats/5/send_album", over)).await, want);
        assert_eq!((call.count(), chat.count()), (1, 0));
    }

    #[tokio::test]
    async fn than_rong_la_object_rong() {
        let (app, call, _) = app();
        assert_eq!(send(&app, ok("POST", "/api/live/hangup", Body::empty())).await.0, 200);
        assert_eq!(call.last(), ("hangup".into(), "".into(), "".into(), json!({})));
    }

    #[tokio::test]
    async fn browser_camera_frames_need_auth_are_size_limited_and_do_not_spend_control_budget() {
        let (call, chat) = (Arc::new(Fake::default()), Arc::new(Fake::default()));
        let app = app_limit(&call, &chat, 1);
        let path = "/api/live/video/in/c1";
        assert_eq!(send(&app, req("POST", path, None, vec![0xFF, 0xD8, 0xFF, 0xD9])).await.0, 401);
        assert_eq!(send(&app, ok("POST", path, vec![0; 262_145])).await.0, 413);
        for _ in 0..5 { assert_eq!(send(&app, ok("POST", path, vec![0xFF, 0xD8, 9, 0xFF, 0xD9])).await.0, 200); }
        assert_eq!(call.last(), ("video_in".into(), "c1".into(), "".into(), json!([255, 216, 9, 255, 217])));
        assert_eq!(send(&app, ok("GET", "/api/live/calls", Body::empty())).await.0, 200);
        assert_eq!(send(&app, ok("GET", "/api/live/calls", Body::empty())).await.0, 429, "khung camera không tiêu ngân sách điều khiển");
    }

    #[tokio::test]
    async fn browser_pcm_is_raw_bearer_limited_and_does_not_spend_control_budget() {
        let (call, chat) = (Arc::new(Fake::default()), Arc::new(Fake::default()));
        let app = app_limit(&call, &chat, 1);
        let path = "/api/live/audio/in/c1";
        assert_eq!(send(&app, req("POST", path, None, vec![0; 2])).await.0, 401);
        assert_eq!(send(&app, req("POST", &format!("{path}?t={TOK}"), None, vec![0; 2])).await.0, 401);
        assert_eq!(send(&app, ok("POST", path, vec![0; 65_538])).await.0, 413);
        for _ in 0..31 { assert_eq!(send(&app, ok("POST", path, vec![1, 2, 3, 4])).await.0, 200); }
        assert_eq!(call.last(), ("audio_in".into(), "c1".into(), "".into(), json!([1,2,3,4])));
        assert_eq!(send(&app, ok("GET", "/api/live/calls", Body::empty())).await.0, 200);
        assert_eq!(send(&app, ok("GET", "/api/live/calls", Body::empty())).await.0, 429);
    }

    #[tokio::test]
    async fn device_catalog_routes_require_bearer_and_pass_account() {
        let (app,call,_) = app();
        for (method,path) in [("GET","/api/live/devices"),("POST","/api/live/ha_devices")] {
            assert_eq!(send(&app,req(method,path,None,Body::empty())).await.0,401);
        }
        assert_eq!(send(&app,ok("GET","/api/live/devices",Body::empty())).await.0,200);
        assert_eq!(call.last(),("devices".into(),"".into(),"".into(),Value::Null));
        assert_eq!(send(&app,ok("POST","/api/live/ha_devices?account=second",r#"{"mic":[]}"#)).await.0,200);
        assert_eq!(call.last().3,json!({"mic":[],"account":"second"}));
    }

    #[tokio::test]
    async fn browser_audio_stream_has_wav_headers_and_closes_at_expiry_or_end() {
        let (app,_,_) = app();
        for path in ["/api/live/audio/out/c1", "/api/live/audio/out/c1?t=wrong", "/api/live/audio/out/c1?token=bi-mat-thu"] {
            assert_eq!(send(&app,req("GET",path,None,Body::empty())).await.0,403);
        }
        let (audio,receiver)=tokio::sync::broadcast::channel(2);
        let (alive,stopped)=tokio::sync::watch::channel(());
        let response=audio_out_response(crate::contract::AudioOut {prime:b"RIFF".to_vec(),audio:receiver,stopped,expires:Instant::now()+Duration::from_secs(1)});
        assert_eq!(response.headers()[header::CONTENT_TYPE],"audio/wav");
        assert_eq!(response.headers()[header::CACHE_CONTROL],"no-store");
        assert!(!response.headers().contains_key(header::CONTENT_LENGTH));
        let mut body=response.into_body().into_data_stream();
        assert_eq!(body.next().await.unwrap().unwrap().as_ref(),b"RIFF");
        audio.send(vec![1,2]).unwrap();assert_eq!(body.next().await.unwrap().unwrap().as_ref(),[1,2]);
        drop(alive);assert!(body.next().await.is_none());
        let (_alive,stopped)=tokio::sync::watch::channel(());
        let response=audio_out_response(crate::contract::AudioOut{prime:vec![0],audio:audio.subscribe(),stopped,expires:Instant::now()+Duration::from_millis(10)});
        let mut body=response.into_body().into_data_stream();
        assert!(body.next().await.is_some());
        assert!(tokio::time::timeout(Duration::from_millis(100),body.next()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn dinh_tuyen_live_toi_call_control() {
        let (app, call, chat) = app();
        let (st, v) = send(&app, ok("GET", "/api/live/calls", Body::empty())).await;
        assert_eq!((st, v), (200, json!({"ok": true, "from": "calls"})));
        for (p, name) in [("call", "call_out"), ("accept", "accept"), ("hangup", "hangup"), ("settings", "settings"),
                          ("media", "media"), ("playback", "playback"), ("say", "say")] {
            let (st, _) = send(&app, ok("POST", &format!("/api/live/{p}"), r#"{"id":"c1"}"#)).await;
            assert_eq!(st, 200, "{p}");
            assert_eq!(call.last(), (name.into(), "".into(), "".into(), json!({"id": "c1"})), "{p}");
        }
        // ?account= được chép vào thân cho CallControl.
        send(&app, ok("POST", "/api/live/call?account=%2B84123", r#"{"user_id":5}"#)).await;
        assert_eq!(call.last().3, json!({"user_id": 5, "account": "+84123"}));
        // GET /api/live/settings không có trong CallControl → ChatControl.
        send(&app, ok("GET", "/api/live/settings", Body::empty())).await;
        assert_eq!(chat.last(), ("GET".into(), "/api/live/settings".into(), "".into(), Value::Null));
        assert_eq!(chat.count(), 1);
    }

    #[tokio::test]
    async fn dinh_tuyen_con_lai_toi_chat_control() {
        let (app, call, chat) = app();
        send(&app, ok("GET", "/api/chats/-1001/messages?limit=3&account=x", Body::empty())).await;
        assert_eq!(chat.last(), ("GET".into(), "/api/chats/-1001/messages".into(), "limit=3&account=x".into(), Value::Null));
        for p in ["/api/accounts", "/api/chats", "/api/contacts", "/api/feed", "/api/search"] {
            assert_eq!(send(&app, ok("GET", p, Body::empty())).await.0, 200, "{p}");
            assert_eq!(chat.last().1, p);
        }
        for p in ["/api/messages/delete", "/api/messages/edit", "/api/messages/forward", "/api/messages/react",
                  "/api/resolve", "/api/send_voice", "/api/translate", "/api/chats/7/read", "/api/chats/7/send",
                  "/api/chats/7/send_album", "/api/do/getChats"] {
            assert_eq!(send(&app, ok("POST", p, r#"{"k":1}"#)).await.0, 200, "{p}");
            assert_eq!(chat.last(), ("POST".into(), p.into(), "".into(), json!({"k": 1})), "{p}");
        }
        assert_eq!(call.count(), 0);
    }

    #[tokio::test]
    async fn send_file_than_tho_thanh_base64() {
        let (app, _, chat) = app();
        let (st, _) = send(&app, ok("POST", "/api/chats/9/send_file?kind=photo&name=a.jpg", &b"\x00\xffabc"[..])).await;
        assert_eq!(st, 200);
        assert_eq!(chat.last(), ("POST".into(), "/api/chats/9/send_file".into(), "kind=photo&name=a.jpg".into(),
                                 json!({"data_b64": "AP9hYmM="})));
        // Thân trên 400 KB vẫn được (giới hạn 64 MB).
        assert_eq!(send(&app, ok("POST", "/api/chats/9/send_file", vec![1u8; 500_000])).await.0, 200);
        // kind kiểm trước, rồi mới tới tệp rỗng — như Python.
        let kind_msg = err("BAD_REQUEST", "kind phải là photo, video, voice, audio hoặc document.");
        assert_eq!(send(&app, ok("POST", "/api/chats/9/send_file?kind=x", Body::empty())).await, (400, kind_msg));
        let empty = err("BAD_REQUEST", "Tệp rỗng.");
        assert_eq!(send(&app, ok("POST", "/api/chats/9/send_file", Body::empty())).await, (400, empty));
        assert_eq!(chat.count(), 2);
    }

    #[tokio::test]
    async fn gw_error_thanh_json_errors_py() {
        let (app, call, chat) = app();
        *call.reply.lock().unwrap() = Some(Err(GwError::new(409, "ACCOUNT_BUSY", "Tài khoản đang bận.")));
        let got = send(&app, ok("POST", "/api/live/call", "{}")).await;
        assert_eq!(got, (409, err("ACCOUNT_BUSY", "Tài khoản đang bận.")));
        *chat.reply.lock().unwrap() = Some(Err(GwError::not_found("Không có.")));
        assert_eq!(send(&app, ok("GET", "/api/contacts", Body::empty())).await, (404, err("NOT_FOUND", "Không có.")));
        *chat.reply.lock().unwrap() = Some(Err(GwError::new(503, "ACCOUNT_OFFLINE", "m")));
        assert_eq!(send(&app, ok("GET", "/api/chats", Body::empty())).await, (503, err("ACCOUNT_OFFLINE", "m")));
        // Trường retry chỉ xuất hiện khi Telegram cung cấp thời gian chờ.
        for (status, code) in [
            (400, "BAD_REQUEST"), (404, "NOT_FOUND"), (409, "ACCOUNT_BUSY"),
            (409, "CALL_NOT_ACTIVE"), (409, "ENDPOINT_BUSY"), (429, "FLOOD_WAIT"),
            (429, "LOGIN_LOCKED"), (502, "MEDIA_ENGINE_ERROR"),
            (503, "ACCOUNT_OFFLINE"), (501, "NOT_IMPLEMENTED"),
        ] {
            *chat.reply.lock().unwrap() = Some(Err(GwError::new(status, code, "Thông báo từ trait.")));
            assert_eq!(send(&app, ok("GET", "/api/chats", Body::empty())).await,
                       (status, err(code, "Thông báo từ trait.")));
        }
        *chat.reply.lock().unwrap() = Some(Err(GwError::from_tdlib(
            crate::contract::TdError::telegram(429, "FLOOD_WAIT_17"),
        )));
        let response = app.oneshot(ok("GET", "/api/chats", Body::empty())).await.unwrap();
        assert_eq!(response.status(), 429);
        assert_eq!(response.headers().get(header::RETRY_AFTER).and_then(|v| v.to_str().ok()), Some("17"));
        let raw = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&raw).unwrap(), json!({"error":{
            "code":"FLOOD_WAIT", "message":"Telegram yêu cầu chờ trước khi thử lại.", "retry_after_seconds":17
        }}));
    }

    #[tokio::test]
    async fn kiem_truoc_auth_404_405_422_nhu_fastapi() {
        let (app, call, chat) = app();
        assert_eq!(send(&app, req("GET", "/api/nope", None, Body::empty())).await, (404, json!({"detail": "Not Found"})));
        let m = (405, json!({"detail": "Method Not Allowed"}));
        assert_eq!(send(&app, req("POST", "/api/live/calls", None, Body::empty())).await, m);
        assert_eq!(send(&app, req("DELETE", "/health", None, Body::empty())).await, m);
        assert_eq!(send(&app, req("GET", "/api/live/hangup", None, Body::empty())).await, m);
        let want = json!({"detail": [{"type": "int_parsing", "loc": ["path", "chat_id"],
            "msg": "Input should be a valid integer, unable to parse string as an integer", "input": "abc"}]});
        assert_eq!(send(&app, req("GET", "/api/chats/abc/messages", None, Body::empty())).await, (422, want.clone()));
        assert_eq!(send(&app, ok("POST", "/api/chats/abc/send", "{}")).await, (422, want));
        assert_eq!(send(&app, req("GET", "/api/file/x", None, Body::empty())).await.0, 422);
        for path in ["/health", "/api/live/calls", "/api/accounts", "/api/live/cast/c1/tok.mp3"] {
            assert_eq!(send(&app, req("HEAD", path, None, Body::empty())).await.0, 405, "{path}");
        }
        assert_eq!((call.count(), chat.count()), (0, 0));
    }

    #[tokio::test]
    async fn duong_nhi_phan_va_cast() {
        let (app, call, chat) = app();
        let jpeg = app.clone().oneshot(ok("GET", "/api/live/snapshot/c1", Body::empty())).await.unwrap();
        assert_eq!(jpeg.headers()[header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(to_bytes(jpeg.into_body(), 3).await.unwrap().as_ref(), &[1, 2, 3]);
        assert_eq!(call.last(), ("snapshot".into(), "c1".into(), "".into(), Value::Null));
        for path in ["/api/file/12", "/api/calltrace/c1"] {
            let response = app.clone().oneshot(ok("GET", path, Body::empty())).await.unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(to_bytes(response.into_body(), 3).await.unwrap().as_ref(), &[1, 2, 3]);
            assert_eq!(chat.last(), ("GET".into(), path.into(), "".into(), Value::Null));
        }
        let cast = send(&app, req("GET", "/api/live/cast/c1/tok.mp3", None, Body::empty())).await;
        assert_eq!(cast, (404, err("NOT_FOUND", "Không có luồng này.")));
        let cast = app.oneshot(req("GET", "/api/live/cast/c1/test-cast-token.mp3", None, Body::empty())).await.unwrap();
        assert_eq!(cast.headers()[header::CONTENT_TYPE], "audio/mpeg");
        assert_eq!(cast.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(cast.headers()[header::CONTENT_LENGTH], "230400000");
        assert_eq!(to_bytes(cast.into_body(), 6).await.unwrap().as_ref(), &[1, 2, 3, 4, 5, 6]);
    }

    #[tokio::test]
    async fn live_view_xac_thuc_phat_nhieu_anh_va_giai_phong_khi_dong() {
        let (app, call, chat) = app();
        let path = "/api/live/view/c1";
        assert_eq!(send(&app, req("GET", path, None, Body::empty())).await.0, 401);
        assert_eq!(send(&app, ok("GET", path, Body::empty())).await.0, 404);

        let first = &b"\xff\xd8a\xff\xd9"[..];
        let tx = watch::Sender::new(Some(Arc::<[u8]>::from(first)));
        *call.video.lock().unwrap() = Some(tx.clone());
        let response = app.clone().oneshot(ok("GET", path, Body::empty())).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "multipart/x-mixed-replace; boundary=frame");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let mut stream = response.into_body().into_data_stream();
        assert_eq!(stream.next().await.unwrap().unwrap().as_ref(),
            b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 5\r\n\r\n\xff\xd8a\xff\xd9\r\n");
        tx.send_replace(Some(Arc::from(&b"\xff\xd8bc\xff\xd9"[..])));
        assert_eq!(stream.next().await.unwrap().unwrap().as_ref(),
            b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 6\r\n\r\n\xff\xd8bc\xff\xd9\r\n");
        drop(stream);
        assert_eq!(tx.receiver_count(), 0);

        let response = app.oneshot(ok("GET", path, Body::empty())).await.unwrap();
        let mut stream = response.into_body().into_data_stream();
        assert!(stream.next().await.is_some());
        *call.video.lock().unwrap() = None;
        drop(tx);
        assert!(tokio::time::timeout(Duration::from_millis(100), stream.next()).await.unwrap().is_none());
        assert_eq!(chat.count(), 0);
    }

    #[tokio::test]
    async fn live_view_khong_co_anh_tra_409_va_mat_anh_dong_luong() {
        let wait = Duration::from_millis(5);
        let tx = watch::Sender::new(None);
        let response = live_view(tx.subscribe(), wait, wait).await;
        assert_eq!(response.status(), 409);
        let raw = to_bytes(response.into_body(), 4096).await.unwrap();
        let body: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(body["error"]["code"], "NO_VIDEO");

        tx.send_replace(Some(Arc::from(&b"\xff\xd8a\xff\xd9"[..])));
        let response = live_view(tx.subscribe(), wait, wait).await;
        let mut stream = response.into_body().into_data_stream();
        assert!(stream.next().await.is_some());
        assert!(tokio::time::timeout(Duration::from_millis(100), stream.next()).await.unwrap().is_none());
        let frames = tx.subscribe();
        drop(tx);
        assert_eq!(live_view(frames, wait, wait).await.status(), 404);
    }

    #[tokio::test]
    async fn chua_co_tra_501_va_calls_rong() {
        let tam = Arc::new(ChuaCo);
        let cfg = RestConfig { token: TOK.into(), rate_limit_per_minute: 120, version: "thu".into() };
        let app = router(tam.clone(), tam, cfg);
        assert_eq!(send(&app, ok("GET", "/api/live/calls", Body::empty())).await, (200, json!({"calls": []})));
        let (st, v) = send(&app, ok("POST", "/api/chats/1/send", "{}")).await;
        assert_eq!((st, v["error"]["code"].clone()), (501, json!("NOT_IMPLEMENTED")));
        assert_eq!(send(&app, req("GET", "/health", None, Body::empty())).await.1["status"], "degraded");
    }

    #[test]
    fn so_sanh_thoi_gian_hang() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
        assert!(ct_eq(b"", b""));
    }

    #[tokio::test]
    async fn appearance_and_profile_routes_enforce_auth_methods_and_forward_to_chat() {
        let (app, call, chat) = app();
        for path in ["/api/settings/appearance", "/api/profile"] {
            assert_eq!(send(&app, req("GET", path, None, Body::empty())).await.0, 401, "{path}");
            assert_eq!(send(&app, req("POST", path, None, "{}")).await.0, 401, "{path}");
            assert_eq!(send(&app, ok("DELETE", path, Body::empty())).await.0, 405, "{path}");
            assert_eq!(send(&app, ok("GET", path, Body::empty())).await.0, 200, "{path}");
            assert_eq!(chat.last(), ("GET".into(), path.into(), "".into(), Value::Null));
            assert_eq!(send(&app, ok("POST", path, r#"{"k":1}"#)).await.0, 200, "{path}");
            assert_eq!(chat.last(), ("POST".into(), path.into(), "".into(), json!({"k": 1})));
        }
        assert_eq!(call.count(), 0);
    }

    #[tokio::test]
    async fn chat_info_routes_enforce_auth_methods_int_param_and_forward_to_chat() {
        let (app, call, chat) = app();
        for (method, path, bad_method, bad_id_path) in [
            ("GET", "/api/chats/42/info", "POST", "/api/chats/abc/info"),
            ("GET", "/api/chats/-1001/members", "DELETE", "/api/chats/abc/members"),
            ("POST", "/api/chats/42/settings", "GET", "/api/chats/abc/settings"),
            ("POST", "/api/chats/42/clear_history", "GET", "/api/chats/abc/clear_history"),
            ("POST", "/api/chats/42/delete", "GET", "/api/chats/abc/delete"),
            ("POST", "/api/chats/-1001/leave", "GET", "/api/chats/abc/leave"),
        ] {
            assert_eq!(send(&app, req(method, bad_id_path, None, Body::empty())).await.0, 422, "{method} {bad_id_path}");
            assert_eq!(send(&app, req(method, path, None, "{}")).await.0, 401, "{method} {path}");
            assert_eq!(send(&app, ok(bad_method, path, "{}")).await.0, 405, "{bad_method} {path}");
            let body = if method == "POST" { r#"{"confirm":true}"# } else { "" };
            assert_eq!(send(&app, ok(method, path, body)).await.0, 200, "{method} {path}");
            let expected_body = if method == "POST" { json!({"confirm": true}) } else { Value::Null };
            assert_eq!(chat.last(), (method.into(), path.into(), "".into(), expected_body));
        }
        assert_eq!(send(&app, ok("GET", "/api/chats/-1001/members?q=an&limit=10", Body::empty())).await.0, 200);
        assert_eq!(chat.last(), ("GET".into(), "/api/chats/-1001/members".into(), "q=an&limit=10".into(), Value::Null));
        assert_eq!(call.count(), 0);
    }

    #[test]
    fn thieu_gateway_token_tu_choi_khoi_dong_bang_tieng_viet() {
        let old = std::env::var_os("GATEWAY_TOKEN");
        std::env::remove_var("GATEWAY_TOKEN");

        let result = serve_config_from_env();

        if let Some(value) = old {
            std::env::set_var("GATEWAY_TOKEN", value);
        }
        let error = result.err().expect("thiếu token phải từ chối cấu hình");
        assert!(error.contains("GATEWAY_TOKEN") && error.contains("chưa đặt"), "{error}");
    }
}
