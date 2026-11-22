use super::*;
use axum::{body::{to_bytes,Body},http::Request};
use tower::ServiceExt;

pub(crate) struct Fixture {
    pub hub:Arc<Accounts>,pub api:Arc<crate::td::fake::Fake>,pub router:axum::Router,pub dir:PathBuf,
}
impl Fixture {
    pub async fn new(name:&str)->Self {
        let dir=std::env::temp_dir().join(format!("tggw-http-{}-{name}",std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (api,factory)=crate::td::fake::Fake::factory();
        let config=LoginConfig{data_dir:dir.clone(),api_id:12345,api_hash:"fake-api-hash".into(),phone:String::new()};
        let hub=Accounts::with_factory(config,"fake".into(),3399,factory).await.unwrap();
        let router=rest_router(hub.clone());
        Self{hub,api,router,dir}
    }
    pub async fn send(&self,method:&str,path:&str,body:Value)->(u16,Value) {
        send(&self.router,method,path,body,true).await
    }
    pub async fn login(&self,phone:&str)->String {
        let (status,value)=self.send("POST","/ui/api/accounts",json!({"phone":phone})).await;
        assert_eq!(status,200,"{value}");value["login_id"].as_str().unwrap().into()
    }
    pub async fn ready(&self,phone:&str) {
        let id=self.login(phone).await;
        assert_eq!(self.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":"22222"})).await,
            (200,json!({"ok":true,"phone":phone,"status":"online","mock":false})));
    }
    pub async fn finish(self) { self.hub.shutdown().await; drop(self.router);drop(self.hub);std::fs::remove_dir_all(self.dir).unwrap(); }
}
fn rest_router(hub:Arc<Accounts>)->axum::Router {
    crate::rest::router_accounts(hub,crate::rest::RestConfig{token:"test-token".into(),rate_limit_per_minute:1000,version:"test".into()})
}
pub(crate) async fn send(router:&axum::Router,method:&str,path:&str,body:Value,auth:bool)->(u16,Value) {
    let mut request=Request::builder().method(method).uri(path);
    if auth {request=request.header("Authorization","Bearer test-token");}
    let response=router.clone().oneshot(request.body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status=response.status().as_u16();
    let bytes=to_bytes(response.into_body(),usize::MAX).await.unwrap();
    (status,serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn two_accounts_route_messages_calls_feed_and_events() {
    let f=Fixture::new("route").await;
    f.ready("+84900000001").await;f.ready("+84900000002").await;
    assert_eq!(f.send("GET","/api/me?account=%2B84900000002",Value::Null).await.1["id"],2);
    assert_eq!(f.send("GET","/api/me",Value::Null).await.1["id"],1);
    assert_eq!(f.send("GET","/api/me?account=unknown",Value::Null).await.0,404);
    assert_eq!(f.send("GET","/api/feed?wait=bad",Value::Null).await.0,400);
    assert_eq!(f.send("GET","/api/feed?since=bad",Value::Null).await.0,400);
    assert_eq!(f.send("GET","/health",Value::Null).await.1["accounts_online"],2);
    for id in 1..=2 {
        f.api.push(json!({"@type":"updateNewMessage","message":{"id":id*100,"chat_id":id*10,"content":{"@type":"messageText","text":{"text":"fake"}}}}),id);
    }
    let deadline=tokio::time::Instant::now()+Duration::from_secs(3);
    loop {
        let feed=f.send("GET","/api/feed?wait=0.1",Value::Null).await.1;
        if feed["items"].as_array().unwrap().len()==2 {
            let items=feed["items"].as_array().unwrap();
            assert_ne!(items[0]["account"],items[1]["account"]);break;
        }
        assert!(tokio::time::Instant::now()<deadline);
    }
    loop {
        let events=f.send("GET","/api/events",Value::Null).await.1;
        if events["events"].as_array().unwrap().len()>=2 {
            let events=events["events"].as_array().unwrap();
            assert_ne!(events[0]["account"],events[1]["account"]);break;
        }
        assert!(tokio::time::Instant::now()<deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Trùng mã cuộc gọi TDLib giữa hai tài khoản vẫn không lẫn điều khiển.
    for id in 1..=2 {
        f.api.push(json!({"@type":"updateCall","call":{"id":71,"user_id":id+100,"is_outgoing":false,"state":{"@type":"callStatePending","is_created":true,"is_received":true}}}),id);
    }
    loop {
        let calls=f.send("GET","/api/live/calls",Value::Null).await.1;
        if calls["calls"].as_array().unwrap().len()==2 {
            let calls=calls["calls"].as_array().unwrap();
            assert_ne!(calls[0]["id"],calls[1]["id"]);
            let own=f.send("GET","/api/live/calls?account=%2B84900000002",Value::Null).await.1;
            assert_eq!(own["calls"].as_array().unwrap().len(),1);
            assert_eq!(own["calls"][0]["id"],calls[1]["id"]);
            let snapshot=format!("/api/live/snapshot/{}",calls[1]["id"].as_str().unwrap());
            assert_eq!(f.send("GET",&snapshot,Value::Null).await.0,409);
            assert_eq!(f.send("GET",&format!("{snapshot}?account=%2B84900000001"),Value::Null).await.0,404);
            let id=calls[1]["id"].clone();
            let result=f.send("POST","/api/live/hangup",json!({"id":id,"account":"+84900000001"})).await;
            assert_eq!(result.0,404);
            assert_eq!(f.send("POST","/api/live/hangup",json!({"id":id})).await.0,200);
            break;
        }
        assert!(tokio::time::Instant::now()<deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f.finish().await;
}

#[tokio::test]
async fn storage_settings_validate_before_replacing_the_account_file() {
    use std::os::unix::fs::PermissionsExt;

    let f = Fixture::new("storage-settings").await;
    // TDLib giả nhận định danh thử nghiệm, không cần số điện thoại có thể gọi được.
    let first = "test-storage-first";
    let second = "test-storage-second";
    f.ready(first).await;
    f.ready(second).await;
    assert_eq!(f.send("GET", "/api/settings/storage", Value::Null).await.0, 400);
    let endpoint = "/api/settings/storage?account=test-storage-first";
    let (status, mut settings) = f.send("GET", endpoint, Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(settings["mode"], "local");
    assert_eq!(settings["keep"], json!({"private":604800,"groups":604800,"channels":604800}));
    assert_eq!(settings["max_cache_bytes"], 2 * 1024 * 1024 * 1024_u64);
    assert_eq!(settings["stream_ttl_s"], 600);

    let first_dir = f.hub.store.lock().unwrap().session_dir(first);
    let second_dir = f.hub.store.lock().unwrap().session_dir(second);
    let file = first_dir.join("storage_settings.json");
    settings["mode"] = json!("stream");
    settings["exceptions"] = json!([{"chat_id":-100123,"keep":null}]);
    let previous = std::fs::read(&file).unwrap();
    for (field, bad) in [
        ("mode", json!("web")),
        ("keep", json!({"private":99,"groups":604800,"channels":null})),
        ("max_cache_bytes", json!(-1)),
        ("stream_ttl_s", json!(0)),
    ] {
        let mut invalid = settings.clone();
        invalid[field] = bad;
        let (status, error) = f.send("POST", endpoint, invalid).await;
        assert_eq!(status, 422, "{field}: {error}");
        assert!(error["error"]["code"].is_string());
        assert_eq!(std::fs::read(&file).unwrap(), previous);
    }
    assert_eq!(f.send("POST", endpoint, settings.clone()).await.0, 400);
    assert_eq!(std::fs::read(&file).unwrap(), previous);
    let mut confirmed = settings.clone();
    confirmed["confirm"] = json!(true);
    assert_eq!(f.send("POST", endpoint, confirmed).await, (200, settings.clone()));
    assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(&first_dir).unwrap().permissions().mode() & 0o777, 0o700);
    assert_ne!(std::fs::read(&file).unwrap(), std::fs::read(second_dir.join("storage_settings.json")).unwrap());
    // Tệp tạm viết dở do tiến trình bị ngắt phải không che mất cấu hình đã commit.
    std::fs::write(first_dir.join("storage_settings.interrupted.tmp"), b"{\"mode\":").unwrap();
    assert_eq!(crate::storage_settings::load(&first_dir).unwrap(), settings);
    assert_eq!(send(&f.router, "GET", endpoint, Value::Null, false).await.0, 401);
    f.finish().await;
}

#[test]
fn storage_upgrade_preserves_all_media_without_a_cache_limit() {
    let dir = std::env::temp_dir().join(format!("tggw-storage-upgrade-{}", std::process::id()));
    let files = dir.join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("td.binlog"), b"old account database").unwrap();
    let settings = crate::storage_settings::load(&dir).unwrap();
    assert_eq!(settings["keep"], json!({"private":null,"groups":null,"channels":null}));
    assert_eq!(settings["max_cache_bytes"], Value::Null);
    assert_eq!(std::fs::read(files.join("td.binlog")).unwrap(), b"old account database");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn storage_usage_aggregates_all_types_and_limits_top_chats_without_leaking_details() {
    let f = Fixture::new("storage-usage").await;
    f.ready("test-storage-usage").await;
    let chats = (0..=25).map(|id| json!({
        "chat_id":id, "size":id + 1, "count":1, "title":"private message content",
        "by_file_type":[{"file_type":{"@type":if id % 2 == 0 {"fileTypePhoto"} else {"fileTypeVideo"}},
            "size":id + 1,"count":1,"path":"/private/file.png"}]
    })).collect::<Vec<_>>();
    *f.api.storage_stats.lock().unwrap() = json!({"@type":"storageStatistics","size":351,"count":26,"by_chat":chats});
    let (status, usage) = f.send("GET", "/api/storage/usage", Value::Null).await;
    assert_eq!(status, 200, "{usage}");
    assert_eq!(usage["total_bytes"], 351);
    assert_eq!(usage["by_type"], json!({"photo":169,"video":182,"file":0,"music":0,"other":0}));
    assert_eq!(usage["top_chats"].as_array().unwrap().len(), 20);
    assert_eq!(usage["top_chats"][0], json!({"chat_id":25,"bytes":26}));
    assert!(!usage.to_string().contains("/private") && !usage.to_string().contains("message content"));
    f.finish().await;
}

#[tokio::test]
async fn storage_clear_requires_confirmation_and_targets_only_the_selected_chat() {
    let f = Fixture::new("storage-clear").await;
    f.ready("test-storage-clear").await;
    *f.api.storage_stats.lock().unwrap() = json!({"@type":"storageStatistics","size":26,"count":2,"by_chat":[
        {"chat_id":10,"size":12,"count":1,"by_file_type":[]},
        {"chat_id":11,"size":14,"count":1,"by_file_type":[]}
    ]});
    let endpoint = "/api/storage/clear";
    assert_eq!(f.send("POST", endpoint, json!({"scope":"chat","chat_id":10})).await.0, 400);
    assert!(!f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage"));
    let (status, cleared) = f.send("POST", endpoint, json!({"scope":"chat","chat_id":10,"confirm":true})).await;
    assert_eq!(status, 200, "{cleared}");
    let clear = f.api.sent.lock().unwrap().iter().find(|(_, req)| req["@type"] == "optimizeStorage").unwrap().1.clone();
    assert_eq!(clear["chat_ids"], json!([10]));
    assert_eq!(clear["size"], 0);
    let other = f.dir.join("outside-cache");
    std::fs::create_dir(&other).unwrap();
    let sentinel = other.join("keep.txt");
    std::fs::write(&sentinel, b"outside cache").unwrap();
    let link = f.hub.store.lock().unwrap().session_dir("test-storage-clear").join("files/photos");
    std::os::unix::fs::symlink(&other, &link).unwrap();
    assert_eq!(f.send("POST", endpoint, json!({"scope":"all","confirm":true})).await.1["error"]["code"], "STORAGE_CACHE_PATH");
    assert_eq!(std::fs::read(sentinel).unwrap(), b"outside cache");
    std::fs::remove_file(link).unwrap();
    f.api.push(json!({"@type":"updateCall","call":{"id":71,"user_id":101,"is_outgoing":false,
        "state":{"@type":"callStatePending","is_created":true,"is_received":true}}}),1);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let (_, calls) = f.send("GET", "/api/live/calls", Value::Null).await;
        if calls["calls"].as_array().is_some_and(|list| !list.is_empty()) { break; }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(f.send("POST", endpoint, json!({"scope":"all","confirm":true})).await.0, 409);
    assert_eq!(f.api.sent.lock().unwrap().iter().filter(|(_, req)| req["@type"] == "optimizeStorage").count(), 1);
    f.finish().await;
}

#[tokio::test]
async fn local_janitor_sweeps_timed_chats_and_separate_exceptions_with_a_budget() {
    let f = Fixture::new("storage-janitor").await;
    let phone = "test-storage-janitor";
    f.ready(phone).await;
    let (_, entry) = f.hub.select(phone).unwrap();
    let mut policy = crate::storage_settings::load(&entry.session).unwrap();
    policy["keep"] = json!({"private":86400,"groups":604800,"channels":null});
    policy["max_cache_bytes"] = Value::Null;
    policy["exceptions"] = json!([{"chat_id":42,"keep":2678400},{"chat_id":43,"keep":null}]);
    let too_large = 5 * 1024 * 1024 * 1024_u64 + 1;
    let chats = [
        (40, 7, json!({"@type":"chatTypePrivate"})),
        (41, 7, json!({"@type":"chatTypeBasicGroup"})),
        (42, 7, json!({"@type":"chatTypePrivate"})),
        (43, 7, json!({"@type":"chatTypePrivate"})),
        (44, 7, json!({"@type":"chatTypeSupergroup","is_channel":true})),
        (45, too_large, json!({"@type":"chatTypePrivate"})),
    ];
    *f.api.storage_chats.lock().unwrap() = chats.iter().map(|(id, _, kind)| (*id, kind.clone())).collect();
    let stats = chats.iter().map(|(id, bytes, _)| json!({"chat_id":id,"size":bytes,"count":1})).collect::<Vec<_>>();
    *f.api.storage_stats.lock().unwrap() = json!({"@type":"storageStatistics","size":too_large + 35,"count":6,"by_chat":stats});
    *f.api.optimize_reply.lock().unwrap() = Some(Box::new(move |req| {
        let bytes: u64 = req["chat_ids"].as_array().unwrap().iter().filter_map(Value::as_i64)
            .map(|id| if id == 45 { too_large } else { 7 }).sum();
        let count = req["chat_ids"].as_array().unwrap().len();
        json!({"@type":"storageStatistics","size":bytes,"count":count,"by_chat":[]})
    }));
    let mut report = crate::storage::Report::new();
    crate::storage::run(entry.client.as_ref(), &policy, &mut report).await.unwrap();
    let requests = f.api.sent.lock().unwrap().iter().filter(|(_, req)| req["@type"] == "optimizeStorage")
        .map(|(_, req)| json!([req["chat_ids"],req["exclude_chat_ids"],req["ttl"]])).collect::<Vec<_>>();
    assert_eq!(requests, vec![json!([[40],[42,43],86400]),json!([[45],[42,43],86400])]);
    assert_eq!(report.json()["limited"], true);
    assert_eq!(report.json()["freed_bytes"], too_large + 7);
    assert_eq!(report.json()["remaining_bytes"], 14);

    // Lượt sau tiếp tục các nhóm còn lại khi không còn chat 40 và 45.
    f.api.sent.lock().unwrap().clear();
    let remaining_stats = chats[1..5].iter().map(|(id, bytes, _)| json!({"chat_id":id,"size":bytes,"count":1})).collect::<Vec<_>>();
    *f.api.storage_stats.lock().unwrap() = json!({"@type":"storageStatistics","size":28,"count":4,"by_chat":remaining_stats});
    let mut next_report = crate::storage::Report::new();
    crate::storage::run(entry.client.as_ref(), &policy, &mut next_report).await.unwrap();
    let next_requests = f.api.sent.lock().unwrap().iter().filter(|(_, req)| req["@type"] == "optimizeStorage")
        .map(|(_, req)| json!([req["chat_ids"],req["exclude_chat_ids"],req["ttl"]])).collect::<Vec<_>>();
    assert_eq!(next_requests, vec![json!([[41],[42,43],604800]),json!([[42],[],2678400])]);
    assert_eq!(next_report.json()["limited"], false);
    f.finish().await;
}

#[tokio::test]
async fn wrong_codes_lock_and_server_flood_wait_keeps_retry_seconds() {
    let f=Fixture::new("flood-contract").await;
    let id=f.login("+84900000003").await;
    let path=format!("/ui/api/accounts/{id}/code");
    for code in ["bad","11111"] {
        let result=f.send("POST",&path,json!({"code":code})).await;
        assert_eq!(result.0,400);
        assert_eq!(result.1["error"]["code"],"BAD_REQUEST");
        assert!(!result.1.to_string().contains(code));
    }
    for _ in 0..2 {
        let result=f.send("POST",&path,json!({"code":"22222".replace('2',"1")})).await;
        assert_eq!(result.0,429);
        assert_eq!(result.1["error"]["code"],"LOGIN_LOCKED");
    }
    assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.0,429);
    assert_eq!(f.send("POST","/ui/api/accounts",json!({"phone":"+84900000003"})).await.0,429);
    let id=f.login("+84900000004").await;
    let result=f.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":"33333"})).await;
    assert_eq!(result,(429,json!({"error":{"code":"FLOOD_WAIT","message":"Telegram yêu cầu chờ trước khi thử lại.","retry_after_seconds":123}})));
    let result=f.send("POST","/ui/api/accounts",json!({"phone":"+84900000300"})).await;
    assert_eq!(result.0,429);
    assert_eq!(result.1["error"]["retry_after_seconds"],321);
    assert_eq!(f.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":"22222"})).await.0,200);
    f.finish().await;
}

#[tokio::test]
async fn qr_confirmation_and_password_status_follow_latest_authorization() {
    let f=Fixture::new("qr-confirm").await;
    let result=f.send("POST","/ui/api/accounts/qr",json!({"phone":"+84900000005"})).await;
    let id=result.1["login_id"].as_str().unwrap();
    let path=format!("/ui/api/accounts/{id}");
    assert_eq!(f.send("POST",&format!("{path}/qr-confirm"),json!({})).await.0,400);
    f.api.auth(1,json!({"@type":"authorizationStateWaitOtherDeviceConfirmation","link":"tg://login?token=rotated-fake"}));
    tokio::time::sleep(Duration::from_millis(20)).await;
    let status=f.send("GET",&path,Value::Null).await.1;
    assert_eq!(status["qr_link"],"tg://login?token=rotated-fake");
    assert_ne!(status["phone"],"+84900000005");
    f.api.auth(1,json!({"@type":"authorizationStateWaitPassword"}));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(f.send("GET",&path,Value::Null).await.1["stage"],"password");
    assert_eq!(f.send("POST",&format!("{path}/password"),json!({"password":" secret2fa "})).await.0,200);
    let result=f.send("POST","/ui/api/accounts/qr",json!({"phone":"+84900000006"})).await;
    let id=result.1["login_id"].as_str().unwrap();
    f.api.auth(2,json!({"@type":"authorizationStateReady"}));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(f.send("POST",&format!("/ui/api/accounts/{id}/qr-confirm"),json!({})).await.0,200);
    assert_eq!(f.send("GET","/health",Value::Null).await.1["accounts_online"],2);
    f.finish().await;
}

#[tokio::test]
async fn legacy_start_keeps_paths_and_http_available_before_ready() {
    let dir=std::env::temp_dir().join(format!("tggw-legacy-http-{}",std::process::id()));
    std::fs::create_dir_all(dir.join("tdlib/files")).unwrap();
    std::fs::write(dir.join("tdlib/files/sentinel"),b"legacy DB").unwrap();
    std::fs::write(dir.join("tdlib/khoa"),b"00112233445566778899aabbccddeeff").unwrap();
    let (api,factory)=crate::td::fake::Fake::factory();
    let cfg=LoginConfig{data_dir:dir.clone(),api_id:12345,api_hash:"fake-api-hash".into(),phone:"+84900000000".into()};
    let hub=Accounts::with_factory(cfg,"fake".into(),3399,factory).await.unwrap();
    let router=rest_router(hub.clone());
    assert_eq!(send(&router,"GET","/health",Value::Null,false).await.0,200);
    assert_eq!(send(&router,"GET","/api/me",Value::Null,true).await.0,503);
    let deadline=tokio::time::Instant::now()+Duration::from_secs(3);
    while !api.sent.lock().unwrap().iter().any(|(_,r)|r["@type"]=="setAuthenticationPhoneNumber") {
        assert!(tokio::time::Instant::now()<deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    api.auth(1,json!({"@type":"authorizationStateReady"}));
    while send(&router,"GET","/health",Value::Null,false).await.1["accounts_online"]!=1 {
        assert!(tokio::time::Instant::now()<deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let sent=api.sent.lock().unwrap().clone();
    let parameters=sent.iter().find(|(_,r)|r["@type"]=="setTdlibParameters").unwrap();
    assert_eq!(parameters.1["database_directory"],dir.join("tdlib/files").to_string_lossy().as_ref());
    assert_eq!(std::fs::read(dir.join("tdlib/files/sentinel")).unwrap(),b"legacy DB");
    assert_eq!(std::fs::read(dir.join("tdlib/khoa")).unwrap(),b"00112233445566778899aabbccddeeff");
    api.profiles.lock().unwrap().insert(1,json!({"@type":"user","id":1,"first_name":"Tên","last_name":"hiện tại"}));
    assert_eq!(send(&router,"GET","/api/accounts",Value::Null,true).await,
        (200,json!({"tai_khoan":[{"phone":"+84900000000","display_name":"Tên hiện tại","status":"online"}]})));
    api.profiles.lock().unwrap().insert(1,json!({"@type":"user","id":1,"first_name":"Tên mới"}));
    assert_eq!(send(&router,"GET","/api/accounts",Value::Null,true).await.1["tai_khoan"][0]["display_name"],"Tên mới");
    assert!(!dir.join("accounts.json").exists());
    hub.shutdown().await;drop(router);drop(hub);std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn restart_restores_all_online_accounts_without_sending_phone_or_code() {
    let dir=std::env::temp_dir().join(format!("tggw-restore-http-{}",std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut store=AccountStore::open(&dir,"").unwrap();
    for phone in ["+84900000001","+84900000002"] {store.put_online(phone,"Thử".into()).unwrap();}
    let mut rows:Value=serde_json::from_slice(&std::fs::read(dir.join("accounts.json")).unwrap()).unwrap();
    rows.as_array_mut().unwrap().push(json!({"phone":"+84900000003","display_name":"Tên offline","status":"offline"}));
    crate::accounts::atomic_json(&dir.join("accounts.json"),&rows).unwrap();
    let (api,factory)=crate::td::fake::Fake::factory();
    api.restored.store(true,Ordering::Relaxed);
    let cfg=LoginConfig{data_dir:dir.clone(),api_id:12345,api_hash:"fake-api-hash".into(),phone:"+84900000000".into()};
    let hub=Accounts::with_factory(cfg,"fake".into(),3399,factory).await.unwrap();
    let router=rest_router(hub.clone());
    let deadline=tokio::time::Instant::now()+Duration::from_secs(3);
    loop {
        let health=send(&router,"GET","/health",Value::Null,false).await.1;
        assert_eq!(health["accounts_total"],3);
        if health["accounts_online"]==2 {break;}
        assert!(tokio::time::Instant::now()<deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(hub.all().len(),2);
    assert_eq!(send(&router,"GET","/api/me?account=%2B84900000003",Value::Null,true).await.0,503);
    assert_eq!(send(&router,"GET","/api/accounts",Value::Null,true).await.1["tai_khoan"][2],
        json!({"phone":"+84900000003","display_name":"Tên offline","status":"offline"}));
    let sent=api.sent.lock().unwrap().clone();
    let parameters:Vec<_>=sent.iter().filter(|(_,r)|r["@type"]=="setTdlibParameters").collect();
    assert_eq!(parameters.len(),2);
    assert_ne!(parameters[0].1["database_directory"],parameters[1].1["database_directory"]);
    assert!(!sent.iter().any(|(_,r)|matches!(r["@type"].as_str(),Some("setAuthenticationPhoneNumber"|"checkAuthenticationCode"|"checkAuthenticationPassword"))));
    hub.shutdown().await;drop(router);drop(hub);std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn live_body_account_and_query_override_preserve_python_peer_busy_rule() {
    let f=Fixture::new("live-body").await;
    f.ready("+84900000001").await;f.ready("+84900000002").await;
    let result=f.send("POST","/api/live/call",json!({"account":"+84900000001","user_id":501})).await;
    assert_eq!(result.0,200,"{result:?}");
    let result=f.send("POST","/api/live/call",json!({"account":"+84900000002","user_id":501})).await;
    assert_eq!(result.0,409);assert_eq!(result.1["error"]["code"],"CALL_BUSY");
    let result=f.send("POST","/api/live/call?account=%2B84900000002",json!({"account":"+84900000001","user_id":502})).await;
    assert_eq!(result.0,200,"{result:?}");
    let sent=f.api.sent.lock().unwrap().clone();
    let calls:Vec<_>=sent.iter().filter(|(_,r)|r["@type"]=="createCall").collect();
    assert_eq!(calls.len(),2);
    assert_eq!(calls[0].0,1);assert_eq!(calls[0].1["user_id"],501);
    assert_eq!(calls[1].0,2);assert_eq!(calls[1].1["user_id"],502);
    f.finish().await;
}

#[tokio::test]
async fn browser_devices_and_media_stay_with_the_selected_account() {
    let f=Fixture::new("browser-media").await;
    f.ready("+84900000001").await;f.ready("+84900000002").await;
    let second="?account=%2B84900000002";
    let settings=json!({"mic":"browser","speaker":"browser","camera":"black"});
    assert_eq!(f.send("POST",&format!("/api/live/settings{second}"),settings).await.0,200);
    let result=f.send("POST",&format!("/api/live/ha_devices{second}"),json!({"mic":[{"id":"sensor.mic","name":"Mic"}],"speaker":[],"camera":[]})).await;
    assert_eq!(result.0,200);
    assert_eq!(f.send("GET",&format!("/api/live/devices{second}"),Value::Null).await.1["catalog"]["mic"].as_array().unwrap().len(),1);
    assert!(f.send("GET","/api/live/devices",Value::Null).await.1["catalog"]["mic"].as_array().unwrap().is_empty());
    assert_eq!(f.send("GET",&format!("/api/live/settings{second}"),Value::Null).await.1["auto"],false);
    let first=f.send("POST","/api/live/call",json!({"user_id":701})).await.1;
    let other=f.send("POST",&format!("/api/live/call{second}"),json!({"user_id":702})).await.1;
    let mut urls=Vec::new();
    for c in [&first,&other] {
        let response=f.send("POST","/api/live/media",json!({"id":c["id"],"action":"audio_out"})).await;
        assert_eq!(response.0,200);urls.push(response.1["audio_out"]["url"].as_str().unwrap().to_owned());
    }
    let path=&urls[1];
    let response=f.router.clone().oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),200);assert_eq!(response.headers()["content-type"],"audio/wav");drop(response);
    let wrong=format!("/api/live/audio/out/{}?t={}",other["id"].as_str().unwrap(),urls[0].split("?t=").nth(1).unwrap());
    assert_eq!(send(&f.router,"GET",&wrong,Value::Null,false).await.0,403);
    let request=Request::builder().method("POST").uri(format!("/api/live/audio/in/{}",other["id"].as_str().unwrap()))
        .header("Authorization","Bearer test-token").body(Body::from(vec![0u8;3200])).unwrap();
    assert_eq!(f.router.clone().oneshot(request).await.unwrap().status(),409);
    assert_eq!(f.send("POST","/api/live/hangup",json!({"id":other["id"]})).await.0,200);
    assert_eq!(send(&f.router,"GET",path,Value::Null,false).await.0,404);
    f.finish().await;
}

#[tokio::test]
async fn browser_media_routes_calls_outside_the_recent_ten() {
    let f=Fixture::new("browser-old-call").await;
    f.ready("+84900000001").await;
    let mut first=Value::Null;
    for user in 800..812 {
        let response=f.send("POST","/api/live/call",json!({"user_id":user,"mic":"browser","speaker":"browser"})).await;
        assert_eq!(response.0,200);
        if first.is_null() { first=response.1; }
    }
    let id=first["id"].as_str().unwrap();
    let listed=f.send("GET","/api/live/calls",Value::Null).await.1;
    assert_eq!(listed["calls"].as_array().unwrap().len(),10);
    assert!(listed["calls"].as_array().unwrap().iter().all(|c|c["id"]!=id));
    let descriptor=f.send("POST","/api/live/media",json!({"id":id,"action":"audio_out"})).await;
    assert_eq!(descriptor.0,200);
    let url=descriptor.1["audio_out"]["url"].as_str().unwrap();
    let response=f.router.clone().oneshot(Request::builder().uri(url).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),200);drop(response);
    assert_eq!(f.send("GET",&format!("/api/live/snapshot/{id}"),Value::Null).await.0,409);
    let request=Request::builder().method("POST").uri(format!("/api/live/audio/in/{id}"))
        .header("Authorization","Bearer test-token").body(Body::from(vec![0u8;3200])).unwrap();
    assert_eq!(f.router.clone().oneshot(request).await.unwrap().status(),409);
    for row in listed["calls"].as_array().unwrap() {
        assert_eq!(f.send("POST", "/api/live/hangup", json!({"id":row["id"]})).await.0, 200);
    }
    assert!(f.send("GET", "/api/live/calls", Value::Null).await.1["calls"].as_array().unwrap().iter().all(|row| row["state"] == "ended"));
    assert_eq!(f.send("POST", "/api/storage/clear", json!({"scope":"all","confirm":true})).await.0, 409);
    assert_eq!(f.send("POST","/api/live/hangup",json!({"id":id})).await.0,200);
    // Kể cả tài khoản mất trạng thái online sau khi cúp, mã cuộc gọi hết vẫn trả 404.
    f.hub.all()[0].1.ready.store(false,Ordering::Release);
    assert_eq!(send(&f.router,"GET",url,Value::Null,false).await.0,404);
    f.finish().await;
}

#[tokio::test]
async fn authenticated_ui_2fa_qr_and_archive_contract() {
    let f=Fixture::new("login").await;
    for (method,path) in [("POST","/ui/api/accounts"),("POST","/ui/api/accounts/qr"),("GET","/ui/api/accounts/id"),
        ("DELETE","/ui/api/accounts/%2B84900000000"),("POST","/ui/api/accounts/id/code"),
        ("POST","/ui/api/accounts/id/password"),("POST","/ui/api/accounts/id/qr-confirm")] {
        assert_eq!(send(&f.router,method,path,json!({}),false).await.0,401);
    }
    let id=f.login("+84900000000").await;
    assert_eq!(f.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":"54321"})).await,
        (200,json!({"ok":true,"needs_password":true,"stage":"password"})));
    assert_eq!(f.send("POST",&format!("/ui/api/accounts/{id}/password"),json!({"password":" secret2fa "})).await.0,200);
    let session=f.hub.store.lock().unwrap().session_dir("+84900000000");
    std::fs::write(session.join("files/sentinel"),b"database").unwrap();
    assert_eq!(f.send("DELETE","/ui/api/accounts/%2B84900000000",Value::Null).await,(200,json!({"ok":true})));
    assert!(!session.exists());
    let archives:Vec<_>=std::fs::read_dir(f.dir.join("tdlib")).unwrap().map(|e|e.unwrap().path()).collect();
    assert_eq!(archives.len(),1);
    assert_eq!(std::fs::read(archives[0].join("files/sentinel")).unwrap(),b"database");
    assert!(archives[0].join("khoa").exists());
    assert!(f.hub.store.lock().unwrap().rows().is_empty());
    let qr=f.send("POST","/ui/api/accounts/qr",json!({"phone":"+84900000002"})).await;
    assert_eq!(qr.0,200);
    let id=qr.1["login_id"].as_str().unwrap();
    assert!(qr.1["qr_link"].as_str().unwrap().starts_with("tg://"));
    f.api.auth(2,json!({"@type":"authorizationStateReady"}));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.1["stage"],"done");
    f.finish().await;
}

#[tokio::test]
async fn downloads_use_selected_python_session_and_reject_other_account_paths() {
    let f=Fixture::new("downloads").await;
    f.ready("+84900000001").await;f.ready("+84900000002").await;
    let mut paths=Vec::new();
    for (index,phone) in ["+84900000001","+84900000002"].into_iter().enumerate() {
        let session=f.hub.store.lock().unwrap().session_dir(phone);
        std::fs::create_dir_all(session.join("files/documents")).unwrap();
        let path=session.join("files/documents/file.txt");
        std::fs::write(&path,format!("account-{index}")).unwrap();
        f.api.downloads.lock().unwrap().insert(index as i32+1,path.to_string_lossy().into_owned());
        paths.push(path);
    }
    for (index,phone) in ["+84900000001","+84900000002"].into_iter().enumerate() {
        let query=form_urlencoded::Serializer::new(String::new()).append_pair("account",phone).finish();
        let response=f.router.clone().oneshot(Request::builder().uri(format!("/api/file/42?{query}"))
            .header("Authorization","Bearer test-token").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),200);
        assert_eq!(to_bytes(response.into_body(),usize::MAX).await.unwrap().as_ref(),format!("account-{index}").as_bytes());
    }
    f.api.downloads.lock().unwrap().insert(1,paths[1].to_string_lossy().into_owned());
    assert_eq!(f.send("GET","/api/file/42?account=%2B84900000001",Value::Null).await.0,404);
    let alias=paths[0].with_file_name("alias.txt");
    std::os::unix::fs::symlink(&paths[1],&alias).unwrap();
    f.api.downloads.lock().unwrap().insert(1,alias.to_string_lossy().into_owned());
    assert_eq!(f.send("GET","/api/file/42?account=%2B84900000001",Value::Null).await.0,404);
    f.finish().await;
}

#[tokio::test]
async fn account_names_follow_get_me_independently_and_keep_saved_name_on_error_or_offline() {
    use std::os::unix::fs::MetadataExt;
    let f=Fixture::new("fresh-names").await;
    f.ready("+84900000001").await;f.ready("+84900000002").await;
    assert_eq!(f.send("GET","/api/accounts",Value::Null).await,
        (200,json!({"tai_khoan":[
            {"phone":"+84900000001","display_name":"Account 1","status":"online"},
            {"phone":"+84900000002","display_name":"Account 2","status":"online"}]})));
    f.api.profiles.lock().unwrap().insert(1,json!({"@type":"user","id":1,"first_name":"Tên","last_name":"mới"}));
    assert_eq!(f.send("GET","/api/accounts",Value::Null).await.1["tai_khoan"],json!([
        {"phone":"+84900000001","display_name":"Tên mới","status":"online"},
        {"phone":"+84900000002","display_name":"Account 2","status":"online"}]));
    f.api.profiles.lock().unwrap().insert(2,json!({"@type":"user","id":2,"first_name":"Tên riêng"}));
    assert_eq!(f.send("GET","/api/accounts",Value::Null).await.1["tai_khoan"],json!([
        {"phone":"+84900000001","display_name":"Tên mới","status":"online"},
        {"phone":"+84900000002","display_name":"Tên riêng","status":"online"}]));
    let path=f.dir.join("accounts.json");
    let saved=std::fs::read(&path).unwrap();
    let rows:Value=serde_json::from_slice(&saved).unwrap();
    assert_eq!(rows[0]["display_name"],"Tên mới");
    assert_eq!(rows[1]["display_name"],"Tên riêng");
    let ledger=std::fs::File::open(&path).unwrap();
    let inode=ledger.metadata().unwrap().ino();
    f.api.profiles.lock().unwrap().insert(1,json!({"@type":"error","code":500,"message":"fake getMe failure"}));
    f.api.profiles.lock().unwrap().insert(2,json!({"@type":"user","id":2,"first_name":"","last_name":""}));
    assert_eq!(f.send("GET","/api/accounts",Value::Null).await.1["tai_khoan"],json!([
        {"phone":"+84900000001","display_name":"Tên mới","status":"online"},
        {"phone":"+84900000002","display_name":"Tên riêng","status":"online"}]));
    f.hub.all()[0].1.ready.store(false,Ordering::Release);
    f.api.sent.lock().unwrap().clear();
    assert_eq!(f.send("GET","/api/accounts",Value::Null).await.1["tai_khoan"][0],
        json!({"phone":"+84900000001","display_name":"Tên mới","status":"offline"}));
    assert!(!f.api.sent.lock().unwrap().iter().any(|(id,r)|*id==1 && r["@type"]=="getMe"));
    assert_eq!(std::fs::read(&path).unwrap(),saved);
    assert_eq!(std::fs::metadata(&path).unwrap().ino(),inode);
    f.finish().await;
}

#[tokio::test]
async fn account_name_is_saved_once_and_repeated_polls_preserve_ledger_metadata() {
    use std::os::unix::fs::{MetadataExt,PermissionsExt};
    let f=Fixture::new("saved-name").await;
    f.ready("+84900000000").await;
    let path=f.dir.join("accounts.json");
    let original=std::fs::File::open(&path).unwrap();
    let mut expected:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    expected[0]["display_name"]=json!("Tên đổi");
    f.api.profiles.lock().unwrap().insert(1,json!({"@type":"user","id":1,"first_name":"Tên đổi"}));
    assert_eq!(f.send("GET","/api/accounts",Value::Null).await,
        (200,json!({"tai_khoan":[{"phone":"+84900000000","display_name":"Tên đổi","status":"online"}]})));
    let saved=std::fs::File::open(&path).unwrap();
    let metadata=saved.metadata().unwrap();
    assert_ne!(metadata.ino(),original.metadata().unwrap().ino());
    assert_eq!(metadata.permissions().mode() & 0o777,0o600);
    let bytes=std::fs::read(&path).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),expected);
    for _ in 0..3 {
        assert_eq!(f.send("GET","/api/accounts",Value::Null).await.1["tai_khoan"][0]["display_name"],"Tên đổi");
        let after=std::fs::metadata(&path).unwrap();
        assert_eq!(after.ino(),metadata.ino());
        assert_eq!(after.modified().unwrap(),metadata.modified().unwrap());
        assert_eq!(std::fs::read(&path).unwrap(),bytes);
    }
    assert_eq!(AccountStore::open(&f.dir,"").unwrap().rows()[0].display_name,"Tên đổi");
    f.finish().await;
}

#[tokio::test]
async fn switching_local_to_stream_requires_confirm_and_delays_janitor_until_confirmed() {
    let f = Fixture::new("storage-mode-confirm").await;
    let phone = "test-storage-confirm";
    f.ready(phone).await;
    *f.api.storage_stats.lock().unwrap() = json!({
        "@type":"storageStatistics","size":8,"count":1,
        "by_chat":[{"chat_id":7,"size":8,"count":1,"by_file_type":[]}]
    });
    let (_, mut settings) = f.send("GET", "/api/settings/storage", Value::Null).await;
    assert_eq!(settings["mode"], "local");
    let file = f.hub.store.lock().unwrap().session_dir(phone).join("storage_settings.json");

    settings["mode"] = json!("stream");
    assert_eq!(f.send("POST", "/api/settings/storage", settings.clone()).await.0, 400);
    let mut unconfirmed = settings.clone();
    unconfirmed["confirm"] = json!(false);
    assert_eq!(f.send("POST", "/api/settings/storage", unconfirmed).await.0, 400);
    assert_eq!(crate::storage_settings::load(&file.parent().unwrap()).unwrap()["mode"], "local");

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage"));

    let mut confirmed = settings.clone();
    confirmed["confirm"] = json!(true);
    let (status, saved) = f.send("POST", "/api/settings/storage", confirmed).await;
    assert_eq!((status, saved.clone()), (200, settings.clone()));
    assert!(saved.get("confirm").is_none());
    assert!(crate::storage_settings::load(file.parent().unwrap()).unwrap().get("confirm").is_none());

    // M1: lưu cấu hình không tự kích hoạt dọn ngay sau lưu; lượt dọn chạy theo lịch.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage"));

    let (_, entry) = f.hub.select(phone).unwrap();
    entry.storage_sweep(None).await.unwrap();
    assert!(f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage" && req["ttl"] == 600));

    settings["stream_ttl_s"] = json!(300);
    assert_eq!(f.send("POST", "/api/settings/storage", settings.clone()).await.0, 400);
    let mut confirm_ttl = settings.clone();
    confirm_ttl["confirm"] = json!(true);
    assert_eq!(f.send("POST", "/api/settings/storage", confirm_ttl).await.0, 200);
    settings["stream_ttl_s"] = json!(600);
    assert_eq!(f.send("POST", "/api/settings/storage", settings.clone()).await.0, 200);
    settings["mode"] = json!("local");
    assert_eq!(f.send("POST", "/api/settings/storage", settings).await.0, 200);
    f.finish().await;
}

#[tokio::test]
async fn corrupted_storage_settings_fail_writes_and_janitor_but_allow_file_view_and_safe_read() {
    let f = Fixture::new("storage-corrupt").await;
    let phone = "test-storage-corrupt";
    f.ready(phone).await;
    let (_, entry) = f.hub.select(phone).unwrap();
    std::fs::create_dir_all(entry.session.join("files/photos")).unwrap();
    let pic = entry.session.join("files/photos/pic.jpg");
    std::fs::write(&pic, b"pic-bytes").unwrap();
    f.api.downloads.lock().unwrap().insert(1, pic.to_string_lossy().into_owned());
    let file = entry.session.join("storage_settings.json");

    for raw in [b"{corrupt".as_slice(), b"{\"mode\":\"unknown\"}".as_slice()] {
        std::fs::write(&file, raw).unwrap();
        let response = f.router.clone().oneshot(
            Request::builder().uri("/api/file/1").header("Authorization", "Bearer test-token")
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[axum::http::header::CACHE_CONTROL], "private, max-age=86400");
        assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), b"pic-bytes");

        let (status, read_back) = f.send("GET", "/api/settings/storage", Value::Null).await;
        assert_eq!(status, 200);
        assert_eq!(read_back["mode"], "local");
        assert_eq!(read_back["keep"], json!({"private":null,"groups":null,"channels":null}));
        assert_eq!(read_back["max_cache_bytes"], Value::Null);

        let (status, err) = f.send("POST", "/api/settings/storage", crate::storage_settings::safe_default()).await;
        assert_eq!(status, 500);
        assert_eq!(err["error"]["code"], "STORAGE_SETTINGS_IO");
        assert_eq!(std::fs::read(&file).unwrap(), raw);

        let sweep_err = entry.storage_sweep(None).await.unwrap_err();
        assert_eq!((sweep_err.status, sweep_err.code.as_str()), (500, "STORAGE_SETTINGS_IO"));
        assert_eq!(f.send("GET", "/api/storage/janitor", Value::Null).await.1["error"], "STORAGE_SETTINGS_IO");
        assert!(!f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage"));
    }
    f.finish().await;
}

#[tokio::test]
async fn long_storage_sweep_does_not_block_concurrent_api_and_noop_skips_scan() {
    let f = Fixture::new("storage-t4").await;
    let phone = "test-storage-t4";
    f.ready(phone).await;
    let (_, entry) = f.hub.select(phone).unwrap();
    let files_dir = entry.session.join("files");
    std::fs::create_dir_all(files_dir.join("documents")).unwrap();
    let doc = files_dir.join("documents/doc.txt");
    std::fs::write(&doc, b"doc-bytes").unwrap();
    f.api.downloads.lock().unwrap().insert(1, doc.to_string_lossy().into_owned());

    // Chính sách không làm gì (no-op) không quét cây tệp và không gọi TDLib.
    crate::storage_settings::save(&entry.session, &crate::storage_settings::safe_default()).unwrap();
    let sym = files_dir.join("documents/sym");
    std::os::unix::fs::symlink(&doc, &sym).unwrap();
    f.api.sent.lock().unwrap().clear();
    entry.storage_sweep(None).await.unwrap();
    assert!(!f.api.sent.lock().unwrap().iter().any(|(_, req)| matches!(req["@type"].as_str(), Some("getStorageStatistics" | "optimizeStorage"))));
    std::fs::remove_file(sym).unwrap();

    // Lượt dọn dài (quét cây + optimizeStorage chậm) không giữ khoá ghi chặn API hay xem tệp.
    let mut stream = crate::storage_settings::safe_default();
    stream["mode"] = json!("stream");
    crate::storage_settings::save(&entry.session, &stream).unwrap();
    *f.api.storage_stats.lock().unwrap() = json!({
        "@type":"storageStatistics","size":100,"count":1,
        "by_chat":[{"chat_id":7,"size":100,"count":1,"by_file_type":[]}]
    });
    *f.api.optimize_delay.lock().unwrap() = Some(Duration::from_millis(350));
    f.api.sent.lock().unwrap().clear();

    let sweep_entry = entry.clone();
    let sweep = tokio::spawn(async move { sweep_entry.storage_sweep(None).await });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage") {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let started = std::time::Instant::now();
    assert_eq!(f.send("GET", "/api/me", Value::Null).await.0, 200);
    assert_eq!(f.send("GET", "/api/storage/usage", Value::Null).await.0, 200);
    assert_eq!(f.send("GET", "/api/settings/storage", Value::Null).await.0, 200);
    let file_resp = f.router.clone().oneshot(
        Request::builder().uri("/api/file/1").header("Authorization", "Bearer test-token")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(file_resp.status(), 200);
    assert!(started.elapsed() < Duration::from_millis(150), "elapsed {:?}", started.elapsed());

    sweep.await.unwrap().unwrap();
    f.finish().await;
}

#[tokio::test]
async fn storage_routes_enforce_auth_and_cookie_csrf() {
    let dir = std::env::temp_dir().join(format!("tggw-storage-csrf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (api, factory) = crate::td::fake::Fake::factory();
    let cfg = LoginConfig { data_dir: dir.clone(), api_id: 12345, api_hash: "fake-api-hash".into(), phone: String::new() };
    let hub = Accounts::with_factory(cfg, "fake".into(), 3399, factory).await.unwrap();
    let ui_auth = Arc::new(crate::ui_auth::UiAuth::open(&dir, "test-token", None, None, false).unwrap());
    let router = crate::rest::router_accounts_with_ui_auth(hub.clone(), crate::rest::RestConfig {
        token: "test-token".into(), rate_limit_per_minute: 1000, version: "test".into(),
    }, ui_auth);

    let login = send(&router, "POST", "/ui/api/accounts", json!({"phone":"test-storage-csrf"}), true).await.1;
    let id = login["login_id"].as_str().unwrap();
    assert_eq!(send(&router, "POST", &format!("/ui/api/accounts/{id}/code"), json!({"code":"22222"}), true).await.0, 200);

    let setup_resp = router.clone().oneshot(
        Request::builder().method("POST").uri("/ui/api/auth/setup")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"token":"test-token","ten":"nguoi-thu","mat_khau":"mat-khau-thu-dai"}).to_string())).unwrap(),
    ).await.unwrap();
    assert_eq!(setup_resp.status(), 200);
    let cookie = setup_resp.headers()[axum::http::header::SET_COOKIE].to_str().unwrap().split(';').next().unwrap().to_owned();

    // Không có xác thực -> 401.
    for (method, path) in [
        ("GET", "/api/settings/storage"), ("POST", "/api/settings/storage"),
        ("GET", "/api/storage/usage"), ("GET", "/api/storage/janitor"), ("POST", "/api/storage/clear"),
    ] {
        assert_eq!(send(&router, method, path, json!({}), false).await.0, 401, "{method} {path}");
    }

    let (_, settings) = send(&router, "GET", "/api/settings/storage", Value::Null, true).await;
    let mut next_settings = settings.clone();
    next_settings["max_cache_bytes"] = json!(5 * 1024 * 1024 * 1024_u64);

    // Cookie ghi thiếu CSRF hoặc sai Origin -> 403.
    let send_cookie = |method: &'static str, path: &'static str, body: Value, extra: Vec<(&'static str, &'static str)>| {
        let router = router.clone();
        let cookie = cookie.clone();
        async move {
            let mut req = Request::builder().method(method).uri(path).header(axum::http::header::COOKIE, cookie);
            for (k, v) in extra { req = req.header(k, v); }
            let resp = router.oneshot(req.body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
            resp.status().as_u16()
        }
    };
    for (path, body) in [
        ("/api/settings/storage", next_settings.clone()),
        ("/api/storage/clear", json!({"scope":"all","confirm":true})),
    ] {
        assert_eq!(send_cookie("POST", path, body.clone(), vec![]).await, 403);
        assert_eq!(send_cookie("POST", path, body, vec![("Host", "gateway.example"), ("Origin", "https://evil.example")]).await, 403);
    }
    assert_eq!(send(&router, "GET", "/api/settings/storage", Value::Null, true).await.1, settings);
    assert!(!api.sent.lock().unwrap().iter().any(|(_, r)| r["@type"] == "optimizeStorage"));

    // Cookie kèm X-Requested-With hoặc khớp Host/Origin -> 200.
    assert_eq!(send_cookie("GET", "/api/storage/usage", Value::Null, vec![]).await, 200);
    assert_eq!(send_cookie("POST", "/api/settings/storage", next_settings, vec![("X-Requested-With", "fetch")]).await, 200);
    assert_eq!(send_cookie("POST", "/api/storage/clear", json!({"scope":"all","confirm":true}), vec![("Host", "gateway.example"), ("Origin", "http://gateway.example")]).await, 200);

    hub.shutdown().await;
    drop(router);
    drop(hub);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn legacy_account_serves_avatar_and_thumbnail_and_preserves_them_on_janitor() {
    let dir = std::env::temp_dir().join(format!("tggw-legacy-media-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("tdlib/files/profile_photos")).unwrap();
    std::fs::create_dir_all(dir.join("tdlib/files/thumbnails")).unwrap();
    let avatar = dir.join("tdlib/files/profile_photos/avatar.jpg");
    let thumb = dir.join("tdlib/files/thumbnails/thumb.jpg");
    std::fs::write(&avatar, b"avatar-bytes").unwrap();
    std::fs::write(&thumb, b"thumb-bytes").unwrap();
    std::fs::write(dir.join("tdlib/khoa"), b"00112233445566778899aabbccddeeff").unwrap();

    let (api, factory) = crate::td::fake::Fake::factory();
    api.restored.store(true, Ordering::Relaxed);
    let cfg = LoginConfig { data_dir: dir.clone(), api_id: 12345, api_hash: "fake-api-hash".into(), phone: "test-storage-legacy".into() };
    let hub = Accounts::with_factory(cfg, "fake".into(), 3399, factory).await.unwrap();
    let router = rest_router(hub.clone());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while send(&router, "GET", "/health", Value::Null, false).await.1["accounts_online"] != 1 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    for (path, expected) in [(avatar.clone(), b"avatar-bytes".as_slice()), (thumb.clone(), b"thumb-bytes".as_slice())] {
        api.downloads.lock().unwrap().insert(1, path.to_string_lossy().into_owned());
        let resp = router.clone().oneshot(
            Request::builder().uri("/api/file/1").header("Authorization", "Bearer test-token")
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()[axum::http::header::CONTENT_TYPE], "image/jpeg");
        assert_eq!(resp.headers()[axum::http::header::CACHE_CONTROL], "private, max-age=86400");
        assert_eq!(to_bytes(resp.into_body(), 1024).await.unwrap().as_ref(), expected);
    }

    let (_, settings) = send(&router, "GET", "/api/settings/storage", Value::Null, true).await;
    assert_eq!(settings["keep"], json!({"private":null,"groups":null,"channels":null}));
    assert_eq!(settings["max_cache_bytes"], Value::Null);

    let (_, entry) = hub.select("test-storage-legacy").unwrap();
    entry.storage_sweep(None).await.unwrap();
    assert!(!api.sent.lock().unwrap().iter().any(|(_, r)| r["@type"] == "optimizeStorage"));
    assert_eq!(std::fs::read(avatar).unwrap(), b"avatar-bytes");
    assert_eq!(std::fs::read(thumb).unwrap(), b"thumb-bytes");

    hub.shutdown().await;
    drop(router);
    drop(hub);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn multi_account_storage_routes_require_explicit_account_parameter() {
    let f = Fixture::new("storage-multi-account").await;
    let first = "test-storage-acc1";
    let second = "test-storage-acc2";
    f.ready(first).await;
    f.ready(second).await;

    for (method, path, body) in [
        ("GET", "/api/settings/storage", Value::Null),
        ("POST", "/api/settings/storage", crate::storage_settings::safe_default()),
        ("GET", "/api/storage/usage", Value::Null),
        ("GET", "/api/storage/janitor", Value::Null),
        ("POST", "/api/storage/clear", json!({"scope":"all","confirm":true})),
        ("POST", "/api/storage/clear", json!({"scope":"all","confirm":true,"account":""})),
    ] {
        assert_eq!(f.send(method, path, body).await.0, 400, "{method} {path}");
    }

    // Kể cả khi một tài khoản offline, vẫn từ chối account bỏ trống.
    f.hub.select(second).unwrap().1.ready.store(false, Ordering::Release);
    assert_eq!(f.send("GET", "/api/storage/usage", Value::Null).await.0, 400);
    assert_eq!(f.send("GET", "/api/storage/usage?account=test-storage-acc1", Value::Null).await.0, 200);
    assert_eq!(f.send("GET", "/api/storage/usage?account=test-storage-unknown", Value::Null).await.0, 404);
    f.finish().await;
}

#[tokio::test]
async fn tightening_storage_policy_requires_confirm_and_never_triggers_immediate_sweep() {
    let f = Fixture::new("storage-tighten").await;
    let phone = "test-storage-tighten";
    f.ready(phone).await;
    *f.api.storage_stats.lock().unwrap() = json!({
        "@type":"storageStatistics","size":100,"count":1,
        "by_chat":[{"chat_id":7,"size":100,"count":1,"by_file_type":[]}]
    });

    let (_, mut settings) = f.send("GET", "/api/settings/storage", Value::Null).await;
    // Giảm thời hạn giữ hoặc giới hạn bộ nhớ đệm thiếu confirm -> 400.
    let mut shorter_keep = settings.clone();
    shorter_keep["keep"]["private"] = json!(86400);
    assert_eq!(f.send("POST", "/api/settings/storage", shorter_keep.clone()).await.0, 400);
    assert_eq!(f.send("GET", "/api/settings/storage", Value::Null).await.1["keep"]["private"], 604800);

    shorter_keep["confirm"] = json!(true);
    assert_eq!(f.send("POST", "/api/settings/storage", shorter_keep).await.0, 200);

    let mut smaller_cache = f.send("GET", "/api/settings/storage", Value::Null).await.1;
    smaller_cache["max_cache_bytes"] = json!(500 * 1024 * 1024_u64);
    assert_eq!(f.send("POST", "/api/settings/storage", smaller_cache.clone()).await.0, 400);
    smaller_cache["confirm"] = json!(true);
    assert_eq!(f.send("POST", "/api/settings/storage", smaller_cache).await.0, 200);

    // Nới lỏng và thêm ngoại lệ Mãi mãi không cần confirm -> 200.
    settings["max_cache_bytes"] = json!(5 * 1024 * 1024 * 1024_u64);
    settings["exceptions"] = json!([{"chat_id":43,"keep":null}]);
    assert_eq!(f.send("POST", "/api/settings/storage", settings.clone()).await.0, 200);

    // Gỡ ngoại lệ Mãi mãi khi chính sách còn giới hạn -> 400 nếu thiếu confirm.
    settings["exceptions"] = json!([]);
    assert_eq!(f.send("POST", "/api/settings/storage", settings.clone()).await.0, 400);
    settings["confirm"] = json!(true);
    assert_eq!(f.send("POST", "/api/settings/storage", settings).await.0, 200);

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage"));
    f.finish().await;
}

#[tokio::test]
async fn saving_forever_exception_during_active_sweep_aborts_before_deleting_that_chat() {
    let f = Fixture::new("storage-mid-sweep").await;
    let phone = "test-storage-mid-sweep";
    f.ready(phone).await;
    let (_, entry) = f.hub.select(phone).unwrap();

    let mut initial = crate::storage_settings::safe_default();
    initial["keep"] = json!({"private":86400,"groups":604800,"channels":null});
    initial["confirm"] = json!(true);
    assert_eq!(f.send("POST", "/api/settings/storage", initial).await.0, 200);

    *f.api.storage_chats.lock().unwrap() = [
        (40_i64, json!({"@type":"chatTypePrivate"})),
        (41_i64, json!({"@type":"chatTypeBasicGroup"})),
        (42_i64, json!({"@type":"chatTypeBasicGroup"})),
    ].into_iter().collect();
    *f.api.storage_stats.lock().unwrap() = json!({
        "@type":"storageStatistics","size":200,"count":2,
        "by_chat":[
            {"chat_id":40,"size":100,"count":1,"by_file_type":[]},
            {"chat_id":41,"size":100,"count":1,"by_file_type":[]}
        ]
    });
    *f.api.optimize_delay.lock().unwrap() = Some(Duration::from_millis(250));
    f.api.sent.lock().unwrap().clear();

    let sweep_entry = entry.clone();
    let sweep = tokio::spawn(async move { sweep_entry.storage_sweep(None).await });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !f.api.sent.lock().unwrap().iter().any(|(_, req)| req["@type"] == "optimizeStorage") {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let mut updated = f.send("GET", "/api/settings/storage", Value::Null).await.1;
    updated["exceptions"] = json!([{"chat_id":41,"keep":null}]);
    assert_eq!(f.send("POST", "/api/settings/storage", updated).await.0, 200);

    sweep.await.unwrap().unwrap();
    let optimize_reqs: Vec<_> = f.api.sent.lock().unwrap().iter()
        .filter(|(_, req)| req["@type"] == "optimizeStorage")
        .map(|(_, req)| req.clone())
        .collect();
    assert_eq!(optimize_reqs.len(), 1);
    assert_eq!(optimize_reqs[0]["chat_ids"], json!([40]));

    let (_, janitor) = f.send("GET", "/api/storage/janitor", Value::Null).await;
    assert_eq!(janitor["policy_changed"], true);
    assert!(janitor["error"].is_null());

    // Lượt dọn kế tiếp đọc cấu hình mới và giữ nguyên chat 41.
    *f.api.optimize_delay.lock().unwrap() = None;
    f.api.sent.lock().unwrap().clear();
    *f.api.storage_stats.lock().unwrap() = json!({
        "@type":"storageStatistics","size":200,"count":2,
        "by_chat":[
            {"chat_id":41,"size":100,"count":1,"by_file_type":[]},
            {"chat_id":42,"size":100,"count":1,"by_file_type":[]}
        ]
    });
    entry.storage_sweep(None).await.unwrap();
    let next_reqs: Vec<_> = f.api.sent.lock().unwrap().iter()
        .filter(|(_, req)| req["@type"] == "optimizeStorage")
        .map(|(_, req)| req.clone())
        .collect();
    assert_eq!(next_reqs.len(), 1);
    assert_eq!(next_reqs[0]["chat_ids"], json!([42]));
    assert_eq!(next_reqs[0]["exclude_chat_ids"], json!([41]));
    f.finish().await;
}

#[tokio::test]
async fn manual_clear_all_and_type_preserve_forever_chats_until_explicit_chat_clear() {
    let f = Fixture::new("storage-clear-m4").await;
    let phone = "test-storage-clear-m4";
    f.ready(phone).await;
    let (_, entry) = f.hub.select(phone).unwrap();

    let mut settings = f.send("GET", "/api/settings/storage", Value::Null).await.1;
    settings["exceptions"] = json!([{"chat_id":43,"keep":null}]);
    assert_eq!(f.send("POST", "/api/settings/storage", settings).await.0, 200);

    *f.api.storage_stats.lock().unwrap() = json!({
        "@type":"storageStatistics","size":300,"count":2,
        "by_chat":[
            {"chat_id":10,"size":100,"count":1,"by_file_type":[{"file_type":{"@type":"fileTypePhoto"},"size":100,"count":1}]},
            {"chat_id":43,"size":200,"count":1,"by_file_type":[{"file_type":{"@type":"fileTypePhoto"},"size":200,"count":1}]}
        ]
    });

    assert_eq!(f.send("POST", "/api/storage/clear", json!({"scope":"type","file_types":["photo"],"confirm":true})).await.0, 200);
    assert_eq!(f.api.sent.lock().unwrap().iter().rfind(|(_, r)| r["@type"] == "optimizeStorage").unwrap().1["exclude_chat_ids"], json!([43]));

    assert_eq!(f.send("POST", "/api/storage/clear", json!({"scope":"all","confirm":true})).await.0, 200);
    assert_eq!(f.api.sent.lock().unwrap().iter().rfind(|(_, r)| r["@type"] == "optimizeStorage").unwrap().1["exclude_chat_ids"], json!([43]));

    assert_eq!(f.send("POST", "/api/storage/clear", json!({"scope":"chat","chat_id":43,"confirm":true})).await.0, 200);
    let explicit = f.api.sent.lock().unwrap().iter().rfind(|(_, r)| r["@type"] == "optimizeStorage").unwrap().1.clone();
    assert_eq!(explicit["chat_ids"], json!([43]));
    assert_eq!(explicit["exclude_chat_ids"], json!([]));

    // Nếu cấu hình trên đĩa hỏng thì xoá toàn bộ không được tự đoán mà phải dừng với 500.
    let count_before = f.api.sent.lock().unwrap().iter().filter(|(_, r)| r["@type"] == "optimizeStorage").count();
    std::fs::write(entry.session.join(crate::storage_settings::FILE), b"{corrupt").unwrap();
    let (status, err) = f.send("POST", "/api/storage/clear", json!({"scope":"all","confirm":true})).await;
    assert_eq!(status, 500);
    assert_eq!(err["error"]["code"], "STORAGE_SETTINGS_IO");
    assert_eq!(f.api.sent.lock().unwrap().iter().filter(|(_, r)| r["@type"] == "optimizeStorage").count(), count_before);
    f.finish().await;
}
