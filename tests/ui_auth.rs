use std::{fs, net::{IpAddr, Ipv4Addr, SocketAddr}, path::{Path, PathBuf}, process::{Command, Stdio}, sync::Arc, time::{Duration, Instant}};

use axum::{body::{to_bytes, Body}, http::{header, Request}, response::Response, Router};
use serde_json::{json, Value};
use telegram_gateway_rs::{rest::{self, RestConfig}, ui_auth::UiAuth};
use tower::ServiceExt;

const GATEWAY_TOKEN: &str = "token-gia-de-thu";
const USER: &str = "tai-khoan-gia";
const PASSWORD: &str = "mat-khau-gia-de-thu";

fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tggw-ui-auth-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn app(dir: &Path) -> Router {
    let auth = Arc::new(UiAuth::open(dir, GATEWAY_TOKEN, None, None, false).unwrap());
    let stub = Arc::new(rest::ChuaCo);
    rest::router_with_ui_auth(stub.clone(), stub, RestConfig {
        token: GATEWAY_TOKEN.into(), rate_limit_per_minute: 120, version: "test".into(),
    }, auth)
}

fn request(method: &str, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    for (name, value) in headers { builder = builder.header(*name, *value); }
    if body.is_some() { builder = builder.header(header::CONTENT_TYPE, "application/json"); }
    builder.body(body.map(|value| Body::from(value.to_string())).unwrap_or_else(Body::empty)).unwrap()
}

async fn send(app: &Router, method: &str, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Response {
    app.clone().oneshot(request(method, path, body, headers)).await.unwrap()
}

async fn json_response(response: Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let raw = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

fn session_cookie(response: &Response) -> String {
    response.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned()
}

async fn setup(app: &Router) -> String {
    let response = send(app, "POST", "/ui/api/auth/setup", Some(json!({
        "token": GATEWAY_TOKEN, "ten": USER, "mat_khau": PASSWORD,
    })), &[]).await;
    assert_eq!(response.status(), 200);
    session_cookie(&response)
}

async fn login(app: &Router, user: &str, password: &str, remember: bool) -> Response {
    send(app, "POST", "/ui/api/auth/login", Some(json!({
        "ten": user, "mat_khau": password, "ghi_nho": remember,
    })), &[]).await
}

async fn login_from(app: &Router, user: &str, password: &str, ip: Ipv4Addr) -> Response {
    let mut request = request("POST", "/ui/api/auth/login", Some(json!({
        "ten": user, "mat_khau": password, "ghi_nho": false,
    })), &[]);
    request.extensions_mut().insert(axum::extract::ConnectInfo(SocketAddr::new(IpAddr::V4(ip), 12345)));
    app.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn tc01_first_setup_requires_correct_gateway_token() {
    let dir = data_dir("setup-token");
    let app = app(&dir);
    for token in ["sai", ""] {
        let (status, _) = json_response(send(&app, "POST", "/ui/api/auth/setup", Some(json!({
            "token": token, "ten": USER, "mat_khau": PASSWORD,
        })), &[]).await).await;
        assert_eq!(status, 401);
        assert!(!dir.join("ui_auth.json").exists());
    }
    assert!(!setup(&app).await.is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc02_setup_cannot_replace_an_existing_account() {
    let dir = data_dir("setup-once");
    let app = app(&dir);
    setup(&app).await;
    let response = send(&app, "POST", "/ui/api/auth/setup", Some(json!({
        "token": GATEWAY_TOKEN, "ten": "nguoi-dung-khac", "mat_khau": "mat-khau-khac-gia",
    })), &[]).await;
    assert_eq!(response.status(), 409);
    let cookie = session_cookie(&login(&app, USER, PASSWORD, true).await);
    let (_, status) = json_response(send(&app, "GET", "/ui/api/auth/status", None, &[("Cookie", &cookie)]).await).await;
    assert_eq!(status["nguoi_dung"], USER);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc03_login_sets_required_cookie_flags_and_remember_duration() {
    let dir = data_dir("cookie");
    let app = app(&dir);
    setup(&app).await;
    let remembered = login(&app, USER, PASSWORD, true).await;
    assert_eq!(remembered.status(), 200);
    let header = remembered.headers()[header::SET_COOKIE].to_str().unwrap();
    for flag in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=7776000"] { assert!(header.contains(flag), "{flag}"); }
    let temporary = login(&app, USER, PASSWORD, false).await;
    assert_eq!(temporary.status(), 200);
    let header = temporary.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(header.contains("HttpOnly") && header.contains("SameSite=Lax") && header.contains("Path=/"));
    assert!(!header.contains("Max-Age"));
    let secure = send(&app, "POST", "/ui/api/auth/login", Some(json!({
        "ten": USER, "mat_khau": PASSWORD, "ghi_nho": true,
    })), &[("Origin", "https://gateway.example")]).await;
    assert!(secure.headers()[header::SET_COOKIE].to_str().unwrap().contains("Secure"));
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc04_unknown_user_and_wrong_password_have_identical_generic_401() {
    let dir = data_dir("generic-login-error");
    let app = app(&dir);
    setup(&app).await;
    let mut answers = Vec::new();
    for user in [USER, "khong-ton-tai"] {
        let response = login(&app, user, "mat-khau-sai-gia", false).await;
        let (status, body) = json_response(response).await;
        answers.push((status, body["error"]["message"].clone()));
    }
    assert_eq!(answers[0], (401, json!("Tên hoặc mật khẩu không đúng.")));
    assert_eq!(answers[1], answers[0]);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc04_wrong_password_and_unknown_user_have_similar_median_timing() {
    let dir = data_dir("generic-login-timing");
    let app = app(&dir);
    setup(&app).await;
    let mut known = Vec::new();
    let mut unknown = Vec::new();
    for attempt in 1..=20 {
        let ip = |host| Ipv4Addr::new(192, 0, 2, host);
        let started = Instant::now();
        assert_eq!(login_from(&app, USER, "mat-khau-sai-gia", ip(attempt)).await.status(), 401);
        known.push(started.elapsed().as_nanos());
        let started = Instant::now();
        assert_eq!(login_from(&app, "khong-ton-tai", "mat-khau-sai-gia", ip(attempt + 20)).await.status(), 401);
        unknown.push(started.elapsed().as_nanos());
    }
    known.sort_unstable();
    unknown.sort_unstable();
    let known_median = known[known.len() / 2];
    let unknown_median = unknown[unknown.len() / 2];
    let difference = known_median.abs_diff(unknown_median);
    assert!(difference * 100 <= known_median.max(unknown_median) * 25,
        "known median {known_median}ns; unknown median {unknown_median}ns");
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc05_sixth_wrong_login_is_rate_limited() {
    let dir = data_dir("login-limit");
    let app = app(&dir);
    setup(&app).await;
    for _ in 0..5 { assert_eq!(login(&app, USER, "mat-khau-sai-gia", false).await.status(), 401); }
    let limited = login(&app, USER, "mat-khau-sai-gia", false).await;
    assert_eq!(limited.status(), 429);
    assert!(limited.headers()[header::RETRY_AFTER].to_str().unwrap().parse::<u64>().unwrap() > 0);
    let (_, body) = json_response(limited).await;
    assert!(body["error"]["retry_after_seconds"].as_u64().unwrap() > 0);
    fs::remove_dir_all(dir).unwrap();
}

// QA thêm: song song không được vượt giới hạn của cùng một (IP, tên).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn qa_tc05_parallel_wrong_logins_cannot_bypass_the_five_attempt_limit() {
    let dir = data_dir("parallel-login-limit");
    let app = app(&dir);
    setup(&app).await;
    let ready = Arc::new(tokio::sync::Barrier::new(6));
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let app = app.clone();
        let ready = ready.clone();
        requests.spawn(async move {
            ready.wait().await;
            let response = login(&app, USER, "mat-khau-sai-gia", false).await;
            if response.status() == 429 {
                assert!(response.headers()[header::RETRY_AFTER].to_str().unwrap().parse::<u64>().unwrap() > 0);
            }
            response.status().as_u16()
        });
    }
    let mut statuses = Vec::new();
    while let Some(result) = requests.join_next().await { statuses.push(result.unwrap()); }
    statuses.sort_unstable();
    assert_eq!(statuses, vec![401, 401, 401, 401, 401, 429]);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc06_cookie_authenticates_existing_api_and_no_auth_is_401() {
    let dir = data_dir("cookie-api");
    let app = app(&dir);
    setup(&app).await;
    let cookie = session_cookie(&login(&app, USER, PASSWORD, true).await);
    for path in ["/api/accounts", "/api/live/calls", "/api/events", "/api/live/snapshot/c1", "/api/calltrace/c1"] {
        let response = send(&app, "GET", path, None, &[("Cookie", &cookie)]).await;
        let bearer = send(&app, "GET", path, None, &[("Authorization", "Bearer token-gia-de-thu")]).await;
        assert_eq!(response.status(), bearer.status(), "{path}");
    }
    assert_eq!(send(&app, "GET", "/api/live/calls", None, &[]).await.status(), 401);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc07_cookie_writes_require_csrf_header_or_matching_origin() {
    let dir = data_dir("csrf");
    let app = app(&dir);
    setup(&app).await;
    let cookie = session_cookie(&login(&app, USER, PASSWORD, true).await);
    assert_eq!(send(&app, "POST", "/api/live/hangup", Some(json!({})), &[("Cookie", &cookie)]).await.status(), 403);
    assert_eq!(send(&app, "POST", "/api/live/hangup", Some(json!({})), &[
        ("Cookie", &cookie), ("Host", "gateway.example"), ("Origin", "https://evil.example"),
    ]).await.status(), 403);
    let requested = send(&app, "POST", "/api/live/hangup", Some(json!({})), &[
        ("Cookie", &cookie), ("X-Requested-With", "fetch"),
    ]).await;
    let matching_origin = send(&app, "POST", "/api/live/hangup", Some(json!({})), &[
        ("Cookie", &cookie), ("Host", "gateway.example"), ("Origin", "http://gateway.example"),
    ]).await;
    assert_eq!(requested.status(), 501);
    assert_eq!(matching_origin.status(), 501);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc08_existing_bearer_authentication_remains_available() {
    let dir = data_dir("bearer");
    let app = app(&dir);
    setup(&app).await;
    let response = send(&app, "GET", "/api/live/calls", None,
        &[("Authorization", "Bearer token-gia-de-thu")]).await;
    assert_eq!(response.status(), 200);
    assert_eq!(send(&app, "GET", "/health", None, &[]).await.status(), 200);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc09_logout_and_logout_all_revoke_sessions() {
    let dir = data_dir("logout");
    let app = app(&dir);
    setup(&app).await;
    let cookie_a = session_cookie(&login(&app, USER, PASSWORD, true).await);
    let cookie_b = session_cookie(&login(&app, USER, PASSWORD, true).await);
    let cookie_c = session_cookie(&login(&app, USER, PASSWORD, true).await);
    assert_eq!(send(&app, "POST", "/ui/api/auth/logout", None, &[
        ("Cookie", &cookie_a), ("X-Requested-With", "fetch"),
    ]).await.status(), 200);
    assert_eq!(send(&app, "GET", "/api/live/calls", None, &[("Cookie", &cookie_a)]).await.status(), 401);
    let revoked = send(&app, "POST", "/ui/api/auth/logout-all", None, &[
        ("Cookie", &cookie_b), ("X-Requested-With", "fetch"),
    ]).await;
    assert_eq!(revoked.status(), 200);
    assert!(revoked.headers()[header::SET_COOKIE].to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(send(&app, "GET", "/api/live/calls", None, &[("Cookie", &cookie_b)]).await.status(), 401);
    assert_eq!(send(&app, "GET", "/api/live/calls", None, &[("Cookie", &cookie_c)]).await.status(), 401);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc10_password_change_checks_policy_and_revokes_other_sessions() {
    let dir = data_dir("password");
    let app = app(&dir);
    setup(&app).await;
    let current = session_cookie(&login(&app, USER, PASSWORD, true).await);
    let other = session_cookie(&login(&app, USER, PASSWORD, true).await);
    for (old, new, status) in [("sai", "mat-khau-moi-gia", 401), (PASSWORD, "ngan", 400),
        (PASSWORD, USER, 400)] {
        assert_eq!(send(&app, "POST", "/ui/api/auth/password", Some(json!({"cu":old,"moi":new})), &[
            ("Cookie", &current), ("X-Requested-With", "fetch"),
        ]).await.status(), status);
    }
    assert_eq!(send(&app, "POST", "/ui/api/auth/password", Some(json!({
        "cu":PASSWORD,"moi":"mat-khau-moi-gia",
    })), &[("Cookie", &current), ("X-Requested-With", "fetch")]).await.status(), 200);
    assert_eq!(send(&app, "GET", "/api/live/calls", None, &[("Cookie", &other)]).await.status(), 401);
    assert_eq!(send(&app, "GET", "/api/live/calls", None, &[("Cookie", &current)]).await.status(), 200);
    assert_eq!(login(&app, USER, PASSWORD, false).await.status(), 401);
    assert_eq!(login(&app, USER, "mat-khau-moi-gia", false).await.status(), 200);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc11_credentials_and_raw_session_tokens_are_not_persisted() {
    use std::os::unix::fs::PermissionsExt;
    let dir = data_dir("permissions");
    let app = app(&dir);
    let cookie = setup(&app).await;
    assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
    let file = dir.join("ui_auth.json");
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    let saved = fs::read_to_string(file).unwrap();
    assert!(!saved.contains(PASSWORD));
    assert!(!saved.contains(GATEWAY_TOKEN));
    assert!(!saved.contains(cookie.split_once('=').unwrap().1));
    let stored: Value = serde_json::from_str(&saved).unwrap();
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let salt = URL_SAFE_NO_PAD.decode(stored["account"]["salt"].as_str().unwrap()).unwrap();
    let password_hash = URL_SAFE_NO_PAD.decode(stored["account"]["password_hash"].as_str().unwrap()).unwrap();
    assert!(salt.len() >= 16);
    assert_eq!(password_hash.len(), 32);
    ring::pbkdf2::verify(ring::pbkdf2::PBKDF2_HMAC_SHA256, std::num::NonZeroU32::new(210_000).unwrap(),
        &salt, PASSWORD.as_bytes(), &password_hash).unwrap();
    assert!(URL_SAFE_NO_PAD.decode(cookie.split_once('=').unwrap().1).unwrap().len() >= 32);
    let session_hash = ring::digest::digest(&ring::digest::SHA256, cookie.split_once('=').unwrap().1.as_bytes());
    let session_hash: String = session_hash.as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    assert!(stored["sessions"].as_array().unwrap().iter().any(|item| item["hash"] == session_hash));
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc06_tc11_serve_accepts_cookie_on_account_and_event_routes_without_logging_secrets() {
    let dir = data_dir("serve-auth");
    // The logger deliberately masks URLs. Pick our own loopback port instead of parsing its output.
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let base = format!("http://127.0.0.1:{port}");
    let mut server = tokio::process::Command::new(env!("CARGO_BIN_EXE_tggw"))
        .arg("serve").env_clear().env("TGGW_DATA", &dir)
        .env("TG_API_ID", "12345").env("TG_API_HASH", "hash-gia-de-thu")
        .env("GATEWAY_HOST", "127.0.0.1").env("REST_PORT", port.to_string()).env("GATEWAY_TOKEN", GATEWAY_TOKEN)
        .stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    use tokio::io::{AsyncBufReadExt as _, BufReader};
    let mut stderr = BufReader::new(server.stderr.take().unwrap());
    let mut first = String::new();
    tokio::time::timeout(Duration::from_secs(10), stderr.read_line(&mut first)).await.unwrap().unwrap();
    assert!(first.contains("tggw serve: nghe "), "gateway did not start");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).no_proxy().build().unwrap();
    let payload = json!({"token":GATEWAY_TOKEN,"ten":USER,"mat_khau":PASSWORD}).to_string();
    let setup = client.post(format!("{base}/ui/api/auth/setup"))
        .header(header::CONTENT_TYPE, "application/json").body(payload).send().await.unwrap();
    assert_eq!(setup.status(), 200);
    let cookie = setup.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();
    for path in ["/api/accounts", "/api/live/calls", "/api/events"] {
        let request = client.get(format!("{base}{path}"));
        let response = request.try_clone().unwrap().header(header::COOKIE, &cookie).send().await.unwrap();
        let bearer = request.try_clone().unwrap().bearer_auth(GATEWAY_TOKEN).send().await.unwrap();
        assert_eq!(bearer.status(), response.status(), "{path}");
        assert_eq!(bearer.bytes().await.unwrap(), response.bytes().await.unwrap(), "{path}");
        assert_eq!(request.send().await.unwrap().status(), 401, "{path}");
    }
    // Exercise both successful and rejected login/password paths before reading all log sinks.
    let logged_in = client.post(format!("{base}/ui/api/auth/login")).header(header::CONTENT_TYPE, "application/json")
        .body(json!({"ten":USER,"mat_khau":PASSWORD}).to_string()).send().await.unwrap();
    assert_eq!(logged_in.status(), 200);
    let login_cookie = logged_in.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();
    for (old, new, expected) in [("wrong-old-password-value", "unused-new-password-value", 401),
        (PASSWORD, "changed-password-value", 200)] {
        let response = client.post(format!("{base}/ui/api/auth/password")).header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, &login_cookie).header("X-Requested-With", "fetch")
            .body(json!({"cu":old,"moi":new}).to_string()).send().await.unwrap();
        assert_eq!(response.status(), expected);
    }
    let changed_login = client.post(format!("{base}/ui/api/auth/login")).header(header::CONTENT_TYPE, "application/json")
        .body(json!({"ten":USER,"mat_khau":"changed-password-value"}).to_string()).send().await.unwrap();
    assert_eq!(changed_login.status(), 200);
    let changed_cookie = changed_login.headers()[header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();
    // The connection address defines the limit; spoofed forwarding headers cannot bypass it.
    for host in 1..=6 {
        let response = client.post(format!("{base}/ui/api/auth/login"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("X-Forwarded-For", format!("192.0.2.{host}"))
            .body(json!({"ten":USER,"mat_khau":"mat-khau-sai-gia"}).to_string())
            .send().await.unwrap();
        assert_eq!(response.status(), if host == 6 { 429 } else { 401 });
    }
    server.start_kill().unwrap();
    let mut rest = String::new();
    use tokio::io::AsyncReadExt as _;
    stderr.read_to_string(&mut rest).await.unwrap();
    let output = server.wait_with_output().await.unwrap();
    let mut logs = format!("{first}{rest}{}", String::from_utf8_lossy(&output.stdout));
    for entry in fs::read_dir(dir.join("calltrace")).into_iter().flatten().flatten() {
        if entry.path().is_file() { logs.push_str(&fs::read_to_string(entry.path()).unwrap()); }
    }
    for secret in [GATEWAY_TOKEN, PASSWORD, "mat-khau-sai-gia", "wrong-old-password-value", "unused-new-password-value",
        "changed-password-value", cookie.split_once('=').unwrap().1, login_cookie.split_once('=').unwrap().1,
        changed_cookie.split_once('=').unwrap().1] {
        assert!(!logs.contains(secret), "gateway logs contained an authentication secret");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc12_remembered_session_survives_server_auth_state_reload() {
    let dir = data_dir("restart");
    let first = app(&dir);
    setup(&first).await;
    let cookie = session_cookie(&login(&first, USER, PASSWORD, true).await);
    drop(first);
    let restarted = app(&dir);
    assert_ne!(send(&restarted, "GET", "/api/live/calls", None, &[("Cookie", &cookie)]).await.status(), 401);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc13_pages_use_cookie_fetch_and_do_not_store_auth_secrets() {
    for html in [include_str!("../web/index.html"), include_str!("../web/app.html")] {
        assert!(html.contains("credentials: \"same-origin\""));
        assert!(!html.contains("sessionStorage"));
        assert!(!html.contains("localStorage"));
        assert!(!html.contains("innerHTML"));
        assert!(!html.contains("gw_token"));
        assert!(!html.contains("GATEWAY_TOKEN"));
    }
}

#[tokio::test]
async fn tc14_passwd_command_changes_password_and_revokes_all_sessions() {
    let dir = data_dir("passwd-command");
    let initial = app(&dir);
    setup(&initial).await;
    let old_cookie = session_cookie(&login(&initial, USER, PASSWORD, true).await);
    let other_cookie = session_cookie(&login(&initial, USER, PASSWORD, true).await);
    for _ in 0..5 { assert_eq!(login(&initial, USER, "wrong-password-value", false).await.status(), 401); }
    assert_eq!(login(&initial, USER, PASSWORD, false).await.status(), 429);
    let output = Command::new(env!("CARGO_BIN_EXE_tggw"))
        .arg("passwd").env("TGGW_DATA", &dir).env("TG_UI_USER", USER)
        .env("TG_UI_PASSWORD", "mat-khau-sau-reset").env_remove("GATEWAY_TOKEN")
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    // Reset must unlock the very first login, before status/API reads reload the account.
    assert_eq!(login(&initial, USER, "mat-khau-sau-reset", false).await.status(), 200);
    assert_eq!(send(&initial, "GET", "/api/live/calls", None, &[("Cookie", &old_cookie)]).await.status(), 401);
    assert_eq!(send(&initial, "GET", "/api/live/calls", None, &[("Cookie", &other_cookie)]).await.status(), 401);
    assert_eq!(login(&initial, USER, PASSWORD, false).await.status(), 401);
    assert_eq!(login(&initial, USER, "mat-khau-sau-reset", false).await.status(), 200);
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn tc14_passwd_reads_stdin_without_echoing_the_password() {
    use std::io::Write;
    let dir = data_dir("passwd-stdin");
    let initial = app(&dir);
    let old_cookie = setup(&initial).await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_tggw"))
        .arg("passwd").env_clear().env("TGGW_DATA", &dir)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"mat-khau-tu-stdin\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("mat-khau-tu-stdin"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("mat-khau-tu-stdin"));
    assert_eq!(send(&initial, "GET", "/api/live/calls", None, &[("Cookie", &old_cookie)]).await.status(), 401);
    assert_eq!(login(&initial, USER, "mat-khau-tu-stdin", false).await.status(), 200);
    fs::remove_dir_all(dir).unwrap();
}
