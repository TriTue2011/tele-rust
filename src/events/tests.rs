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
    let bridge = EventBridge::start(&root.0, "+000123").await.unwrap();
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

    let bridge = EventBridge::start(&root.0, "+000123").await.unwrap();
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
    assert_eq!(spool.append(json!({"type":"account_status"})).unwrap()["id"], 271);
}
