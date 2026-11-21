use super::*;

fn get_user(id: i64) -> Value { json!({"@type":"getUser","user_id":id}) }
fn list(ids: &[i64]) -> FakeReply {
    ok(json!({"@type":"getContacts"}), json!({"user_ids":ids}))
}
fn blocked_list() -> Value {
    json!({"@type":"getBlockedMessageSenders","block_list":{"@type":"blockListMain"},"offset":0,"limit":100})
}
fn success(req: Value) -> FakeReply { ok(req, json!({"@type":"ok"})) }
fn sender(id: i64) -> Value { json!({"@type":"messageSenderUser","user_id":id}) }

#[tokio::test]
async fn contacts_last_seen_orders_online_timestamps_and_unknown() {
    let (chat, _) = setup(vec![list(&[9,8,7,6,5,4,3]),
        ok(get_user(9), json!({"id":9,"status":{"@type":"userStatusOffline","was_online":1000}})),
        ok(get_user(8), json!({"id":8,"status":{"@type":"userStatusEmpty"}})),
        ok(get_user(7), json!({"id":7,"status":{"@type":"userStatusOffline","was_online":2000},"profile_photo":{"small":{"id":15}},"is_mutual_contact":true})),
        ok(get_user(6), json!({"id":6,"status":{"@type":"userStatusOnline","expires":2147483647}})),
        ok(get_user(5), json!({"id":5,"status":{"@type":"userStatusOffline","was_online":2000}})),
        ok(get_user(4), json!({"id":4})),
        ok(get_user(3), json!({"id":3,"status":{"@type":"userStatusLastWeek"}})),
        ok(blocked_list(), json!({"senders":[sender(7)]})),
    ]);
    let value = get(&chat, "/api/contacts", "").await.unwrap();
    let rows = value["contacts"].as_array().unwrap();
    assert_eq!(rows.iter().map(|r| r["id"].as_i64().unwrap()).collect::<Vec<_>>(), [6,5,7,9,3,4,8]);
    assert_eq!(rows[0]["status"]["kind"], "online");
    assert_eq!(rows[2]["status"]["last_seen"], 2000);
    assert_eq!(rows[2]["avatar_file"], 15);
    assert_eq!(rows[2]["is_mutual"], true);
    assert_eq!(rows[2]["blocked"], true);
    assert_eq!(rows[4]["status"]["kind"], "week");
    assert_eq!(rows[6]["status"]["kind"], "hidden");
}

#[tokio::test]
async fn contacts_name_folds_accents_and_sections_and_handles_empty_names() {
    let (chat, _) = setup(vec![list(&[6,5,4,3,2,1]),
        ok(get_user(6), json!({"id":6,"first_name":"☆☆"})),
        ok(get_user(5), json!({"id":5,"first_name":"Bình"})),
        ok(get_user(4), json!({"id":4,"first_name":"anh"})),
        ok(get_user(3), json!({"id":3,"first_name":"Ánh"})),
        ok(get_user(2), json!({"id":2,"first_name":"!Đào"})),
        ok(get_user(1), json!({"id":1,"first_name":""})),
        ok(blocked_list(), json!({"senders":[]})),
    ]);
    let value = get(&chat, "/api/contacts", "sort=name").await.unwrap();
    let rows = value["contacts"].as_array().unwrap();
    let names: Vec<_> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(&names[..4], ["Ánh","anh","Bình","!Đào"]);
    assert!(names[4..].contains(&"1") && names[4..].contains(&"☆☆"));
    assert_eq!(rows.iter().map(|r| r["section"].as_str().unwrap()).collect::<Vec<_>>(), ["A","A","B","D","#","#"]);
    assert_eq!(get(&chat, "/api/contacts", "sort=invalid").await.unwrap_err().status, 400);
}

const TEST_PHONE: &str = "+84947000000";

fn contact(phone: &str, first: &str, last: &str) -> Value {
    json!({"@type":"importedContact","phone_number":phone,"first_name":first,"last_name":last})
}

fn import_phone() -> Value { json!({"@type":"importContacts","contacts":[contact(TEST_PHONE,"An", "")]}) }

#[tokio::test]
async fn contacts_add_validates_normalizes_and_distinguishes_unregistered_numbers() {
    let (chat, _) = setup(vec![
        ok(import_phone(), json!({"user_ids":[0]})),
        ok(import_phone(), json!({"user_ids":[7]})),
        success(json!({"@type":"addContact","user_id":7,"contact":contact(TEST_PHONE,"An",""),"share_phone_number":true})),
        ok(get_user(7), json!({"id":7,"first_name":"An"})),
        err(import_phone(), 500, "Unexpected import failure"),
    ]);
    for phone in ["", "badnumber", "++84947000000", "+0000000"] {
        let error = post(&chat,"/api/contacts/add","",json!({"first_name":"An","phone":phone})).await.unwrap_err();
        assert_eq!(error.status, 422);
    }
    let missing = post(&chat,"/api/contacts/add","",json!({"first_name":"An","phone":"+84 947-000.000"})).await.unwrap_err();
    assert_eq!((missing.status, missing.code.as_str()), (404, "not_telegram_user"));
    let added = post(&chat,"/api/contacts/add","",json!({"first_name":"An","phone":"0947 (000) 000","share_my_phone":true})).await.unwrap();
    assert_eq!(added["contact"]["id"], 7);
    let unusual = post(&chat,"/api/contacts/add","",json!({"first_name":"An","phone":TEST_PHONE})).await.unwrap_err();
    assert_eq!((unusual.status, unusual.code.as_str()), (502, "MEDIA_ENGINE_ERROR"));
}

#[tokio::test]
async fn contacts_edit_remove_block_and_unblock_send_versioned_requests() {
    let (chat, _) = setup(vec![
        ok(get_user(7), json!({"id":7,"is_contact":true,"phone_number":"84947000000"})),
        success(json!({"@type":"addContact","user_id":7,"contact":contact("84947000000","An","Bình"),"share_phone_number":false})),
        success(json!({"@type":"removeContacts","user_ids":[7,8]})),
        ok(json!({"@type":"createPrivateChat","user_id":7,"force":false}), json!({"id":9})),
        ok(json!({"@type":"reportChat","chat_id":9,"option_id":"","message_ids":[],"text":""}), json!({"@type":"reportChatResultOk"})),
        success(json!({"@type":"setMessageSenderBlockList","sender_id":sender(7),"block_list":{"@type":"blockListMain"}})),
        success(json!({"@type":"deleteChatHistory","chat_id":9,"remove_from_chat_list":true,"revoke":false})),
        ok(blocked_list(), json!({"senders":[sender(7), {"@type":"messageSenderChat","chat_id":42}]})),
        ok(get_user(7), json!({"id":7,"first_name":"An"})),
        success(json!({"@type":"setMessageSenderBlockList","sender_id":sender(7),"block_list":null})),
    ]);
    assert_eq!(post(&chat,"/api/contacts/edit","",json!({"user_id":7,"first_name":"An","last_name":"Bình"})).await.unwrap()["edited"],true);
    assert_eq!(post(&chat,"/api/contacts/remove","",json!({"user_ids":(1..=101).collect::<Vec<_>>()})).await.unwrap_err().status,400);
    assert_eq!(post(&chat,"/api/contacts/remove","",json!({"user_ids":[7,8]})).await.unwrap()["removed"],true);
    assert_eq!(post(&chat,"/api/contacts/block","",json!({"user_id":7,"report_spam":true,"delete_chat":true})).await.unwrap()["blocked"],true);
    let blocked = get(&chat,"/api/contacts/blocked", "").await.unwrap();
    assert_eq!(blocked["contacts"].as_array().unwrap().len(),1);
    assert_eq!(blocked["contacts"][0]["blocked"],true);
    assert_eq!(post(&chat,"/api/contacts/unblock","",json!({"user_id":7})).await.unwrap()["blocked"],false);
}

#[tokio::test]
async fn contacts_http_exposes_routes_and_structured_errors() {
    use axum::{body::{to_bytes, Body}, http::Request};
    use tower::ServiceExt;
    use crate::rest::{self, RestConfig};

    let (chat, _) = setup(vec![ok(blocked_list(), json!({"senders":[]}))]);
    let app = rest::router(Arc::new(rest::ChuaCo), Arc::new(chat), RestConfig {
        token: "test-token".into(), rate_limit_per_minute: 120, version: "test".into(),
    });
    let add = Request::post("/api/contacts/add").header("Authorization", "Bearer test-token")
        .body(Body::from(r#"{"first_name":"An","phone":""}"#)).unwrap();
    let response = app.clone().oneshot(add).await.unwrap();
    assert_eq!(response.status(), 422);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_PHONE");
    assert!(error["error"]["message"].is_string());
    let blocked = Request::get("/api/contacts/blocked").header("Authorization", "Bearer test-token")
        .body(Body::empty()).unwrap();
    let response = app.oneshot(blocked).await.unwrap();
    assert_eq!(response.status(), 200);
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!({"contacts":[]}));
}

thread_local! { static LOG_LINES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) }; }

struct CaptureLog;
impl log::Log for CaptureLog {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool { true }
    fn log(&self, record: &log::Record<'_>) { LOG_LINES.with(|lines| lines.borrow_mut().push(record.args().to_string())); }
    fn flush(&self) {}
}

#[tokio::test]
async fn contacts_phone_is_not_logged_or_echoed_by_tdlib_errors() {
    log::set_logger(&CaptureLog).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    log::debug!("contact log fixture is active");
    let (chat, _) = setup(vec![err(import_phone(),500,&format!("Unexpected input {TEST_PHONE}"))]);
    let error = post(&chat,"/api/contacts/add","",json!({"first_name":"An","phone":"0947 000 000"})).await.unwrap_err();
    assert!(!error.message.contains("84947000000"));
    LOG_LINES.with(|lines| {
        let lines = lines.borrow();
        assert_eq!(lines.first().map(String::as_str), Some("contact log fixture is active"));
        assert!(!lines.iter().any(|line| ["84947000000", "0947 000 000", "0947000000"].iter().any(|phone| line.contains(phone))), "log chứa số điện thoại");
    });
}
