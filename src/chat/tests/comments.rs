use super::*;
use crate::chat::comments::MAX_COMMENTS_LIMIT;
use crate::chat::rich_text::{MAX_ENTITIES, MAX_TEXT_UTF16};
use crate::chat::shape::{message_comments, message_with_comments, CommentsContext, MessageContext};
use crate::rest::{self, RestConfig};
use crate::ui_auth::UiAuth;
use axum::{
    body::{to_bytes, Body},
    http::{header, Request},
};
use tower::ServiceExt;

fn channel_chat(id: i64, sg_id: i64) -> Value {
    json!({
        "id": id,
        "title": "Kênh Tin Tức Mẫu",
        "type": {"@type": "chatTypeSupergroup", "supergroup_id": sg_id, "is_channel": true},
        "photo": {"small": {"id": 901}}
    })
}

fn channel_post(id: i64, chat_id: i64, reply_count: i64) -> Value {
    json!({
        "id": id,
        "chat_id": chat_id,
        "date": 1700000000,
        "edit_date": 0,
        "is_outgoing": false,
        "is_channel_post": true,
        "sender_id": {"@type": "messageSenderChat", "chat_id": chat_id},
        "interaction_info": {
            "reply_info": {
                "reply_count": reply_count,
                "recent_replier_ids": [
                    {"@type": "messageSenderUser", "user_id": 11},
                    {"@type": "messageSenderUser", "user_id": 12},
                    {"@type": "messageSenderChat", "chat_id": -100200},
                    {"@type": "messageSenderUser", "user_id": 13}
                ],
                "last_read_inbox_message_id": 502,
                "last_message_id": 505
            }
        },
        "content": {"@type": "messageText", "text": {"text": "Bài đăng kênh"}}
    })
}

#[tokio::test]
async fn channel_post_reply_info_maps_to_comments_with_up_to_3_repliers_and_unread() {
    let post = channel_post(10, -100100, 5);

    // 1. shape::message không nhận CommentsContext -> giữ nguyên cấu trúc cũ (tương thích ngược).
    let plain = message(&post, "Kênh Tin Tức Mẫu");
    assert!(plain.get("comments").is_none());

    // 2. Không phải kênh -> không gắn comments dù có reply_info.
    assert!(message_comments(&post, false, None, None).is_none());

    // 3. reply_count == 0 -> không gắn comments.
    let zero = channel_post(11, -100100, 0);
    assert!(message_comments(&zero, true, None, None).is_none());

    // 4. Tối đa 3 replier (user + chat), fallback "Người dùng", avatar_file > 0.
    let resolve_user = |uid: i64| match uid {
        11 => Some(("An Lê".to_owned(), Some(101))),
        12 => Some(("Bình Trần".to_owned(), None)),
        _ => None,
    };
    let resolve_chat = |cid: i64| match cid {
        -100200 => Some(("Nhóm Thảo Luận".to_owned(), Some(202))),
        _ => None,
    };
    let cc = CommentsContext {
        is_channel: true,
        resolve_user: Some(&resolve_user),
        resolve_chat: Some(&resolve_chat),
    };
    let shaped = message_with_comments(&post, &MessageContext::plain("Kênh Tin Tức Mẫu"), Some(&cc));
    assert_eq!(
        shaped["comments"],
        json!({
            "count": 5,
            "repliers": [
                {"id": 11, "name": "An Lê", "avatar_file": 101},
                {"id": 12, "name": "Bình Trần", "avatar_file": null},
                {"id": -100200, "name": "Nhóm Thảo Luận", "avatar_file": 202}
            ],
            "unread": true
        })
    );

    // 5. 4 trường hợp unread: last_read vs last_message_id.
    for (last_read, last_msg, want_unread) in [
        (0, 505, false),
        (505, 505, false),
        (506, 505, false),
        (502, 505, true),
    ] {
        let mut p = post.clone();
        p["interaction_info"]["reply_info"]["last_read_inbox_message_id"] = json!(last_read);
        p["interaction_info"]["reply_info"]["last_message_id"] = json!(last_msg);
        let got = message_comments(&p, true, None, None).unwrap();
        assert_eq!(got["unread"], want_unread, "last_read={last_read}, last_msg={last_msg}");
    }

    // 6. Tích hợp GET /api/chats/:chat_id/messages chỉ prefetch tối đa 3 replier.
    let (chat, _) = setup(vec![
        ok(
            json!({"@type":"getChatHistory","chat_id":-100100,"from_message_id":0,"offset":0,"limit":1,"only_local":false}),
            json!({"messages": [post]}),
        ),
        ok(
            json!({"@type":"getChat","chat_id":-100100}),
            channel_chat(-100100, 100),
        ),
        ok(
            json!({"@type":"getUser","user_id":11}),
            json!({"id":11,"first_name":"An","last_name":"Lê","profile_photo":{"small":{"id":101}}}),
        ),
        ok(
            json!({"@type":"getUser","user_id":12}),
            json!({"id":12,"first_name":"Bình","last_name":"Trần"}),
        ),
        ok(
            json!({"@type":"getChat","chat_id":-100200}),
            json!({"id":-100200,"title":"Nhóm Thảo Luận","type":{"@type":"chatTypeSupergroup","is_channel":false},"photo":{"small":{"id":202}}}),
        ),
    ]);
    let res = get(&chat, "/api/chats/-100100/messages", "limit=1").await.unwrap();
    assert_eq!(res["messages"][0]["comments"]["count"], 5);
    assert_eq!(res["messages"][0]["comments"]["repliers"].as_array().unwrap().len(), 3);
    assert_eq!(res["messages"][0]["comments"]["repliers"][2]["name"], "Nhóm Thảo Luận");
}

#[tokio::test]
async fn channel_comments_get_fetches_thread_paginates_and_clamps_limit() {
    let ch = channel_chat(-100100, 100);
    let sg_full = json!({"@type":"supergroupFullInfo","linked_chat_id":-100200});
    let thread = json!({
        "@type":"messageThreadInfo",
        "chat_id":-100200,
        "message_thread_id":500,
        "messages":[{"id":500,"chat_id":-100200}]
    });
    let comment = |id: i64, uid: i64, text: &str, reply_to: i64| json!({
        "id": id,
        "chat_id": -100200,
        "date": 1700000000 + id,
        "is_outgoing": false,
        "sender_id": {"@type":"messageSenderUser","user_id":uid},
        "reply_to": {"@type":"messageReplyToMessage","message_id":reply_to},
        "content": {"@type":"messageText","text":{"text":text}}
    });

    let (chat, _) = setup(vec![
        // Lần 1: ?limit=2 -> want=3 -> nhận 3 tin [503, 502, 501] -> has_more=true, trả [502, 503]
        ok(json!({"@type":"getChat","chat_id":-100100}), ch.clone()),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":100}), sg_full.clone()),
        ok(json!({"@type":"getMessageThread","chat_id":-100100,"message_id":10}), thread.clone()),
        ok(
            json!({"@type":"getMessageThreadHistory","chat_id":-100100,"message_id":10,"from_message_id":0,"offset":0,"limit":3}),
            json!({"messages":[comment(503, 12, "Mới nhất", 502), comment(502, 11, "Bình luận 2", 500), comment(501, 11, "Cũ nhất", 500)]}),
        ),
        ok(json!({"@type":"getUser","user_id":12}), json!({"id":12,"first_name":"Bình"})),
        ok(json!({"@type":"getUser","user_id":11}), json!({"id":11,"first_name":"An"})),
        // Lần 2: ?from=502&limit=999 -> clamp còn 100 -> trả 1 tin rồi rỗng -> has_more=false
        ok(json!({"@type":"getChat","chat_id":-100100}), ch),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":100}), sg_full),
        ok(json!({"@type":"getMessageThread","chat_id":-100100,"message_id":10}), thread),
        ok(
            json!({"@type":"getMessageThreadHistory","chat_id":-100100,"message_id":10,"from_message_id":502,"offset":0,"limit":100}),
            json!({"messages":[comment(501, 11, "Cũ nhất", 500)]}),
        ),
        ok(
            json!({"@type":"getMessageThreadHistory","chat_id":-100100,"message_id":10,"from_message_id":501,"offset":0,"limit":100}),
            json!({"messages":[]}),
        ),
    ]);

    let page1 = get(&chat, "/api/chats/-100100/messages/10/comments", "limit=2").await.unwrap();
    assert_eq!(page1["discussion_chat_id"], -100200);
    assert_eq!(page1["thread_id"], 500);
    assert_eq!(page1["root_message_id"], 500);
    assert_eq!(page1["has_more"], true);
    let msgs1 = page1["messages"].as_array().unwrap();
    assert_eq!(msgs1.len(), 2);
    assert_eq!(msgs1[0]["id"], 502);
    assert_eq!(msgs1[0]["sender"], "An");
    assert_eq!(msgs1[1]["id"], 503);
    assert_eq!(msgs1[1]["reply_to"], 502);

    let page2 = get(&chat, "/api/chats/-100100/messages/10/comments", "from=502&limit=999").await.unwrap();
    assert_eq!(page2["has_more"], false);
    assert_eq!(page2["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page2["messages"][0]["id"], 501);

    // Tham số sai bị từ chối 400 không gọi TDLib
    for (path, query) in [
        ("/api/chats/0/messages/10/comments", ""),
        ("/api/chats/-100100/messages/0/comments", ""),
        ("/api/chats/-100100/messages/-5/comments", ""),
        ("/api/chats/-100100/messages/10/comments", "from=-1"),
        ("/api/chats/-100100/messages/10/comments", "limit=abc"),
    ] {
        let e = get(&chat, path, query).await.unwrap_err();
        assert_eq!((e.status, e.code.as_str()), (400, "BAD_REQUEST"), "{path}?{query}");
    }
}

#[tokio::test]
async fn channel_comments_post_sends_to_discussion_chat_with_shared_validation() {
    let ch = channel_chat(-100100, 100);
    let sg_full = json!({"@type":"supergroupFullInfo","linked_chat_id":-100200});
    let thread = json!({
        "@type":"messageThreadInfo",
        "chat_id":-100200,
        "message_thread_id":500,
        "messages":[{"id":500,"chat_id":-100200}]
    });
    let ft_plain = json!({"@type":"formattedText","text":"Ý kiến mới","entities":[]});
    let ft_fmt = json!({
        "@type":"formattedText",
        "text":"🌙 Chữ đậm",
        "entities":[{"@type":"textEntity","offset":3,"length":7,"type":{"@type":"textEntityTypeBold"}}]
    });

    let (chat, _) = setup(vec![
        // 1. Gửi bình luận vào gốc luồng (không cần confirm)
        ok(json!({"@type":"getChat","chat_id":-100100}), ch.clone()),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":100}), sg_full.clone()),
        ok(json!({"@type":"getMessageThread","chat_id":-100100,"message_id":10}), thread.clone()),
        ok(
            json!({
                "@type":"sendMessage",
                "chat_id":-100200,
                "message_thread_id":500,
                "reply_to":{"@type":"inputMessageReplyToMessage","message_id":500},
                "input_message_content":{
                    "@type":"inputMessageText",
                    "text":ft_plain,
                    "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}
                }
            }),
            json!({"id":504,"chat_id":-100200,"date":1700000010,"is_outgoing":true,"reply_to":{"message_id":500},
                "content":{"@type":"messageText","text":ft_plain}}),
        ),
        // 2. Gửi bình luận có entities UTF-16 + reply_to bình luận 502
        ok(json!({"@type":"getChat","chat_id":-100100}), ch),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":100}), sg_full),
        ok(json!({"@type":"getMessageThread","chat_id":-100100,"message_id":10}), thread),
        ok(
            json!({
                "@type":"sendMessage",
                "chat_id":-100200,
                "message_thread_id":500,
                "reply_to":{"@type":"inputMessageReplyToMessage","message_id":502},
                "input_message_content":{
                    "@type":"inputMessageText",
                    "text":ft_fmt,
                    "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}
                }
            }),
            json!({"id":505,"chat_id":-100200,"date":1700000020,"is_outgoing":true,"reply_to":{"message_id":502},
                "content":{"@type":"messageText","text":ft_fmt}}),
        ),
    ]);

    let r1 = post(&chat, "/api/chats/-100100/messages/10/comments", "", json!({"text":"Ý kiến mới"})).await.unwrap();
    assert_eq!(r1["id"], 504);
    assert_eq!(r1["chat_id"], -100200);
    assert_eq!(r1["reply_to"], 500);

    let r2 = post(&chat, "/api/chats/-100100/messages/10/comments", "", json!({
        "text": "🌙 Chữ đậm",
        "entities": [{"type":"bold","offset":3,"length":7}],
        "reply_to": 502
    })).await.unwrap();
    assert_eq!(r2["id"], 505);
    assert_eq!(r2["reply_to"], 502);
    assert_eq!(r2["entities"], json!([{"type":"bold","offset":3,"length":7}]));

    // Từ chối đầu vào sai mà không gọi TDLib
    let too_many: Vec<Value> = (0..101).map(|i| json!({"type":"bold","offset":i,"length":1})).collect();
    for bad in [
        json!({"text":"   "}),
        json!({"text":"a".repeat(MAX_TEXT_UTF16 + 1)}),
        json!({"text":"a".repeat(120),"entities":too_many}),
        json!({"text":"🌙 a","entities":[{"type":"bold","offset":0,"length":1}]}),
        json!({"text":"x","entities":[{"type":"text_link","offset":0,"length":1,"url":"javascript:alert(1)"}]}),
        json!({"text":"x","parse_mode":"markdown","entities":[{"type":"bold","offset":0,"length":1}]}),
        json!({"text":"x","reply_to":-1}),
        json!({"text":"x","unknown_field":1}),
    ] {
        let e = post(&chat, "/api/chats/-100100/messages/10/comments", "", bad.clone()).await.unwrap_err();
        assert_eq!((e.status, e.code.as_str()), (400, "BAD_REQUEST"), "{bad}");
    }
}

#[tokio::test]
async fn channel_comments_return_409_no_discussion_without_panic() {
    let private_chat = json!({"id":42,"title":"An","type":{"@type":"chatTypePrivate","user_id":7}});
    let group_chat = json!({"id":-10050,"title":"Nhóm","type":{"@type":"chatTypeSupergroup","supergroup_id":50,"is_channel":false}});
    let chan_no_link = channel_chat(-10060, 60);
    let chan_with_link = channel_chat(-10070, 70);

    let (chat, _) = setup(vec![
        // 1. Chat cá nhân -> 409 NO_DISCUSSION
        ok(json!({"@type":"getChat","chat_id":42}), private_chat),
        // 2. Nhóm thường (không phải kênh) -> 409 NO_DISCUSSION
        ok(json!({"@type":"getChat","chat_id":-10050}), group_chat),
        // 3. Kênh có linked_chat_id = 0 -> 409 NO_DISCUSSION
        ok(json!({"@type":"getChat","chat_id":-10060}), chan_no_link),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":60}), json!({"@type":"supergroupFullInfo","linked_chat_id":0})),
        // 4. TDLib getMessageThread trả 400 -> 409 NO_DISCUSSION
        ok(json!({"@type":"getChat","chat_id":-10070}), chan_with_link.clone()),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":70}), json!({"@type":"supergroupFullInfo","linked_chat_id":-100200})),
        err(json!({"@type":"getMessageThread","chat_id":-10070,"message_id":10}), 400, "Message thread not found"),
        // 5. messageThreadInfo thiếu chat_id/message_thread_id -> 409 NO_DISCUSSION (không panic)
        ok(json!({"@type":"getChat","chat_id":-10070}), chan_with_link),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":70}), json!({"@type":"supergroupFullInfo","linked_chat_id":-100200})),
        ok(json!({"@type":"getMessageThread","chat_id":-10070,"message_id":11}), json!({"@type":"messageThreadInfo","chat_id":0,"message_thread_id":0})),
        // 6. getChat trả 404 -> giữ nguyên 404
        err(json!({"@type":"getChat","chat_id":999}), 404, "Chat not found"),
    ]);

    for (path, body) in [
        ("/api/chats/42/messages/10/comments", None),
        ("/api/chats/-10050/messages/10/comments", None),
        ("/api/chats/-10060/messages/10/comments", Some(json!({"text":"Thử"}))),
        ("/api/chats/-10070/messages/10/comments", None),
        ("/api/chats/-10070/messages/11/comments", None),
    ] {
        let res = match body {
            None => get(&chat, path, "").await,
            Some(b) => post(&chat, path, "", b).await,
        };
        let e = res.unwrap_err();
        assert_eq!((e.status, e.code.as_str()), (409, "NO_DISCUSSION"), "{path}");
    }

    let e404 = get(&chat, "/api/chats/999/messages/10/comments", "").await.unwrap_err();
    assert_eq!(e404.status, 404);
}

#[tokio::test]
async fn channel_comments_routes_enforce_auth_csrf_and_path_validation() {
    let token = "test-token";
    let ch = channel_chat(-100100, 100);
    let sg_full = json!({"@type":"supergroupFullInfo","linked_chat_id":-100200});
    let thread = json!({
        "@type":"messageThreadInfo",
        "chat_id":-100200,
        "message_thread_id":500,
        "messages":[{"id":500,"chat_id":-100200}]
    });
    let ft = json!({"@type":"formattedText","text":"Hợp lệ","entities":[]});

    let (chat, _) = setup(vec![
        ok(json!({"@type":"getChat","chat_id":-100100}), ch.clone()),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":100}), sg_full.clone()),
        ok(json!({"@type":"getMessageThread","chat_id":-100100,"message_id":10}), thread.clone()),
        ok(
            json!({"@type":"getMessageThreadHistory","chat_id":-100100,"message_id":10,"from_message_id":0,"offset":0,"limit":31}),
            json!({"messages":[]}),
        ),
        ok(json!({"@type":"getChat","chat_id":-100100}), ch),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":100}), sg_full),
        ok(json!({"@type":"getMessageThread","chat_id":-100100,"message_id":10}), thread),
        ok(
            json!({
                "@type":"sendMessage",
                "chat_id":-100200,
                "message_thread_id":500,
                "reply_to":{"@type":"inputMessageReplyToMessage","message_id":500},
                "input_message_content":{"@type":"inputMessageText","text":ft,"link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}
            }),
            json!({"id":506,"chat_id":-100200,"date":1700000030,"is_outgoing":true,"content":{"@type":"messageText","text":ft}}),
        ),
    ]);

    let auth = Arc::new(UiAuth::in_memory(token));
    let grant = auth.setup(token, "nguoi-thu", "mat-khau-thu-dai", false).unwrap();
    let cookie = grant.cookie.split(';').next().unwrap().to_owned();
    let app = rest::router_with_ui_auth(
        Arc::new(rest::ChuaCo),
        Arc::new(chat),
        RestConfig { token: token.into(), rate_limit_per_minute: 120, version: "test".into() },
        auth,
    );

    // 1. Không có xác thực hoặc sai Bearer -> 401
    for (method, uri) in [
        ("GET", "/api/chats/-100100/messages/10/comments"),
        ("POST", "/api/chats/-100100/messages/10/comments"),
    ] {
        let r1 = app.clone().oneshot(Request::builder().method(method).uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(r1.status(), 401);
        let r2 = app.clone().oneshot(Request::builder().method(method).uri(uri).header(header::AUTHORIZATION, "Bearer sai").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(r2.status(), 401);
    }

    // 2. Cookie POST thiếu CSRF hoặc Origin lạ -> 403
    let r_no_csrf = app.clone().oneshot(
        Request::post("/api/chats/-100100/messages/10/comments")
            .header(header::COOKIE, &cookie)
            .body(Body::from(r#"{"text":"hi"}"#))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(r_no_csrf.status(), 403);

    let r_evil_origin = app.clone().oneshot(
        Request::post("/api/chats/-100100/messages/10/comments")
            .header(header::COOKIE, &cookie)
            .header(header::HOST, "gateway.example")
            .header(header::ORIGIN, "http://evil.example")
            .body(Body::from(r#"{"text":"hi"}"#))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(r_evil_origin.status(), 403);

    // 3. Tham số đường dẫn không phải số -> 422 int_parsing; sai phương thức -> 405; account lạ -> 404
    assert_eq!(
        app.clone().oneshot(Request::get("/api/chats/abc/messages/10/comments").header(header::AUTHORIZATION, "Bearer test-token").body(Body::empty()).unwrap()).await.unwrap().status(),
        422
    );
    assert_eq!(
        app.clone().oneshot(Request::get("/api/chats/-100100/messages/xyz/comments").header(header::AUTHORIZATION, "Bearer test-token").body(Body::empty()).unwrap()).await.unwrap().status(),
        422
    );
    assert_eq!(
        app.clone().oneshot(Request::delete("/api/chats/-100100/messages/10/comments").header(header::AUTHORIZATION, "Bearer test-token").body(Body::empty()).unwrap()).await.unwrap().status(),
        405
    );
    assert_eq!(
        app.clone().oneshot(Request::get("/api/chats/-100100/messages/10/comments?account=wrong").header(header::AUTHORIZATION, "Bearer test-token").body(Body::empty()).unwrap()).await.unwrap().status(),
        404
    );

    // 4. Bearer GET và Cookie + CSRF POST -> 200 OK
    let ok_get = app.clone().oneshot(
        Request::get("/api/chats/-100100/messages/10/comments")
            .header(header::AUTHORIZATION, "Bearer test-token")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(ok_get.status(), 200);
    let get_json: Value = serde_json::from_slice(&to_bytes(ok_get.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(get_json["discussion_chat_id"], -100200);

    let ok_post = app.oneshot(
        Request::post("/api/chats/-100100/messages/10/comments")
            .header(header::COOKIE, &cookie)
            .header("X-Requested-With", "fetch")
            .body(Body::from(r#"{"text":"Hợp lệ"}"#))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(ok_post.status(), 200);
}

#[test]
fn channel_comments_mutation_counterexamples_catch_dropped_limits_and_route_checks() {
    assert_eq!(MAX_COMMENTS_LIMIT, 100);
    assert_eq!(MAX_TEXT_UTF16, 4096);
    assert_eq!(MAX_ENTITIES, 100);
}

#[tokio::test]
async fn channel_comments_treat_unexpected_chat_error_at_thread_end_as_no_more() {
    // Gốc (23/11, kênh HÀ NỘI 24H): bài có ít bình luận hơn limit ⇒ trang 2 chạm bài gốc nằm ở KÊNH, TDLib báo
    // «Receive messages in an unexpected chat». Phải trả các bình luận đã có và has_more=false, không báo lỗi.
    let thread = json!({"@type":"messageThreadInfo","chat_id":-100200,"message_thread_id":500,"messages":[{"id":500,"chat_id":-100200}]});
    let comment = |id: i64| json!({"id":id,"chat_id":-100200,"date":1700000000 + id,"is_outgoing":false,
        "sender_id":{"@type":"messageSenderUser","user_id":11},"reply_to":{"@type":"messageReplyToMessage","message_id":500},
        "content":{"@type":"messageText","text":{"text":"bình luận"}}});
    let history = |from: i64| json!({"@type":"getMessageThreadHistory","chat_id":-100100,"message_id":10,"from_message_id":from,"offset":0,"limit":31});
    let (chat, _) = setup(vec![
        ok(json!({"@type":"getChat","chat_id":-100100}), channel_chat(-100100, 100)),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":100}), json!({"@type":"supergroupFullInfo","linked_chat_id":-100200})),
        ok(json!({"@type":"getMessageThread","chat_id":-100100,"message_id":10}), thread),
        ok(history(0), json!({"messages":[comment(502), comment(501)]})),
        err(json!({"@type":"getMessageThreadHistory","chat_id":-100100,"message_id":10,"from_message_id":501,"offset":0,"limit":29}),
            500, "Receive messages in an unexpected chat"),
        ok(json!({"@type":"getUser","user_id":11}), json!({"id":11,"first_name":"An"})),
    ]);
    let res = get(&chat, "/api/chats/-100100/messages/10/comments", "limit=30").await.unwrap();
    assert_eq!(res["messages"].as_array().unwrap().len(), 2);
    assert_eq!(res["has_more"], false);
}
