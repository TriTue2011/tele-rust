//! Khởi tạo và quản lý vòng đời của tiến trình `tggw serve`.

#[cfg(test)]
use std::{future::Future,sync::Arc};
use std::time::Duration;

#[cfg(test)]
use serde_json::Value;
use tokio::sync::oneshot;
#[cfg(test)]
use tokio::sync::watch;
#[cfg(test)]
use tokio::task::JoinHandle;

#[cfg(test)]
use crate::contract::{Binary, BoxFut, CallControl, CastAudio, ChatControl, GwError, VideoFrames};
use crate::{rest, td};

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoginStatus {
    Starting,
    Ready,
    Failed,
}

#[cfg(test)]
#[derive(Clone)]
struct Session {
    auth: watch::Receiver<String>,
    status: watch::Receiver<LoginStatus>,
}

#[cfg(test)]
impl Session {
    fn is_ready(&self) -> bool {
        *self.auth.borrow() == "authorizationStateReady" && *self.status.borrow() == LoginStatus::Ready
    }
}

#[cfg(test)]
fn account_offline<'a, T: 'a>() -> BoxFut<'a, Result<T, GwError>> {
    Box::pin(async {
        Err(GwError::new(503, "ACCOUNT_OFFLINE", "Tài khoản đang mất kết nối tới Telegram."))
    })
}

#[cfg(test)]
fn spawn_login<F>(login: F, status: watch::Sender<LoginStatus>) -> JoinHandle<()>
where
    F: Future<Output = Result<(), String>> + Send + 'static,
{
    tokio::spawn(async move {
        match login.await {
            Ok(()) => { let _ = status.send(LoginStatus::Ready); }
            Err(_) => {
                let _ = status.send(LoginStatus::Failed);
                log::error!("LỖI: đăng nhập nền thất bại; HTTP vẫn hoạt động.");
            }
        }
    })
}

#[cfg(test)]
async fn stop_services<C, F>(calls: C, close_tdlib: F)
where
    C: Future<Output = Result<(), String>>,
    F: Future<Output = ()>,
{
    if let Err(e) = calls.await {
        log::error!("LỖI: không dọn được CallManager hoàn toàn: {e}");
    }
    close_tdlib.await;
}

#[cfg(test)]
struct ReadyCalls {
    session: Session,
    inner: Arc<dyn CallControl>,
}

#[cfg(test)]
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
    fn video_frames(&self, id: &str) -> BoxFut<'_, Result<VideoFrames, GwError>> {
        self.inner.video_frames(id)
    }
}

#[cfg(test)]
struct ReadyChat {
    session: Session,
    inner: Arc<dyn ChatControl>,
}

#[cfg(test)]
impl ChatControl for ReadyChat {
    fn handle(&self, method: &str, path: &str, query: &str, body: Value) -> BoxFut<'_, Result<Value, GwError>> {
        if path == "/api/calltrace" || self.session.is_ready() { self.inner.handle(method, path, query, body) } else { account_offline() }
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
        let secure_cookie = crate::ui_auth::secure_cookie_from_env()?;
        let data_dir = login_config.data_dir.clone();
        let ui_auth = std::sync::Arc::new(crate::ui_auth::UiAuth::open(
            &data_dir,
            &rest_config.token,
            std::env::var("TG_UI_USER").ok().as_deref(),
            std::env::var("TG_UI_PASSWORD").ok().as_deref(),
            secure_cookie,
        )?);
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("Không nghe được {addr}: {e}"))?;
        let cast_port = listener.local_addr().map_err(|_| "Không đọc được cổng HTTP.")?.port();
        let library = std::env::var("TDJSON_LIB").unwrap_or_else(|_| "libtdjson.so".into());
        let accounts = crate::account_router::Accounts::new(login_config, library, cast_port).await?;
        let app = rest::router_accounts_with_ui_auth(accounts.clone(), rest_config, ui_auth);
        let (stop_tx, stop_rx) = oneshot::channel();
        let mut server = tokio::spawn(rest::serve(listener, app, async move { let _ = stop_rx.await; }));
        let sweeping = accounts.clone();
        let (sweep_stop_tx, mut sweep_stop_rx) = oneshot::channel::<()>();
        let mut sweeper = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = &mut sweep_stop_rx => break,
                    _ = tick.tick() => {
                        if let Some(logins) = sweeping.logins.get() {
                            logins.expire().await;
                        }
                    }
                }
            }
        });

        let result = tokio::select! {
            response = &mut server => server_result(response),
            _ = rest::shutdown_signal() => {
                let _ = stop_tx.send(());
                match tokio::time::timeout(Duration::from_secs(10), &mut server).await {
                    Ok(response) => server_result(response),
                    Err(_) => {
                        server.abort();
                        let _ = server.await;
                        log::error!("LỖI: HTTP quá thời gian dừng êm; đã đóng các kết nối còn lại.");
                        Ok(())
                    }
                }
            }
        };

        let _ = sweep_stop_tx.send(());
        if tokio::time::timeout(Duration::from_secs(10), &mut sweeper).await.is_err() {
            sweeper.abort();
            let _ = sweeper.await;
        }
        accounts.shutdown().await;
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
