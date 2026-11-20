//! Khởi tạo và quản lý vòng đời của tiến trình `tggw serve`.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::calls::CallManager;
use crate::chat::ChatService;
use crate::contract::{Binary, BoxFut, CallControl, CastAudio, ChatControl, GwError, TdLink};
use crate::events::{EventBridge, Webhook};
use crate::{rest, td};

const CALLS_SHUTDOWN_WAIT: Duration = Duration::from_secs(10);
const TDLIB_CLOSE_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoginStatus {
    Starting,
    Ready,
    Failed,
}

#[derive(Clone)]
struct Session {
    auth: watch::Receiver<String>,
    status: watch::Receiver<LoginStatus>,
}

impl Session {
    fn is_ready(&self) -> bool {
        *self.auth.borrow() == "authorizationStateReady" && *self.status.borrow() == LoginStatus::Ready
    }
}

fn account_offline<'a, T: 'a>() -> BoxFut<'a, Result<T, GwError>> {
    Box::pin(async {
        Err(GwError::new(503, "ACCOUNT_OFFLINE", "Tài khoản đang mất kết nối tới Telegram."))
    })
}

fn spawn_login<F>(login: F, status: watch::Sender<LoginStatus>) -> JoinHandle<()>
where
    F: Future<Output = Result<(), String>> + Send + 'static,
{
    tokio::spawn(async move {
        match login.await {
            Ok(()) => { let _ = status.send(LoginStatus::Ready); }
            Err(_) => {
                let _ = status.send(LoginStatus::Failed);
                eprintln!("LỖI: đăng nhập nền thất bại; HTTP vẫn hoạt động.");
            }
        }
    })
}

async fn stop_services<C, F>(calls: C, close_tdlib: F)
where
    C: Future<Output = Result<(), String>>,
    F: Future<Output = ()>,
{
    if let Err(e) = calls.await {
        eprintln!("LỖI: không dọn được CallManager hoàn toàn: {e}");
    }
    close_tdlib.await;
}

struct ReadyCalls {
    session: Session,
    inner: Arc<dyn CallControl>,
}

impl CallControl for ReadyCalls {
    fn calls(&self) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.calls() } else { account_offline() }
    }
    fn call_out(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.call_out(body) } else { account_offline() }
    }
    fn accept(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.accept(body) } else { account_offline() }
    }
    fn hangup(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.hangup(body) } else { account_offline() }
    }
    fn settings(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.settings(body) } else { account_offline() }
    }
    fn media(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.media(body) } else { account_offline() }
    }
    fn playback(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.playback(body) } else { account_offline() }
    }
    fn say(&self, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.say(body) } else { account_offline() }
    }
    fn snapshot(&self, id: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        if self.session.is_ready() { self.inner.snapshot(id) } else { account_offline() }
    }
    fn cast_audio(&self, id: &str, token: &str) -> BoxFut<'_, Option<CastAudio>> {
        self.inner.cast_audio(id, token)
    }
}

struct ReadyChat {
    session: Session,
    inner: Arc<dyn ChatControl>,
}

impl ChatControl for ReadyChat {
    fn handle(&self, method: &str, path: &str, query: &str, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if self.session.is_ready() { self.inner.handle(method, path, query, body) } else { account_offline() }
    }
    fn bytes(&self, path: &str, query: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        if path.starts_with("/api/calltrace/") || self.session.is_ready() {
            self.inner.bytes(path, query)
        } else { account_offline() }
    }
}

pub struct App;

impl App {
    /// Khởi chạy HTTP ngay sau khi dựng TDLib và CallManager; đăng nhập tiếp tục ở nền.
    pub async fn start() -> Result<(), String> {
        let login_config = td::login::LoginConfig::from_env()
            .map_err(|e| format!("Cấu hình đăng nhập không hợp lệ: {e}"))?;
        let (addr, rest_config) = rest::serve_config_from_env()?;
        let events = EventBridge::start(&login_config.data_dir, &login_config.phone, Webhook::from_env()).await?;
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("Không nghe được {addr}: {e}"))?;
        let cast_port = listener.local_addr().map_err(|_| "Không đọc được cổng HTTP.")?.port();
        let library = std::env::var("TDJSON_LIB").unwrap_or_else(|_| "libtdjson.so".into());
        let client = td::TdClient::open(&library)
            .map_err(|e| format!("Không khởi tạo được TDLib: {e}"))?;
        let td_link: Arc<dyn TdLink> = client.clone();
        let manager = match CallManager::with_cast(td_link.clone(), &login_config.data_dir, Some(cast_port)) {
            Ok(manager) => manager,
            Err(e) => {
                client.close(TDLIB_CLOSE_WAIT).await;
                return Err(format!("Không khởi tạo được CallManager: {}", e.message));
            }
        };

        let (login_tx, login_rx) = watch::channel(LoginStatus::Starting);
        let mut call_events = tokio::spawn(events.sink.clone().follow_calls(manager.events()));
        let auth_rx = client.auth_watch();
        let call_control: Arc<dyn CallControl> = Arc::new(ReadyCalls {
            session: Session { auth: auth_rx.clone(), status: login_rx.clone() },
            inner: Arc::new(manager.clone()),
        });
        let chat: Arc<dyn ChatControl> = Arc::new(ReadyChat {
            session: Session { auth: auth_rx, status: login_rx },
            inner: Arc::new(ChatService::with_events(td_link, login_config.phone.clone(), &login_config.data_dir, Some(events.sink.clone()))),
        });
        let app = rest::router_with_events(call_control, chat, rest_config, Some(events.store.clone()));
        let (stop_tx, stop_rx) = oneshot::channel();
        let mut server = tokio::spawn(rest::serve(listener, app, async move { let _ = stop_rx.await; }));
        let login_client = client.clone();
        let login_events = events.sink.clone();
        let login_task = spawn_login(async move {
            let result = td::login::run(&login_client, &login_config).await;
            let status = if result.is_ok() { "online" } else { "offline" };
            login_events.emit("account_status", serde_json::json!({"status":status}));
            result
        }, login_tx);

        let result = tokio::select! {
            response = &mut server => server_result(response),
            _ = rest::shutdown_signal() => {
                let _ = stop_tx.send(());
                match tokio::time::timeout(Duration::from_secs(10), &mut server).await {
                    Ok(response) => server_result(response),
                    Err(_) => {
                        server.abort();
                        let _ = server.await;
                        eprintln!("LỖI: HTTP quá thời gian dừng êm; đã đóng các kết nối còn lại.");
                        Ok(())
                    }
                }
            }
        };

        login_task.abort();
        let _ = login_task.await;
        stop_services(
            manager.shutdown(CALLS_SHUTDOWN_WAIT),
            async { client.close(TDLIB_CLOSE_WAIT).await },
        ).await;
        drop(manager);
        if tokio::time::timeout(Duration::from_secs(1), &mut call_events).await.is_err() { call_events.abort(); }
        events.shutdown().await;
        result
    }
}

fn server_result(result: Result<Result<(), String>, tokio::task::JoinError>) -> Result<(), String> {
    result.map_err(|e| format!("luồng HTTP lỗi: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    async fn send(app: &axum::Router, uri: &str) -> (u16, Value) {
        let request = Request::builder()
            .uri(uri)
            .header("Authorization", "Bearer app-test-token")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn http_starts_before_ready_and_routes_after_auth_update() {
        let (status_tx, status_rx) = watch::channel(LoginStatus::Starting);
        let (auth_tx, auth_rx) = watch::channel(String::new());
        let auth_for_login = auth_rx.clone();
        let status_for_failure = status_tx.clone();
        let login_task = spawn_login(async move {
            let mut auth = auth_for_login;
            auth.wait_for(|state| state == "authorizationStateReady")
                .await
                .map(|_| ())
                .map_err(|_| "TDLib đã dừng".to_owned())
        }, status_tx);
        let call: Arc<dyn CallControl> = Arc::new(ReadyCalls {
            session: Session { auth: auth_rx.clone(), status: status_rx.clone() },
            inner: Arc::new(rest::ChuaCo),
        });
        let chat: Arc<dyn ChatControl> = Arc::new(ReadyChat {
            session: Session { auth: auth_rx, status: status_rx },
            inner: Arc::new(rest::ChuaCo),
        });
        let app = rest::router(call, chat, rest::RestConfig {
            token: "app-test-token".into(),
            rate_limit_per_minute: 120,
            version: "test".into(),
        });

        let offline = json!({"error":{"code":"ACCOUNT_OFFLINE","message":"Tài khoản đang mất kết nối tới Telegram."}});
        assert_eq!(send(&app, "/api/live/calls").await, (503, offline.clone()));
        assert_eq!(send(&app, "/api/chats").await, (503, offline.clone()));

        auth_tx.send("authorizationStateReady".into()).unwrap();
        login_task.await.unwrap();
        assert_eq!(send(&app, "/api/live/calls").await, (200, json!({"calls":[]})));
        assert_eq!(send(&app, "/api/chats").await.1["error"]["code"], "NOT_IMPLEMENTED");

        status_for_failure.send(LoginStatus::Failed).unwrap();
        assert_eq!(send(&app, "/api/live/calls").await, (503, offline));
    }

    #[tokio::test]
    async fn login_failure_is_recorded_without_stopping_http() {
        let (status_tx, status_rx) = watch::channel(LoginStatus::Starting);
        let task = spawn_login(async { Err("FLOOD_WAIT chi tiết riêng tư".into()) }, status_tx);
        task.await.unwrap();
        assert_eq!(*status_rx.borrow(), LoginStatus::Failed);

        let (auth_tx, auth_rx) = watch::channel(String::new());
        let call: Arc<dyn CallControl> = Arc::new(ReadyCalls {
            session: Session { auth: auth_rx.clone(), status: status_rx.clone() },
            inner: Arc::new(rest::ChuaCo),
        });
        let chat: Arc<dyn ChatControl> = Arc::new(ReadyChat {
            session: Session { auth: auth_rx, status: status_rx },
            inner: Arc::new(rest::ChuaCo),
        });
        let app = rest::router(call, chat, rest::RestConfig {
            token: "app-test-token".into(),
            rate_limit_per_minute: 120,
            version: "test".into(),
        });
        assert_eq!(send(&app, "/health").await.0, 200);
        assert_eq!(send(&app, "/api/live/calls").await.0, 503);
        auth_tx.send("authorizationStateReady".into()).unwrap();
        assert_eq!(send(&app, "/api/live/calls").await.0, 503);
    }

    #[tokio::test]
    async fn shutdown_cleans_calls_before_closing_tdlib_exactly_once() {
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls_order = order.clone();
        let close_order = order.clone();
        let close_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let close_count_for_action = close_count.clone();
        stop_services(
            async move {
                calls_order.lock().unwrap().push("calls");
                Ok(())
            },
            async move {
                close_count_for_action.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                close_order.lock().unwrap().push("tdlib");
            },
        ).await;
        assert_eq!(*order.lock().unwrap(), vec!["calls", "tdlib"]);
        assert_eq!(close_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
