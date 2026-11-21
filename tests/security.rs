use std::{sync::Arc, time::{Duration, Instant}};
use axum::{body::{to_bytes, Body}, http::{header, Request}, response::Response, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use telegram_gateway_rs::{contract::{AudioOut, Binary, BoxFut, CallControl, CastAudio, ChatControl, GwError, VideoFrames}, rest::{self, RestConfig}, ui_auth::UiAuth};
use tower::ServiceExt;

const TOKEN: &str = "token-gia-de-thu";
const USER: &str = "tai-khoan-gia";
const PASSWORD: &str = "mat-khau-gia-de-thu";

struct Media {
    frames: tokio::sync::watch::Sender<Option<Arc<[u8]>>>,
    audio: tokio::sync::broadcast::Sender<Vec<u8>>,
    alive: tokio::sync::watch::Sender<()>,
}

macro_rules! unused_call {
    ($($name:ident),*) => { $(fn $name(&self, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        Box::pin(async { Ok(json!({"ok":true})) })
    })* };
}

impl CallControl for Media {
    fn calls(&self) -> BoxFut<'_, Result<Value, GwError>> { Box::pin(async { Ok(json!({"calls":[]})) }) }
    unused_call!(call_out, accept, hangup, settings, media, playback, say);
    fn snapshot(&self, _: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        Box::pin(async { Ok(binary("image/jpeg", b"\xff\xd8\xff\xd9")) })
    }
    fn video_frames(&self, _: &str) -> BoxFut<'_, Result<VideoFrames, GwError>> {
        Box::pin(async { Ok(self.frames.subscribe()) })
    }
    fn audio_out(&self, _: &str, _: &str) -> BoxFut<'_, Result<AudioOut, GwError>> {
        Box::pin(async { Ok(AudioOut { prime: b"RIFF".to_vec(), audio: self.audio.subscribe(),
            stopped: self.alive.subscribe(), expires: Instant::now() + Duration::from_secs(10) }) })
    }
    fn cast_audio(&self, _: &str, _: &str) -> BoxFut<'_, Option<CastAudio>> {
        Box::pin(async { Some(CastAudio { prime: b"MP3".to_vec(), audio: self.audio.subscribe() }) })
    }
}

fn binary(mime: &str, bytes: &[u8]) -> Binary {
    Binary { bytes: bytes.to_vec(), content_type: mime.into(), cache_control: "no-store", disposition: None }
}

impl ChatControl for Media {
    fn handle(&self, _: &str, _: &str, _: &str, _: Value) -> BoxFut<'_, Result<Value, GwError>> {
        Box::pin(async { Ok(json!({"ok":true})) })
    }
    fn bytes(&self, path: &str, _: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        let file = match path.rsplit('/').next().unwrap() {
            "1" => "a.html", "2" => "a.svg", "3" => "a.xhtml", "4" => "b.png",
            "5" => "a.xml", "6" => "a.js", "7" => "a.pdf", "8" => "a.unknown",
            _ => "trace.jsonl",
        };
        let mime = mime_guess::from_path(file).first_or_octet_stream().to_string();
        Box::pin(async move { Ok(binary(&mime, b"untrusted file content")) })
    }
}

fn app(auth: Arc<UiAuth>) -> Router {
    let media = Arc::new(Media { frames: tokio::sync::watch::Sender::new(Some(Arc::from(&b"\xff\xd8\xff\xd9"[..]))),
        audio: tokio::sync::broadcast::channel(2).0, alive: tokio::sync::watch::Sender::new(()) });
    rest::router_with_ui_auth(media.clone(), media, RestConfig {
        token: TOKEN.into(), rate_limit_per_minute: 10_000, version: "test".into(),
    }, auth)
}

async fn get(app: &Router, path: &str, cookie: &str) -> Response {
    app.clone().oneshot(Request::get(path).header(header::COOKIE, cookie).body(Body::empty()).unwrap()).await.unwrap()
}

fn isolated(response: &Response) {
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get(header::X_CONTENT_TYPE_OPTIONS).and_then(|h| h.to_str().ok()), Some("nosniff"));
    assert_eq!(response.headers().get(header::CONTENT_SECURITY_POLICY).and_then(|h| h.to_str().ok()), Some("default-src 'none'; sandbox"));
    assert_eq!(response.headers().get("cross-origin-resource-policy").and_then(|h| h.to_str().ok()), Some("same-origin"));
}

#[tokio::test]
async fn cao01_files_are_isolated_and_only_safe_media_can_be_inline() {
    let auth = Arc::new(UiAuth::in_memory(TOKEN));
    let grant = auth.setup(TOKEN, USER, PASSWORD, false).unwrap();
    let app = app(auth);
    let cookie = grant.cookie.split(';').next().unwrap();
    for id in 1..=8 {
        let response = get(&app, &format!("/api/file/{id}"), cookie).await;
        isolated(&response);
        if id == 4 {
            assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
            assert!(!response.headers().contains_key(header::CONTENT_DISPOSITION));
        } else {
            assert_eq!(response.headers().get(header::CONTENT_DISPOSITION).and_then(|h| h.to_str().ok()), Some("attachment"));
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/octet-stream");
        }
        assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), b"untrusted file content");
    }
    for path in ["/api/live/snapshot/c1", "/api/calltrace/c1"] {
        let response = get(&app, path, cookie).await;
        isolated(&response);
        assert!(!to_bytes(response.into_body(), 1024).await.unwrap().is_empty());
    }
    for (path, expected) in [("/api/live/view/c1", &b"--frame"[..]),
        ("/api/live/audio/out/c1?t=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &b"RIFF"[..]),
        ("/api/live/cast/c1/fake.mp3", &b"MP3"[..])] {
        let response = get(&app, path, cookie).await;
        isolated(&response);
        assert!(!response.headers().contains_key(header::CONTENT_DISPOSITION));
        let mut stream = response.into_body().into_data_stream();
        let chunk = tokio::time::timeout(Duration::from_secs(1), stream.next()).await.unwrap().unwrap().unwrap();
        assert!(chunk.starts_with(expected), "{path}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trung01_login_flood_does_not_block_valid_cookie_or_bearer_api() {
    let dir = std::env::temp_dir().join(format!("tggw-flood-{}", std::process::id()));
    let auth = Arc::new(UiAuth::open(&dir, TOKEN, None, None, false).unwrap());
    let grant = auth.setup(TOKEN, USER, PASSWORD, false).unwrap();
    let app = app(auth);
    let cookie = grant.cookie.split(';').next().unwrap();
    let ready = Arc::new(tokio::sync::Barrier::new(201));
    let mut flood = tokio::task::JoinSet::new();
    for id in 0..200 {
        let app = app.clone();
        let ready = ready.clone();
        flood.spawn(async move {
            ready.wait().await;
            app.oneshot(Request::post("/ui/api/auth/login").header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"ten":format!("unknown-{id}"),"mat_khau":"wrong-password-value"}).to_string())).unwrap()).await.unwrap().status()
        });
    }
    ready.wait().await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut worst = Duration::ZERO;
    for _ in 0..10 {
        for (name, value) in [("Cookie", cookie), ("Authorization", "Bearer token-gia-de-thu")] {
            let start = Instant::now();
            let response = app.clone().oneshot(Request::get("/api/live/calls").header(name, value).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), 200);
            worst = worst.max(start.elapsed());
        }
    }
    while let Some(result) = flood.join_next().await { assert!(matches!(result.unwrap().as_u16(), 401 | 429)); }
    std::fs::remove_dir_all(dir).unwrap();
    println!("200 parallel logins; slowest authenticated API: {worst:?}; limit: 1s");
    assert!(worst < Duration::from_secs(1), "authenticated API blocked for {worst:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trung01_slow_auth_bodies_do_not_hold_password_hash_slots() {
    let dir = std::env::temp_dir().join(format!("tggw-slow-auth-{}", std::process::id()));
    let auth = Arc::new(UiAuth::open(&dir, TOKEN, None, None, false).unwrap());
    auth.setup(TOKEN, USER, PASSWORD, false).unwrap();
    let app = app(auth);
    let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
    let mut slow_requests = tokio::task::JoinSet::new();
    for _ in 0..2 {
        let app = app.clone();
        let started = started.clone();
        slow_requests.spawn(async move {
            let body = Body::from_stream(futures_util::stream::once(async move {
                started.send(()).unwrap();
                std::future::pending::<Result<axum::body::Bytes, std::convert::Infallible>>().await
            }));
            app.oneshot(Request::post("/ui/api/auth/login").header(header::CONTENT_TYPE, "application/json")
                .body(body).unwrap()).await.unwrap()
        });
    }
    drop(started);
    for _ in 0..2 { starts.recv().await.unwrap(); }

    let login = Request::post("/ui/api/auth/login").header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"ten":USER,"mat_khau":PASSWORD}).to_string())).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(1), app.oneshot(login)).await;
    slow_requests.abort_all();
    while slow_requests.join_next().await.is_some() {}
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(response.expect("a valid login must not wait for other request bodies").unwrap().status(), 200);
}

#[tokio::test]
async fn trung03_every_session_action_requires_csrf_and_keeps_session_on_rejection() {
    for action in ["logout", "logout-all", "password"] {
        let auth = Arc::new(UiAuth::in_memory(TOKEN));
        let grant = auth.setup(TOKEN, USER, PASSWORD, false).unwrap();
        let app = app(auth);
        let cookie = grant.cookie.split(';').next().unwrap();
        let path = format!("/ui/api/auth/{action}");
        for origin in [None, Some("http://evil.example")] {
            let mut request = Request::post(&path).header(header::COOKIE, cookie).header(header::HOST, "gateway.example");
            if let Some(origin) = origin { request = request.header(header::ORIGIN, origin); }
            let response = app.clone().oneshot(request.header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"cu":PASSWORD,"moi":"new-password-value"}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), 403, "{action}");
            assert_eq!(get(&app, "/api/live/calls", cookie).await.status(), 200);
        }
        let response = app.oneshot(Request::post(&path).header(header::COOKIE, cookie)
            .header("X-Requested-With", "fetch").header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"cu":PASSWORD,"moi":"new-password-value"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), 200, "{action}");
    }
}

#[tokio::test]
async fn thap03_secure_configuration_rejects_http_origin_even_with_matching_host() {
    let dir = std::env::temp_dir().join(format!("tggw-csrf-scheme-{}", std::process::id()));
    let auth = Arc::new(UiAuth::open(&dir, TOKEN, None, None, true).unwrap());
    let grant = auth.setup(TOKEN, USER, PASSWORD, false).unwrap();
    let app = app(auth);
    let cookie = grant.cookie.split(';').next().unwrap();
    for (origin, status) in [("http://gateway.example", 403), ("https://gateway.example", 200)] {
        let response = app.clone().oneshot(Request::post("/api/live/hangup").header(header::COOKIE, cookie)
            .header(header::HOST, "gateway.example").header(header::ORIGIN, origin).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), status);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn thap05_auth_store_and_data_directory_reject_symlinks() {
    use std::{fs, os::unix::fs::{symlink, PermissionsExt}};
    let root = std::env::temp_dir().join(format!("tggw-auth-symlink-{}", std::process::id()));
    fs::create_dir_all(root.join("real")).unwrap();
    symlink(root.join("real"), root.join("link")).unwrap();
    fs::set_permissions(root.join("real"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(UiAuth::open(&root.join("link"), TOKEN, None, None, false).is_err());
    assert_eq!(fs::metadata(root.join("real")).unwrap().permissions().mode() & 0o777, 0o755);
    fs::create_dir(root.join("data")).unwrap();
    fs::write(root.join("outside"), b"private data").unwrap();
    symlink(root.join("outside"), root.join("data/ui_auth.json")).unwrap();
    assert!(UiAuth::open(&root.join("data"), TOKEN, None, None, false).is_err());
    assert!(UiAuth::reset_password(&root.join("data"), PASSWORD).is_err());
    assert_eq!(fs::read(root.join("outside")).unwrap(), b"private data");
    fs::remove_dir_all(root).unwrap();
}
