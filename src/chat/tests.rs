use std::{collections::VecDeque, sync::Mutex};
use tokio::sync::broadcast;

use super::*;
use crate::contract::TdError;

type FakeReply = (Value, Result<Value, TdError>);

mod catalog;
mod writing;

struct FakeTd {
    calls: Mutex<VecDeque<FakeReply>>,
    updates: broadcast::Sender<Value>,
}

impl TdLink for FakeTd {
    fn request(&self, req: Value) -> BoxFut<'_, Result<Value, TdError>> {
        Box::pin(async move {
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
    let td = Arc::new(FakeTd { calls: Mutex::new(calls.into()), updates: broadcast::channel(64).0 });
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
        "text":" Chào ","summary":"Chào","reply_to":0,"reactions":[],"file":null})
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
        "muted":true,"last":{"date":0,"outgoing":false,"kind":"text","text":"Chào"},"avatar_file":10,"user_id":null}]});
    assert_eq!(get(&chat,"/api/chats","limit=999").await.unwrap(),expected);
    assert_eq!(get(&chat,"/api/search","q=+nh%C3%B3m+a+").await.unwrap(),expected);
    assert_eq!(get(&chat,"/api/search","q=+").await.unwrap(),json!({"chats":[]}));
}

#[tokio::test]
async fn history_paginates_sorts_and_reuses_sender_for_contacts() {
    let req = |from, limit| json!({"@type":"getChatHistory","chat_id":5,"from_message_id":from,"offset":0,"limit":limit,"only_local":false});
    let (chat, _) = setup(vec![ok(req(11,3),json!({"messages":[td_message(10)]})),ok(req(10,2),json!({"messages":[td_message(8)]})),
        ok(req(8,1),json!({"messages":[]})),ok(json!({"@type":"getUser","user_id":7}),json!({"id":7,"first_name":"Zed"})),
        ok(json!({"@type":"getContacts"}),json!({"user_ids":[7,8]})),
        ok(json!({"@type":"getUser","user_id":8}),json!({"id":8,"usernames":{"active_usernames":["amy"]}})),
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}),json!({"id":5})),
    ]);
    assert_eq!(get(&chat,"/api/chats/5/messages","before=11&limit=3").await.unwrap(),json!({"messages":[normalized(8,"Zed"),normalized(10,"Zed")]}));
    assert_eq!(get(&chat,"/api/contacts", "").await.unwrap(),json!({"contacts":[{"id":8,"name":"@amy","phone":"","username":"amy"},
        {"id":7,"name":"Zed","phone":"","username":""}]}));
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
async fn invalid_arguments_unknown_account_and_binary_routes_never_call_tdlib() {
    let (chat, _) = setup(vec![]);
    for (path, query, status, code) in [
        ("/api/chats", "limit=x",400,"BAD_REQUEST"),("/api/chats/5/messages","before=x",400,"BAD_REQUEST"),
        ("/api/feed","wait=x",400,"BAD_REQUEST"),("/api/me","account=wrong",404,"NOT_FOUND"),
        ("/api/unknown","",404,"NOT_FOUND"),("/api/file/2","",501,"NOT_IMPLEMENTED"),
        ("/api/live/view/abc","",501,"NOT_IMPLEMENTED"),("/api/live/snapshot/abc","",501,"NOT_IMPLEMENTED"),
    ] {
        let e = get(&chat,path,query).await.unwrap_err();
        assert_eq!((e.status,e.code.as_str()),(status,code));
    }
    assert_eq!(post(&chat,"/api/contacts/open","",json!({})).await.unwrap_err().status,400);
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
        ("GET","/api/file/31","",501,json!({"error":{"code":"NOT_IMPLEMENTED","message":no_bytes().message}})),
        ("GET","/api/me?account=wrong","",404,json!({"error":{"code":"NOT_FOUND","message":"Không rõ tài khoản."}})),
    ] {
        let request = Request::builder().method(method).uri(path).header("Authorization","Bearer test-token").body(Body::from(body)).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(),status);
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),expected);
    }
}

#[test]
fn media_and_reactions_have_python_message_shapes() {
    let photo = json!({"id":1,"date":100,"is_outgoing":true,"reply_to":{"message_id":12},"content":{"@type":"messagePhoto","caption":{"text":" cap "},
        "photo":{"sizes":[{"width":90,"photo":{"id":11}},
        {"width":800,"height":600,"photo":{"id":12,"size":0,"expected_size":123,"local":{"is_downloading_completed":true,"path":"x.jpg"}}},
        {"width":2000,"photo":{"id":13}}]}},"interaction_info":{"reactions":{"reactions":[{"type":{"emoji":"👍"},"total_count":2,"is_chosen":true}]}}});
    let m = message(&photo,"An");
    assert_eq!(m,json!({"id":1,"chat_id":null,"date":100,"edit_date":0,"outgoing":true,"sender":"An","kind":"photo","text":" cap ",
        "summary":"🖼 Ảnh: cap","reply_to":12,"reactions":[{"emoji":"👍","count":2,"mine":true}],
        "file":{"id":12,"size":123,"path":"x.jpg","width":800,"height":600,"mime":"image/jpeg"}}));
    let voice = message(&json!({"content":{"@type":"messageVoiceNote","voice_note":{"duration":4,"voice":{"id":21}}}}),"");
    assert_eq!(voice["file"],json!({"id":21,"size":0,"path":"","name":"","mime":"","duration":4,"width":0,"height":0}));
    let doc = message(&json!({"content":{"@type":"messageDocument","document":{"file_name":"a.pdf","mime_type":"application/pdf","document":{"id":31,"size":10}}}}),"");
    assert_eq!(doc["file"],json!({"id":31,"size":10,"path":"","name":"a.pdf","mime":"application/pdf","duration":0,"width":0,"height":0}));
    assert_eq!(doc["summary"],"📎 a.pdf");
}
