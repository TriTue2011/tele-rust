use super::*;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};
use axum::{body::{to_bytes, Body}, http::Request};
use tower::ServiceExt;
use crate::rest::{self, RestConfig};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        Self(PathBuf::from(format!("target/event-test-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))))
    }
}

impl Drop for Temp {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

async fn wait_for_cursor(store: &EventStore, cursor: u64) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = store.poll(0).await.unwrap();
            if value["cursor"].as_u64().unwrap() >= cursor { return value; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap()
}

#[tokio::test]
async fn event_poll_auth_shape_permission_and_cursor_after_restart() {
    let root = Temp::new();
    let bridge = EventBridge::start(&root.0, "+000123", None).await.unwrap();
    let (calls, source) = broadcast::channel(16);
    let follower = tokio::spawn(bridge.sink.clone().follow_calls(source));
    let call = json!({"id":"abc123","outgoing":true,"seconds":1.5});
    calls.send(json!({"kind":"started","peer":"••••","call":call})).unwrap();
    calls.send(json!({"kind":"active","call":call})).unwrap();
    calls.send(json!({"kind":"ended","reason":"người dùng cúp","call":call})).unwrap();
    drop(calls);
    follower.await.unwrap();
    let want = wait_for_cursor(&bridge.store, 2).await;
    assert_eq!(want["events"].as_array().unwrap().len(), 2);
    let first = &want["events"][0];
    assert!(first["timestamp"].is_f64());
    assert_eq!(first["id"], 1);
    assert_eq!(first["type"], "call_started");
    assert_eq!(first["account"], "+000123");
    assert_eq!(first["data"], json!({"call_id":"abc123","peer":"••••","mode":"live","mock":false}));
    assert_eq!(want["events"][1]["type"], "call_ended");
    assert_eq!(want["events"][1]["data"], json!({"call_id":"abc123","reason":"người dùng cúp","duration_seconds":1.5}));
    let dir = root.0.join("events");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    let file = dir.join("000000000001.json");
    assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(serde_json::from_slice::<Value>(&fs::read(file).unwrap()).unwrap(), *first);
    bridge.shutdown().await;

    let bridge = EventBridge::start(&root.0, "+000123", None).await.unwrap();
    assert_eq!(bridge.store.poll(0).await.unwrap(), want);
    bridge.sink.emit("account_status", json!({"status":"online"}));
    let all = wait_for_cursor(&bridge.store, 3).await;
    assert_eq!(all["events"][2]["id"], 3);
    let app = rest::router_with_events(Arc::new(rest::ChuaCo), Arc::new(rest::ChuaCo), RestConfig {
        token: "rest-test".into(), rate_limit_per_minute: 120, version: "test".into(),
    }, Some(bridge.store.clone()));
    let get = |uri, auth: Option<&str>| {
        let mut req = Request::builder().uri(uri);
        if let Some(auth) = auth { req = req.header("Authorization", auth); }
        app.clone().oneshot(req.body(Body::empty()).unwrap())
    };
    assert_eq!(get("/api/events", None).await.unwrap().status(), 401);
    assert_eq!(get("/api/events?since=bad", None).await.unwrap().status(), 422);
    let response = get("/api/events?since=2", Some("Bearer rest-test")).await.unwrap();
    assert_eq!(response.status(), 200);
    let raw = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&raw).unwrap(), json!({"events":[all["events"][2]],"cursor":3}));
    drop(app);
    bridge.shutdown().await;
}

#[tokio::test]
async fn account_status_reports_disconnect_and_waits_for_tdlib_to_finish_catchup() {
    let root = Temp::new();
    let bridge = EventBridge::start(&root.0, "+000123", None).await.unwrap();
    let connection = |state: &str| json!({"@type":"updateConnectionState", "state":{"@type":state}});
    let authorization = |state: &str| json!({"@type":"updateAuthorizationState", "authorization_state":{"@type":state}});
    bridge.sink.account_update(&authorization("authorizationStateReady"));
    bridge.sink.account_update(&connection("connectionStateReady"));
    bridge.sink.account_login(true);
    bridge.sink.account_update(&connection("connectionStateWaitingForNetwork"));
    bridge.sink.account_update(&connection("connectionStateConnecting"));
    bridge.sink.account_update(&connection("connectionStateUpdating"));
    let offline = wait_for_cursor(&bridge.store, 2).await;
    assert_eq!(offline["events"][1]["data"], json!({"status":"offline"}));
    bridge.sink.account_update(&connection("connectionStateReady"));
    bridge.sink.account_update(&connection("connectionStateReady"));
    bridge.sink.account_update(&authorization("authorizationStateClosed"));
    let events = wait_for_cursor(&bridge.store, 4).await;
    let statuses: Vec<_> = events["events"].as_array().unwrap().iter().map(|e| {
        assert_eq!(e["type"], "account_status");
        assert_eq!(e["account"], "+000123");
        e["data"]["status"].as_str().unwrap()
    }).collect();
    assert_eq!(statuses, ["online", "offline", "online", "offline"]);
    bridge.shutdown().await;
}

#[test]
fn rotation_keeps_latest_files_and_ignores_uncommitted_write() {
    let root = Temp::new();
    let dir = root.0.join("events");
    fs::create_dir_all(&dir).unwrap();
    for id in 1..=270u64 {
        let path = dir.join(format!("{id:012}.json"));
        fs::write(path, json!({"id":id,"data":"a".repeat(64_000)}).to_string()).unwrap();
    }
    fs::write(dir.join(".pending"), b"incomplete").unwrap();
    let mut spool = Spool::open(dir.clone()).unwrap();
    assert!(!dir.join("000000000001.json").exists());
    assert!(!dir.join(".pending").exists());
    assert_eq!(spool.poll(269)["events"][0]["id"], 270);
    let event = spool.append(json!({"type":"account_status", "timestamp": 1795202064.6094441})).unwrap();
    assert_eq!(event["id"], 271);
    drop(spool);
    assert_eq!(Spool::open(dir).unwrap().poll(270)["events"][0], event);
}

#[tokio::test]
async fn one_spooled_event_posts_the_same_envelope_and_token_once() {
    use axum::{extract::State, http::{HeaderMap, StatusCode}, routing::post, Json, Router};
    let root = Temp::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, mut received) = mpsc::channel(8);
    let app = Router::new().route("/ha", post(|State(tx): State<mpsc::Sender<(HeaderMap, Value)>>,
        headers: HeaderMap, Json(event): Json<Value>| async move {
        tx.send((headers, event)).await.unwrap();
        StatusCode::NO_CONTENT
    })).with_state(tx);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let webhook = Webhook::new(&format!("http://localhost:{port}/ha"), "webhook-test").unwrap();
    let bridge = EventBridge::start(&root.0, "+000123", Some(webhook)).await.unwrap();
    bridge.sink.emit("message_new", json!({"peer":"••••","tin_id":"7"}));
    let (headers, delivered) = tokio::time::timeout(Duration::from_secs(5), received.recv()).await.unwrap().unwrap();
    assert_eq!(headers["X-Gateway-Token"], "webhook-test");
    assert_eq!(headers["Content-Type"], "application/json");
    assert_eq!(bridge.store.poll(0).await.unwrap(), json!({"events":[delivered],"cursor":1}));
    let dir = root.0.join("events");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    assert_eq!(serde_json::from_slice::<Value>(&fs::read(dir.join("000000000001.json")).unwrap()).unwrap(), delivered);
    assert!(tokio::time::timeout(Duration::from_millis(1250), received.recv()).await.is_err());
    bridge.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn unavailable_webhook_cannot_block_spool_and_has_a_retry_limit() {
    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
    use tokio::sync::Notify;
    let root = Temp::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let notify = Arc::new(Notify::new());
    let (tx, mut received) = mpsc::channel(8);
    let app = Router::new().route("/ha", post(|State((tx, wait)): State<(mpsc::Sender<u64>, Arc<Notify>)>,
        Json(event): Json<Value>| async move {
        let id = event["id"].as_u64().unwrap();
        tx.send(id).await.unwrap();
        if id == 1 { wait.notified().await; }
        StatusCode::SERVICE_UNAVAILABLE
    })).with_state((tx, notify.clone()));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let webhook = Webhook::new(&format!("http://{addr}/ha"), "test").unwrap();
    let bridge = EventBridge::start(&root.0, "test", Some(webhook)).await.unwrap();
    bridge.sink.emit("message_new", json!({"tin_id":"1"}));
    assert_eq!(tokio::time::timeout(Duration::from_secs(5), received.recv()).await.unwrap(), Some(1));
    bridge.sink.emit("message_new", json!({"tin_id":"2"}));
    assert_eq!(wait_for_cursor(&bridge.store, 2).await["events"].as_array().unwrap().len(), 2);
    notify.notify_one();
    for expected in [1, 1, 2] {
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), received.recv()).await.unwrap(), Some(expected));
        if expected == 1 { notify.notify_one(); }
    }
    bridge.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn webhook_blocks_link_local_schemes_and_redirects() {
    use axum::{extract::State, http::StatusCode, routing::post, Router};
    for url in ["file:///data/events", "ftp://localhost/ha", "http://169.254.0.1/ha",
        "https://169.254.255.254/ha", "http://[::ffff:a9fe:1]/ha", "http://[fd00:ec2::254]/ha",
        "http://[64:ff9b::a9fe:a9fe]/ha", "http://[::ffff:0:a9fe:a9fe]/ha", "http://0.0.0.0/ha",
        "http://[::]/ha", "http://255.255.255.255/ha", "http://224.0.0.1/ha", "http://[ff02::1]/ha"] {
        assert!(Webhook::new(url, "test").is_none(), "{url}");
    }
    assert!(Webhook::new("http://127.0.0.1:8123/ha", "test").is_some());
    assert!(Webhook::new("https://192.168.1.2/ha", "test").is_some());
    let root = Temp::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, mut received) = mpsc::channel(8);
    let app = Router::new().route("/ha", post(|State(tx): State<mpsc::Sender<&'static str>>| async move {
        tx.send("ha").await.unwrap();
        (StatusCode::TEMPORARY_REDIRECT, [("Location", "/redirected")])
    })).route("/redirected", post(|State(tx): State<mpsc::Sender<&'static str>>| async move {
        tx.send("redirected").await.unwrap();
        StatusCode::OK
    })).with_state(tx);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let webhook = Webhook::new(&format!("http://{addr}/ha"), "test").unwrap();
    let bridge = EventBridge::start(&root.0, "test", Some(webhook)).await.unwrap();
    bridge.sink.emit("message_new", json!({"tin_id":"1"}));
    assert_eq!(tokio::time::timeout(Duration::from_secs(5), received.recv()).await.unwrap(), Some("ha"));
    assert!(tokio::time::timeout(Duration::from_millis(1250), received.recv()).await.is_err());
    bridge.shutdown().await;
    server.abort();
}

#[test]
fn rotate_recovers_when_oldest_event_file_is_missing_on_disk() {
    let root = Temp::new();
    let dir = root.0.join("events");
    fs::create_dir_all(&dir).unwrap();
    for id in 1..=270_u64 {
        let path = dir.join(format!("{id:012}.json"));
        fs::write(path, json!({"id":id,"data":"a".repeat(64_000)}).to_string()).unwrap();
    }
    let mut spool = Spool::open(dir.clone()).unwrap();
    let mut files: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".json"))
        .collect();
    files.sort();
    let oldest = files[0].clone();
    let second_oldest = files[1].clone();
    fs::remove_file(dir.join(&oldest)).unwrap();
    let payload = "b".repeat(64_000);
    assert!(spool.append(json!({"type":"account_status","data":payload})).is_ok());
    assert!(spool.append(json!({"type":"account_status","data":payload})).is_ok());
    assert!(!dir.join(&second_oldest).exists());
}

#[tokio::test]
async fn account_status_retries_after_full_queue_drop() {
    let (tx, mut rx) = mpsc::channel(1);
    let sink = EventSink { account: "+000123".into(), queue: tx, status: Default::default() };
    sink.emit("message_new", json!({"tin_id":"dummy"}));
    sink.account_login(true);
    let first = rx.try_recv().unwrap();
    assert_eq!(first["type"], "message_new");
    assert!(rx.try_recv().is_err());
    sink.account_update(&json!({"@type":"updateConnectionState","state":{"@type":"connectionStateReady"}}));
    let retried = rx.try_recv().unwrap();
    assert_eq!(retried["type"], "account_status");
    assert_eq!(retried["data"], json!({"status":"online"}));
}
