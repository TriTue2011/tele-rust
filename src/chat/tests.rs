use std::{collections::VecDeque, sync::Mutex};
use tokio::sync::broadcast;

use super::*;
use crate::contract::TdError;

type FakeReply = (Value, Result<Value, TdError>);

mod catalog;
mod comments;
mod contacts;
mod info;
mod interaction;
mod rich_text;
mod shape;
mod writing;

struct FakeTd {
    calls: Mutex<VecDeque<FakeReply>>,
    updates: broadcast::Sender<Value>,
    /// openChat/closeChat là yêu cầu nền: không nằm trong kịch bản, chỉ được ghi lại để khẳng định.
    background: Mutex<Vec<(String, i64)>>,
}

impl TdLink for FakeTd {
    fn request(&self, req: Value) -> BoxFut<'_, Result<Value, TdError>> {
        Box::pin(async move {
            if let Some(kind) = req["@type"].as_str().filter(|k| matches!(*k, "openChat" | "closeChat")) {
                self.background.lock().unwrap().push((kind.to_owned(), req["chat_id"].as_i64().unwrap_or(0)));
                return Ok(json!({"@type":"ok"}));
            }
            let (expected, result) = self.calls.lock().unwrap().pop_front().expect("TDLib request ngoài dự kiến");
            writing::check_request(req, expected);
            result
        })
    }
    fn updates(&self) -> broadcast::Receiver<Value> { self.updates.subscribe() }
}

impl Drop for FakeTd {
    fn drop(&mut self) {
        if !std::thread::panicking() { assert!(self.calls.lock().unwrap().is_empty()); }
    }
}

fn setup(calls: Vec<FakeReply>) -> (ChatService, Arc<FakeTd>) {
    let td = Arc::new(FakeTd { calls: Mutex::new(calls.into()), updates: broadcast::channel(64).0, background: Mutex::new(Vec::new()) });
    (ChatService::new(td.clone(), "+000123", "target/chat-tests"), td)
}

fn ok(req: Value, resp: Value) -> FakeReply { (req, Ok(resp)) }
fn err(req: Value, code: i64, message: &str) -> FakeReply { (req, Err(TdError::telegram(code, message))) }
async fn get(chat: &ChatService, path: &str, query: &str) -> Reply { chat.handle("GET", path, query, Value::Null).await }
async fn post(chat: &ChatService, path: &str, query: &str, body: Value) -> Reply { chat.handle("POST", path, query, body).await }

fn td_message(id: i64) -> Value {
    json!({"id":id,"chat_id":5,"sender_id":{"@type":"messageSenderUser","user_id":7},
        "content":{"@type":"messageText","text":{"text":" Chào "}}})
}

fn normalized(id: i64, sender: &str) -> Value {
    json!({"id":id,"chat_id":5,"date":0,"edit_date":0,"outgoing":false,"sender":sender,"kind":"text",
        "text":" Chào ","entities":[],"summary":"Chào","reply_to":0,"reactions":[],"file":null})
}

#[tokio::test]
async fn accounts_and_me_keep_distinct_python_shapes() {
    let user = json!({"id":7,"first_name":"Lê","last_name":"Bình","phone_number":"000123"});
    let (chat, _) = setup(vec![ok(json!({"@type":"getMe"}), user.clone()), ok(json!({"@type":"getMe"}), user)]);
    assert_eq!(get(&chat,"/api/accounts", "").await.unwrap(),json!({"tai_khoan":[{"phone":"+000123","display_name":"Lê Bình","status":"online"}]}));
    assert_eq!(get(&chat,"/api/me", "account=wrong&account=%2B000123").await.unwrap(),json!({"id":7,"name":"Lê Bình","phone":"000123"}));
}

#[tokio::test]
async fn chats_and_search_summarize_and_skip_missing_chats() {
    let td_chat = json!({"id":5,"title":"Nhóm","type":{"@type":"chatTypeSupergroup","is_channel":true},"last_message":td_message(3),
        "unread_count":2,"unread_mention_count":1,"positions":[{"is_pinned":true}],"photo":{"small":{"id":10}},"notification_settings":{"mute_for":30}});
    let list = json!({"@type":"chatListMain"});
    let (chat, _) = setup(vec![
        err(json!({"@type":"loadChats","chat_list":list,"limit":200}),404,""),
        ok(json!({"@type":"getChats","chat_list":list,"limit":200}),json!({"chat_ids":[5,9]})),
        ok(json!({"@type":"getChat","chat_id":5}),td_chat.clone()),err(json!({"@type":"getChat","chat_id":9}),404,"Chat not found"),
        ok(json!({"@type":"searchChats","query":"nhóm a","limit":20}),json!({"chat_ids":[5]})),
        ok(json!({"@type":"getChat","chat_id":5}),td_chat),
    ]);
    let expected = json!({"chats":[{"id":5,"title":"Nhóm","kind":"channel","unread":2,"unread_mentions":1,"pinned":true,
        "archived":false,"marked_unread":false,"muted":true,"verified":false,"online":false,"status":"","was_online":0,
        "member_count":0,"auto_delete_seconds":0,"last_read_outbox_message_id":0,
        "last":{"id":3,"date":0,"outgoing":false,"kind":"text","text":"Chào","sender":"","read":false,"thumb_file":null},
        "avatar_file":10,"user_id":null}]});
    assert_eq!(get(&chat,"/api/chats","limit=999").await.unwrap(),expected);
    assert_eq!(get(&chat,"/api/search","q=+nh%C3%B3m+a+").await.unwrap(),expected);
    assert_eq!(get(&chat,"/api/search","q=+").await.unwrap(),json!({"chats":[]}));
}

#[tokio::test]
async fn history_paginates_sorts_and_contacts_refresh_user_details() {
    let req = |from, limit| json!({"@type":"getChatHistory","chat_id":5,"from_message_id":from,"offset":0,"limit":limit,"only_local":false});
    let (chat, _) = setup(vec![ok(req(11,3),json!({"messages":[td_message(10)]})),ok(req(10,2),json!({"messages":[td_message(8)]})),
        ok(req(8,1),json!({"messages":[]})),ok(json!({"@type":"getUser","user_id":7}),json!({"id":7,"first_name":"Zed"})),
        ok(json!({"@type":"getContacts"}),json!({"user_ids":[7,8]})),
        ok(json!({"@type":"getUser","user_id":7}),json!({"id":7,"first_name":"Zed"})),
        ok(json!({"@type":"getUser","user_id":8}),json!({"id":8,"usernames":{"active_usernames":["amy"]}})),
        ok(json!({"@type":"getBlockedMessageSenders","block_list":{"@type":"blockListMain"},"offset":0,"limit":100}),json!({"senders":[]})),
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}),json!({"id":5})),
    ]);
    assert_eq!(get(&chat,"/api/chats/5/messages","before=11&limit=3").await.unwrap(),json!({"messages":[normalized(8,"Zed"),normalized(10,"Zed")]}));
    let contacts = get(&chat,"/api/contacts", "").await.unwrap();
    assert_eq!(contacts["contacts"][0]["name"], "Zed");
    assert_eq!(contacts["contacts"][1]["username"], "amy");
    assert_eq!(post(&chat,"/api/contacts/open","",json!({"user_id":"7"})).await.unwrap(),json!({"chat_id":5}));
}

#[tokio::test]
async fn feed_long_poll_wakes_for_update_without_network_calls() {
    let (chat, td) = setup(vec![]);
    assert_eq!(get(&chat,"/api/feed","wait=0").await.unwrap(),json!({"items":[],"cursor":0,"reset":false}));
    let updater = async {
        tokio::task::yield_now().await;
        td.updates.send(json!({"@type":"updateUser","user":{"id":7,"first_name":"An"}})).unwrap();
        td.updates.send(json!({"@type":"updateNewMessage","message":td_message(6)})).unwrap();
    };
    let (feed, ()) = tokio::join!(get(&chat,"/api/feed","since=0&wait=1"),updater);
    let mut feed = feed.unwrap();
    assert!(feed["items"][0]["ts"].is_f64());
    feed["items"][0].as_object_mut().unwrap().remove("ts");
    assert_eq!(feed,json!({"items":[{"id":1,"type":"message","chat_id":5,"message":normalized(6,"An")}],"cursor":1,"reset":false}));
    assert_eq!(get(&chat,"/api/feed","since=1").await.unwrap(),json!({"items":[],"cursor":1,"reset":false}));
}

#[tokio::test]
async fn chat_feed_emits_only_final_message_metadata_to_events() {
    use crate::events::EventBridge;
    let data = PathBuf::from(format!("target/chat-event-test-{}", std::process::id()));
    let bridge = EventBridge::start(&data, "+000123", None).await.unwrap();
    let (chat, td) = setup(vec![]);
    drop(chat);
    let chat = ChatService::with_events(td.clone(), "+000123", &data, Some(bridge.sink.clone()));
    let mut draft = td_message(4);
    draft["sending_state"] = json!({"@type":"messageSendingStatePending"});
    td.updates.send(json!({"@type":"updateNewMessage","message":draft})).unwrap();
    td.updates.send(json!({"@type":"updateMessageSendSucceeded","old_message_id":4,"message":td_message(7)})).unwrap();
    let value = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = bridge.store.poll(0).await.unwrap();
            if value["cursor"].as_u64().unwrap() > 0 { break value; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert_eq!(value["events"].as_array().unwrap().len(), 1);
    assert_eq!(value["events"][0]["type"], "message_new");
    assert_eq!(value["events"][0]["data"], json!({"peer":"••••","tin_id":"7"}));
    td.updates.send(json!({"@type":"updateMessageEdited","chat_id":5,"message_id":7})).unwrap();
    td.updates.send(json!({"@type":"updateMessageContent","chat_id":5,"message_id":7})).unwrap();
    td.updates.send(json!({"@type":"updateDeleteMessages","chat_id":5,"message_ids":[7],"is_permanent":false})).unwrap();
    td.updates.send(json!({"@type":"updateDeleteMessages","chat_id":5,"message_ids":[7,8],"is_permanent":true})).unwrap();
    let mut contact = td_message(9);
    contact["content"] = json!({"@type":"messageContactRegistered"});
    td.updates.send(json!({"@type":"updateNewMessage","message":contact})).unwrap();
    let value = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let value = bridge.store.poll(0).await.unwrap();
            if value["cursor"].as_u64().unwrap() >= 6 { break value; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let actual: Vec<_> = value["events"].as_array().unwrap().iter().map(|e| json!({"type":e["type"],"data":e["data"]})).collect();
    assert_eq!(actual, vec![
        json!({"type":"message_new","data":{"peer":"••••","tin_id":"7"}}),
        json!({"type":"message_edited","data":{"chat_id":5,"message_id":7}}),
        json!({"type":"message_edited","data":{"chat_id":5,"message_id":7}}),
        json!({"type":"message_deleted","data":{"chat_id":5,"ids":[7,8]}}),
        json!({"type":"message_new","data":{"peer":"••••","tin_id":"9"}}),
        json!({"type":"contact_joined","data":{"peer":"••••"}}),
    ]);
    drop(chat);
    bridge.shutdown().await;
    std::fs::remove_dir_all(data).unwrap();
}

#[tokio::test]
async fn invalid_arguments_unknown_account_and_binary_routes_never_call_tdlib() {
    let (chat, _) = setup(vec![]);
    for (path, query, status, code) in [
        ("/api/chats", "limit=x",400,"BAD_REQUEST"),("/api/chats/5/messages","before=x",400,"BAD_REQUEST"),
        ("/api/feed","wait=x",400,"BAD_REQUEST"),("/api/me","account=wrong",404,"NOT_FOUND"),
        ("/api/unknown","",404,"NOT_FOUND"),
    ] {
        let e = get(&chat,path,query).await.unwrap_err();
        assert_eq!((e.status,e.code.as_str()),(status,code));
    }
    assert_eq!(post(&chat,"/api/contacts/open","",json!({})).await.unwrap_err().status,400);
    assert_eq!(chat.bytes("/api/file/2", "account=wrong").await.unwrap_err().status, 404);
}

#[tokio::test]
async fn tdlib_errors_map_to_gateway_errors() {
    let req = json!({"@type":"getMe"});
    let (chat, _) = setup(vec![err(req.clone(),429,"FLOOD_WAIT_5"),err(req.clone(),403,""),
        err(req.clone(),401,"Unauthorized"),err(req.clone(),404,"Chat not found"),
        err(req.clone(),500,"Internal"),err(req.clone(),400,"USER_INVALID"),err(req.clone(),429,"Slow down"),
        (req.clone(),Err(TdError::transport("TDLib không trả lời getMe sau 30 giây"))),
        (req,Err(TdError::transport("mất kết nối TDLib (luồng nhận đã dừng)")))]);
    for (status, code, retry) in [(429,"FLOOD_WAIT",Some(5)),(403,"TELEGRAM_ERROR",None),
        (401,"TELEGRAM_ERROR",None),(404,"TELEGRAM_ERROR",None),(502,"MEDIA_ENGINE_ERROR",None),
        (400,"TELEGRAM_ERROR",None),(429,"TELEGRAM_ERROR",None),(504,"TELEGRAM_TIMEOUT",None),
        (503,"ACCOUNT_OFFLINE",None)] {
        let e = get(&chat,"/api/me", "").await.unwrap_err();
        assert_eq!((e.status,e.code.as_str(),e.retry_after_seconds),(status,code,retry));
    }
}

#[tokio::test]
async fn rest_passes_raw_upload_and_maps_chat_results() {
    use axum::{body::{to_bytes, Body}, http::Request};
    use tower::ServiceExt;
    use crate::rest::{self, RestConfig};

    let (chat, _) = setup(vec![
        ok(json!({"@type":"sendMessage","chat_id":5,"input_message_content":{"@type":"inputMessageAudio",
            "audio":{"@type":"inputAudio","audio":{"@type":"inputFileLocal","path":"sample.mp3","bytes":"raw audio"}},
            "caption":{"@type":"formattedText","text":"bản thử","entities":[]}}}),td_message(31)),
    ]);
    let app = rest::router(Arc::new(rest::ChuaCo), Arc::new(chat), RestConfig {
        token:"test-token".into(),rate_limit_per_minute:120,version:"test".into(),
    });
    for (method, path, body, status, expected) in [
        ("POST","/api/chats/5/send_file?kind=audio&name=sample.mp3&caption=b%E1%BA%A3n+th%E1%BB%AD&account=%2B000123",
            "raw audio",200,normalized(31,"")),
        ("GET","/api/me?account=wrong","",404,json!({"error":{"code":"NOT_FOUND","message":"Không rõ tài khoản."}})),
    ] {
        let request = Request::builder().method(method).uri(path).header("Authorization","Bearer test-token").body(Body::from(body)).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(),status);
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),expected);
    }
}

#[tokio::test]
async fn downloads_are_bounded_private_and_never_follow_symlinks() {
    use axum::{body::{to_bytes, Body}, http::{header, Request}};
    use std::{fs, os::unix::fs::symlink};
    use tower::ServiceExt;
    use crate::rest::{self, RestConfig};

    let root = PathBuf::from("target").join(format!("file-test-{}", std::process::id()));
    let data = root.join("data");
    let media = data.join("tdlib/files/photos");
    fs::create_dir_all(&media).unwrap();
    let valid = media.join("photo.jpg");
    fs::write(&valid, b"fake JPEG").unwrap();
    let text = media.join("note.txt");
    fs::write(&text, "ghi chú").unwrap();
    let large = media.join("large.jpg");
    fs::File::create(&large).unwrap().set_len(files::MAX_FILE_BYTES + 1).unwrap();
    let outside = root.join("outside.jpg");
    fs::write(&outside, b"private").unwrap();
    let link = media.join("link.jpg");
    symlink(std::path::absolute(&outside).unwrap(), &link).unwrap();
    let escape = media.join("../../../../outside.jpg");
    let traces = data.join("calltrace");
    fs::create_dir(&traces).unwrap();
    fs::write(traces.join("abcdef.log"), b"trace").unwrap();
    let req = |id| json!({"@type":"downloadFile","file_id":id,"priority":16,"offset":0,"limit":0,"synchronous":true});
    let downloaded = |id, path: &Path| ok(req(id), json!({"local":{"path":path,"is_downloading_completed":true}}));
    let (chat, td) = setup(vec![downloaded(1, &valid), err(req(2), 404, "/private/backend/path"),
        downloaded(3, &large), downloaded(4, &link), downloaded(5, &outside), downloaded(6, &escape),
        downloaded(7, &text)]);
    drop(chat);
    let chat = ChatService::new(td, "+000123", &data);
    let app = rest::router(Arc::new(rest::ChuaCo), Arc::new(chat), RestConfig {
        token:"test-token".into(), rate_limit_per_minute:120,version:"test".into(),
    });
    let get = |uri| app.clone().oneshot(Request::builder().uri(uri)
        .header(header::AUTHORIZATION, "Bearer test-token").body(Body::empty()).unwrap());
    let response = get("/api/file/1").await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "9");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "private, max-age=86400");
    assert!(!response.headers().contains_key(header::CONTENT_DISPOSITION));
    assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "fake JPEG");
    for (uri, status) in [("/api/file/2",404),("/api/file/3",413),("/api/file/4",404),
        ("/api/file/5",404),("/api/file/6",404),("/api/file/-1",404),("/api/file/xyz",422)] {
        let response = get(uri).await.unwrap();
        assert_eq!(response.status(), status, "{uri}");
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let body = std::str::from_utf8(&bytes).unwrap();
        assert!(!body.contains("private") && !body.contains("file-test-"), "{body}");
    }
    let response = get("/api/file/7").await.unwrap();
    assert_eq!(response.status(), 200);
    // The security policy now downloads all non-media content, including text and traces.
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/octet-stream");
    assert_eq!(response.headers()[header::CONTENT_DISPOSITION], "attachment");
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "8");
    assert_eq!(to_bytes(response.into_body(), 100).await.unwrap().as_ref(), "ghi chú".as_bytes());
    let response = get("/api/calltrace/abcdef").await.unwrap();
    let listing = get("/api/calltrace").await.unwrap();
    assert_eq!(listing.status(), 200);
    let listing: Value = serde_json::from_slice(&to_bytes(listing.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(listing["ok"], true);
    assert_eq!(listing["keep"], 3);
    assert_eq!(listing["calls"][0]["id"], "abcdef");
    assert_eq!(listing["calls"][0]["live"], false);
    assert_eq!(response.headers()[header::CONTENT_DISPOSITION], "attachment; filename=\"calltrace-abcdef.log\"");
    assert_eq!(to_bytes(response.into_body(), 100).await.unwrap(), "trace");
    assert_eq!(get("/api/calltrace/ABCDEF").await.unwrap().status(), 404);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn message_entities_preserve_utf16_offsets_and_keep_only_supported_ranges() {
    let entity = |kind, offset, length| json!({"offset":offset,"length":length,"type":{"@type":kind}});
    let mut link = entity("textEntityTypeTextUrl", 10, 4);
    link["type"]["url"] = json!("https://example.com/");
    let mut pre = entity("textEntityTypePreCode", 20, 3);
    pre["type"]["language"] = json!("sh");
    let mut mention = entity("textEntityTypeMentionName", 24, 2);
    mention["type"]["user_id"] = json!(7);
    let mut entities = vec![entity("textEntityTypeUnknown", 0, 2), entity("textEntityTypeItalic", -1, 1),
        entity("textEntityTypeBold", 3, 6), link, entity("textEntityTypeCode", 15, 4), pre, mention];
    entities.extend((27..123).map(|offset| entity("textEntityTypeUnderline", offset, 1)));
    let text = format!("🌙 Header link code run An {}", "x".repeat(96));
    let td = json!({"content":{"@type":"messageText","text":{"text":text,"entities":entities}}});
    let result = message(&td, "An");
    let actual = result["entities"].as_array().expect("message entities");
    assert_eq!(result["text"], text);
    assert_eq!(actual.len(), 100);
    assert_eq!(&actual[..5], json!([
        {"type":"bold","offset":3,"length":6},
        {"type":"text_link","offset":10,"length":4,"url":"https://example.com/"},
        {"type":"code","offset":15,"length":4},
        {"type":"pre","offset":20,"length":3,"language":"sh"},
        {"type":"text_mention","offset":24,"length":2,"user_id":7}
    ]).as_array().expect("expected list").as_slice());
    assert_eq!(actual.last(), Some(&json!({"type":"underline","offset":121,"length":1})));

    let caption = json!({"content":{"@type":"messagePhoto","caption":{"text":"Ảnh","entities":[entity("textEntityTypeItalic", 0, 3)]}}});
    assert_eq!(message(&caption, "")["entities"], json!([{"type":"italic","offset":0,"length":3}]));
}

#[test]
fn media_and_reactions_have_python_message_shapes() {
    let photo = json!({"id":1,"date":100,"is_outgoing":true,"reply_to":{"message_id":12},"content":{"@type":"messagePhoto","caption":{"text":" cap "},
        "photo":{"sizes":[{"width":90,"photo":{"id":11}},
        {"width":800,"height":600,"photo":{"id":12,"size":0,"expected_size":123,"local":{"is_downloading_completed":true,"path":"x.jpg"}}},
        {"width":2000,"photo":{"id":13}}]}},"interaction_info":{"reactions":{"reactions":[{"type":{"emoji":"👍"},"total_count":2,"is_chosen":true}]}}});
    let m = message(&photo,"An");
    assert_eq!(m,json!({"id":1,"chat_id":null,"date":100,"edit_date":0,"outgoing":true,"sender":"An","kind":"photo","text":" cap ",
        "entities":[],"summary":"🖼 Ảnh: cap","reply_to":12,"reactions":[{"emoji":"👍","count":2,"mine":true}],
        "file":{"id":12,"size":123,"path":"x.jpg","width":800,"height":600,"mime":"image/jpeg"}}));
    let voice = message(&json!({"content":{"@type":"messageVoiceNote","voice_note":{"duration":4,"voice":{"id":21}}}}),"");
    assert_eq!(voice["file"],json!({"id":21,"size":0,"path":"","name":"","mime":"","duration":4,"width":0,"height":0}));
    let doc = message(&json!({"content":{"@type":"messageDocument","document":{"file_name":"a.pdf","mime_type":"application/pdf","document":{"id":31,"size":10}}}}),"");
    assert_eq!(doc["file"],json!({"id":31,"size":10,"path":"","name":"a.pdf","mime":"application/pdf","duration":0,"width":0,"height":0}));
    assert_eq!(doc["summary"],"📎 a.pdf");
}

#[test]
fn phone_masking_hides_middle_digits() {
    assert_eq!(mask_phone(""), "");
    assert_eq!(mask_phone("1234"), "+••••");
    assert_eq!(mask_phone("12345"), "+12•45");
    assert_eq!(mask_phone("+84900000000"), "+849•••••000");
    assert_eq!(mask_phone("+84 912 345 678"), "+849•••••678");
}

#[tokio::test]
async fn profile_get_and_update_require_confirmation_and_validate_before_tdlib() {
    let me_before = json!({
        "id":7, "first_name":"Bình", "last_name":"Lê", "phone_number":"84900000000",
        "usernames":{"editable_username":"binh_le","active_usernames":["binh_le"]},
        "profile_photo":{"small":{"id":91}}
    });
    let full_before = json!({"@type":"userFullInfo","bio":{"text":"Xin chào"}});
    let me_after = json!({
        "id":7, "first_name":"An", "last_name":"Trần", "phone_number":"84900000000",
        "usernames":{"editable_username":"an_tran","active_usernames":["an_tran"]},
        "profile_photo":{"small":{"id":91}}
    });
    let full_after = json!({"@type":"userFullInfo","bio":{"text":"Tiểu sử mới"}});
    let (chat, _) = setup(vec![
        ok(json!({"@type":"getMe"}), me_before.clone()),
        ok(json!({"@type":"getUserFullInfo","user_id":7}), full_before.clone()),
        ok(json!({"@type":"setName","first_name":"An","last_name":"Trần"}), json!({"@type":"ok"})),
        ok(json!({"@type":"setBio","bio":"Tiểu sử mới"}), json!({"@type":"ok"})),
        ok(json!({"@type":"setUsername","username":"an_tran"}), json!({"@type":"ok"})),
        ok(json!({"@type":"getMe"}), me_after.clone()),
        ok(json!({"@type":"getUserFullInfo","user_id":7}), full_after),
        ok(json!({"@type":"getMe"}), me_after.clone()),
        ok(json!({"@type":"setName","first_name":"An Nhi","last_name":"Trần"}), json!({"@type":"ok"})),
        ok(json!({"@type":"setUsername","username":""}), json!({"@type":"ok"})),
        ok(json!({"@type":"getMe"}), json!({"id":7,"first_name":"An Nhi","last_name":"Trần","phone_number":"84900000000"})),
        ok(json!({"@type":"getUserFullInfo","user_id":7}), json!({"@type":"userFullInfo","bio":{"text":""}})),
    ]);

    let profile = get(&chat, "/api/profile", "").await.unwrap();
    assert_eq!(profile, json!({
        "id":7, "name":"Bình Lê", "first_name":"Bình", "last_name":"Lê",
        "bio":"Xin chào", "username":"binh_le", "phone_masked":"+849•••••000", "avatar_file":91
    }));
    assert!(!profile.to_string().contains("84900000000"));

    let updated = post(&chat, "/api/profile", "", json!({
        "confirm":true, "first_name":" An ", "last_name":" Trần ", "bio":" Tiểu sử mới ", "username":"@an_tran"
    })).await.unwrap();
    assert_eq!(updated["name"], "An Trần");
    assert_eq!(updated["bio"], "Tiểu sử mới");
    assert_eq!(updated["username"], "an_tran");

    let partial = post(&chat, "/api/profile", "", json!({
        "confirm":true, "first_name":"An Nhi", "username":""
    })).await.unwrap();
    assert_eq!(partial["first_name"], "An Nhi");
    assert_eq!(partial["last_name"], "Trần");
    assert_eq!(partial["username"], "");
}

#[tokio::test]
async fn invalid_profile_updates_fail_without_calling_tdlib() {
    let (chat, _) = setup(vec![]);
    for bad in [
        json!({"first_name":"An"}),
        json!({"confirm":false,"first_name":"An"}),
        json!({"confirm":"true","first_name":"An"}),
        json!({"confirm":true}),
        json!({"confirm":true,"extra":"x","first_name":"An"}),
        json!({"confirm":true,"first_name":"   "}),
        json!({"confirm":true,"first_name":"a".repeat(65)}),
        json!({"confirm":true,"last_name":"b".repeat(65)}),
        json!({"confirm":true,"bio":"c".repeat(71)}),
        json!({"confirm":true,"username":"ab"}),
        json!({"confirm":true,"username":"1abcde"}),
        json!({"confirm":true,"username":"abcde_"}),
        json!({"confirm":true,"username":"ab__cd"}),
        json!({"confirm":true,"username":"bad-name"}),
    ] {
        let err = post(&chat, "/api/profile", "", bad.clone()).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_REQUEST"), "{bad}");
    }
}

#[test]
fn message_call_table_formats_direction_video_reason_duration_and_missed_flag() {
    let cases = [
        (false, true, "callDiscardReasonHungUp", 18, "Cuộc gọi video đến (18 giây)", "Cuộc gọi video đến", false, "incoming", "18 giây"),
        (true, false, "callDiscardReasonHungUp", 125, "Cuộc gọi đi (2 phút 5 giây)", "Cuộc gọi đi", false, "outgoing", "2 phút 5 giây"),
        (false, false, "callDiscardReasonMissed", 0, "Cuộc gọi nhỡ", "Cuộc gọi nhỡ", true, "missed", ""),
        (false, true, "callDiscardReasonMissed", 0, "Cuộc gọi video nhỡ", "Cuộc gọi video nhỡ", true, "missed", ""),
        (false, false, "callDiscardReasonDeclined", 0, "Cuộc gọi bị từ chối", "Cuộc gọi bị từ chối", false, "declined", ""),
        (true, true, "callDiscardReasonDeclined", 0, "Cuộc gọi video bị từ chối", "Cuộc gọi video bị từ chối", false, "declined", ""),
        (false, false, "callDiscardReasonDisconnected", 3665, "Cuộc gọi đến (1 giờ 1 phút)", "Cuộc gọi đến", false, "incoming", "1 giờ 1 phút"),
        (true, false, "callDiscardReasonEmpty", 0, "Cuộc gọi đi", "Cuộc gọi đi", false, "outgoing", ""),
    ];
    for (outgoing, is_video, discard_ty, duration, want_summary, want_label, want_missed, want_status, want_dur) in cases {
        let msg = json!({
            "id": 10,
            "chat_id": 42,
            "date": 1700000000,
            "is_outgoing": outgoing,
            "content": {
                "@type": "messageCall",
                "is_video": is_video,
                "duration": duration,
                "discard_reason": {"@type": discard_ty}
            }
        });
        let out = message(&msg, "Đại ca");
        assert_eq!(out["kind"], "call");
        assert_eq!(out["summary"], want_summary);
        assert_eq!(out["call"]["is_video"], is_video);
        assert_eq!(out["call"]["outgoing"], outgoing);
        assert_eq!(out["call"]["duration"], duration);
        assert_eq!(out["call"]["missed"], want_missed);
        assert_eq!(out["call"]["status"], want_status);
        assert_eq!(out["call"]["label"], want_label);
        assert_eq!(out["call"]["duration_text"], want_dur);
    }
}

#[tokio::test]
async fn system_messages_use_real_names_never_numeric_ids_and_handle_missing_users() {
    assert_eq!(user_name(&json!({"id": 987654321})), "Người dùng");
    let u_binh = json!({"id": 7, "first_name": "Bình", "last_name": "Lê"});
    let u_ngoc = json!({"id": 88, "first_name": "Anh Tuan Chị Ngọc"});
    let u_daica = json!({"id": 77, "first_name": "Đại ca"});
    let msgs = vec![
        json!({"id": 1, "chat_id": -5, "date": 100, "is_outgoing": true, "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageChatSetMessageAutoDeleteTime", "message_auto_delete_time": 86400, "from_user_id": 7}}),
        json!({"id": 2, "chat_id": -5, "date": 101, "is_outgoing": false, "sender_id": {"@type": "messageSenderUser", "user_id": 77},
            "content": {"@type": "messageChatSetMessageAutoDeleteTime", "message_auto_delete_time": 604800, "from_user_id": 77}}),
        json!({"id": 3, "chat_id": -5, "date": 102, "is_outgoing": true, "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageChatSetMessageAutoDeleteTime", "message_auto_delete_time": 0, "from_user_id": 7}}),
        json!({"id": 4, "chat_id": -5, "date": 103, "is_outgoing": false, "sender_id": {"@type": "messageSenderUser", "user_id": 88},
            "content": {"@type": "messageContactRegistered"}}),
        json!({"id": 5, "chat_id": -5, "date": 104, "is_outgoing": false, "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageChatAddMembers", "member_user_ids": [7]}}),
        json!({"id": 6, "chat_id": -5, "date": 105, "is_outgoing": false, "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageChatAddMembers", "member_user_ids": [88, 999999]}}),
        json!({"id": 7, "chat_id": -5, "date": 106, "is_outgoing": false, "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageChatDeleteMember", "user_id": 7}}),
    ];
    let (chat, _) = setup(vec![
        ok(json!({"@type":"getChatHistory","chat_id":-5,"from_message_id":0,"offset":0,"limit":10,"only_local":false}), json!({"messages": msgs})),
        ok(json!({"@type":"getChatHistory","chat_id":-5,"from_message_id":7,"offset":0,"limit":3,"only_local":false}), json!({"messages": []})),
        ok(json!({"@type":"getUser","user_id":7}), u_binh),
        ok(json!({"@type":"getUser","user_id":77}), u_daica),
        ok(json!({"@type":"getUser","user_id":88}), u_ngoc),
        err(json!({"@type":"getUser","user_id":999999}), 404, "User not found"),
    ]);
    let res = get(&chat, "/api/chats/-5/messages", "limit=10").await.unwrap();
    let list = res["messages"].as_array().unwrap();
    assert_eq!(list[0]["kind"], "service");
    assert_eq!(list[0]["summary"], "Bạn đã đặt hẹn giờ tự hủy cho mọi cuộc trò chuyện sau 1 ngày");
    assert_eq!(list[1]["summary"], "Đại ca đã đặt hẹn giờ tự hủy cho mọi cuộc trò chuyện sau 1 tuần");
    assert_eq!(list[2]["summary"], "Bạn đã tắt hẹn giờ tự hủy tin nhắn");
    assert_eq!(list[3]["summary"], "Anh Tuan Chị Ngọc đã tham gia Telegram");
    assert_eq!(list[4]["summary"], "Bình Lê đã tham gia nhóm");
    assert_eq!(list[5]["summary"], "Bình Lê đã thêm Anh Tuan Chị Ngọc, Người dùng");
    assert!(!list[5]["summary"].as_str().unwrap().contains("999999"));
    assert_eq!(list[6]["summary"], "Bình Lê đã rời nhóm");
}

#[tokio::test]
async fn chat_list_computes_effective_mute_from_scope_default_verified_online_read_ticks_and_thumbnails() {
    let list = json!({"@type":"chatListMain"});
    let c1 = json!({
        "id": 42, "title": "Telegram", "type": {"@type": "chatTypePrivate", "user_id": 777},
        "positions": [], "unread_count": 0, "last_read_outbox_message_id": 500,
        "notification_settings": {"@type":"chatNotificationSettings","use_default_mute_for":true,"mute_for":0},
        "last_message": {"id": 500, "date": 100, "is_outgoing": true, "content": {"@type":"messageText","text":{"text":"Ok"}}}
    });
    let c2 = json!({
        "id": 43, "title": "Bạn thân", "type": {"@type": "chatTypePrivate", "user_id": 778},
        "positions": [], "unread_count": 2,
        "notification_settings": {"@type":"chatNotificationSettings","use_default_mute_for":false,"mute_for":0},
        "last_message": {"id": 501, "date": 101, "is_outgoing": false, "content": {"@type":"messageCall","is_video":true,"duration":18,"discard_reason":{"@type":"callDiscardReasonHungUp"}}}
    });
    let c3 = json!({
        "id": -1001, "title": "星驰公益", "type": {"@type": "chatTypeSupergroup", "supergroup_id": 1001, "is_channel": false},
        "positions": [{"list":{"@type":"chatListMain"},"is_pinned":true}], "unread_count": 14682, "unread_mention_count": 3,
        "notification_settings": {"@type":"chatNotificationSettings","use_default_mute_for":false,"mute_for":2147483647},
        "last_message": {"id": 600, "date": 102, "is_outgoing": false, "sender_id": {"@type":"messageSenderUser","user_id": 88},
            "content": {"@type":"messageAnimation","caption":{"text":"Phượt đêm, chỉ 1 giây mất l…"},"animation":{"thumbnail":{"file":{"id":777}}}}}
    });
    let (chat, td) = setup(vec![
        ok(json!({"@type":"loadChats","chat_list":list,"limit":60}), json!({"@type":"ok"})),
        ok(json!({"@type":"getChats","chat_list":list,"limit":60}), json!({"chat_ids":[42, 43, -1001]})),
        ok(json!({"@type":"getChat","chat_id":42}), c1),
        ok(json!({"@type":"getChat","chat_id":43}), c2),
        ok(json!({"@type":"getChat","chat_id":-1001}), c3),
    ]);
    td.updates.send(json!({"@type":"updateUser","user":{"id":777,"first_name":"Telegram","is_verified":true,"status":{"@type":"userStatusOnline"}}})).unwrap();
    td.updates.send(json!({"@type":"updateUser","user":{"id":88,"first_name":"星驰管家"}})).unwrap();
    td.updates.send(json!({"@type":"updateSupergroup","supergroup":{"id":1001,"member_count":2500,"is_verified":false}})).unwrap();
    td.updates.send(json!({"@type":"updateScopeNotificationSettings","scope":{"@type":"notificationSettingsScopePrivateChats"},"notification_settings":{"mute_for":2147483647}})).unwrap();
    let _ = get(&chat, "/api/feed", "since=0&wait=1").await.unwrap();

    let res = get(&chat, "/api/chats", "").await.unwrap();
    let arr = res["chats"].as_array().unwrap();
    assert_eq!(arr[0]["muted"], true);
    assert_eq!(arr[0]["verified"], true);
    assert_eq!(arr[0]["online"], true);
    assert_eq!(arr[0]["last"]["read"], true);

    assert_eq!(arr[1]["muted"], false);
    assert_eq!(arr[1]["last"]["kind"], "call");
    assert_eq!(arr[1]["last"]["text"], "Cuộc gọi video đến (18 giây)");
    assert_eq!(arr[1]["last"]["call"]["missed"], false);

    assert_eq!(arr[2]["unread"], 14682);
    assert_eq!(arr[2]["unread_mentions"], 3);
    assert_eq!(arr[2]["member_count"], 2500);
    assert_eq!(arr[2]["last"]["sender"], "星驰管家");
    assert_eq!(arr[2]["last"]["thumb_file"], 777);
    assert_eq!(arr[2]["last"]["caption"], "Phượt đêm, chỉ 1 giây mất l…");
}

#[tokio::test]
async fn chat_history_supports_in_chat_search_and_validates_query() {
    let long_q = "a".repeat(129);
    let (chat, _) = setup(vec![
        ok(json!({
            "@type": "searchChatMessages",
            "chat_id": 5,
            "query": "xin chào",
            "from_message_id": 0,
            "offset": 0,
            "limit": 20,
            "filter": Value::Null
        }), json!({"total_count": 1, "messages": [td_message(99)]})),
        ok(json!({"@type":"getUser","user_id":7}), json!({"id":7,"first_name":"An"})),
    ]);

    let err = get(&chat, "/api/chats/5/messages", &format!("q={long_q}")).await.unwrap_err();
    assert_eq!((err.status, err.code.as_str()), (400, "BAD_REQUEST"));

    let found = get(&chat, "/api/chats/5/messages", "q=+xin+ch%C3%A0o+&limit=20").await.unwrap();
    assert_eq!(found["total_count"], 1);
    assert_eq!(found["messages"], json!([normalized(99, "An")]));
}
