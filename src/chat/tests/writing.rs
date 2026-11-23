use super::*;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub(super) fn check_request(mut request: Value, mut expected: Value) {
    fn uploads(actual: &mut Value, expected: &mut Value) {
        if let Some(offset) = expected.as_str().and_then(|s| s.strip_prefix("now+")).and_then(|s| s.parse::<i64>().ok()) {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            assert!((now+offset-5..=now+offset).contains(&actual.as_i64().unwrap()));
            *expected = actual.clone();
        } else if expected["@type"] == "inputFileLocal" && !expected["path"].as_str().is_some_and(|p| p.contains("/wallpapers/")) {      // ảnh nền tải lên nằm ở wallpapers/ (giữ nguyên, so sánh đường dẫn thẳng)
            let name = expected["path"].as_str().unwrap();
            let path = Path::new(actual["path"].as_str().unwrap());
            assert_eq!(actual["@type"], "inputFileLocal");
            assert_eq!(path.parent().unwrap(), Path::new("target/chat-tests/uploads"));
            assert!(path.file_name().unwrap().to_str().unwrap().ends_with(&format!("_{name}")));
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(fs::read(path).unwrap(), expected["bytes"].as_str().unwrap().as_bytes());
            fs::remove_file(path).unwrap();
            actual["path"] = expected["path"].clone();
            expected.as_object_mut().unwrap().remove("bytes");
        } else if let (Some(a), Some(e)) = (actual.as_object_mut(), expected.as_object_mut()) {
            for (key, e) in e { if let Some(a) = a.get_mut(key) { uploads(a, e); } }
        } else if let (Some(a), Some(e)) = (actual.as_array_mut(), expected.as_array_mut()) {
            for (a,e) in a.iter_mut().zip(e) { uploads(a,e); }
        }
    }
    uploads(&mut request, &mut expected);
    assert_eq!(request, expected);
}

fn ft(s: &str) -> Value { json!({"@type":"formattedText","text":s,"entities":[]}) }
fn file(name: &str, bytes: &str) -> Value { json!({"@type":"inputFileLocal","path":name,"bytes":bytes}) }
fn send(content: Value) -> Value { json!({"@type":"sendMessage","chat_id":5,"input_message_content":content}) }
fn private(user: i64) -> Value { json!({"@type":"createPrivateChat","user_id":user,"force":false}) }
fn success(req: Value) -> FakeReply { ok(req,json!({"@type":"ok"})) }

#[tokio::test]
async fn send_edit_read_and_message_mutations_use_the_correct_ids_and_shapes() {
    let real = 3 << 20;
    let html = json!({"@type":"formattedText","text":"new","entities":[{"offset":0,"length":3,"type":{"@type":"textEntityTypeItalic"}}]});
    let md = json!({"@type":"formattedText","text":"hey","entities":[{"offset":0,"length":3,"type":{"@type":"textEntityTypeBold"}}]});
    let (chat, td) = setup(vec![
        ok(json!({"@type":"parseTextEntities","text":"*hey*","parse_mode":{"@type":"textParseModeMarkdown","version":2}}),md.clone()),
        ok(json!({"@type":"sendMessage","chat_id":5,"reply_to":{"@type":"inputMessageReplyToMessage","message_id":real},
            "input_message_content":{"@type":"inputMessageText","text":md,"link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}}),td_message(17)),
        ok(json!({"@type":"parseTextEntities","text":"<i>new</i>","parse_mode":{"@type":"textParseModeHTML"}}),html.clone()),
        ok(json!({"@type":"editMessageText","chat_id":5,"message_id":real,"input_message_content":{"@type":"inputMessageText","text":html}}),td_message(real)),
        success(json!({"@type":"deleteMessages","chat_id":5,"message_ids":[real],"revoke":true})),
        success(json!({"@type":"forwardMessages","chat_id":6,"from_chat_id":5,"message_ids":[real]})),
        success(json!({"@type":"addMessageReaction","chat_id":5,"message_id":real,"reaction_type":{"@type":"reactionTypeEmoji","emoji":"👍"},"is_big":false,"update_recent_reactions":true})),
        success(json!({"@type":"removeMessageReaction","chat_id":5,"message_id":real,"reaction_type":{"@type":"reactionTypeEmoji","emoji":"😃"}})),
        success(json!({"@type":"viewMessages","chat_id":5,"message_ids":[17],"source":{"@type":"messageSourceChatHistory"},"force_read":true})),
    ]);
    let update = async {
        tokio::task::yield_now().await;
        td.updates.send(json!({"@type":"updateMessageSendSucceeded","old_message_id":17,"message":td_message(real)})).unwrap();
    };
    let (sent, ()) = tokio::join!(post(&chat,"/api/chats/5/send","",json!({"text":"*hey*","parse_mode":"Markdown","reply_to":"17"})), update);
    assert_eq!(sent.unwrap(),normalized(17,""));
    let feed = get(&chat,"/api/feed", "").await.unwrap();
    assert_eq!(feed["items"][0]["old_id"],17);
    assert_eq!(feed["items"][0]["message"]["id"],real);
    assert_eq!(post(&chat,"/api/messages/edit","",json!({"chat_id":5,"message_id":17,"text":"<i>new</i>","parse_mode":"html"})).await.unwrap(),normalized(real,""));
    for (path, body) in [
        ("/api/messages/delete",json!({"chat_id":5,"message_ids":[17]})),
        ("/api/messages/forward",json!({"to_chat_id":6,"from_chat_id":5,"message_ids":[17]})),
        ("/api/messages/react",json!({"chat_id":5,"message_id":17})),
        ("/api/messages/react",json!({"chat_id":5,"message_id":real,"emoji":"😃","on":false})),
        ("/api/chats/5/read",json!({"message_ids":["17"]})), ("/api/chats/5/read",json!({})),
    ] { assert_eq!(post(&chat,path,"",body).await.unwrap(),json!({"ok":true})); }
}

#[tokio::test]
async fn resolves_chats_and_translates_with_python_shapes() {
    let (chat, _) = setup(vec![ok(private(7),json!({"id":5})),
        ok(json!({"@type":"searchUserByPhoneNumber","phone_number":"000999","only_local":false}),json!({"@type":"user","id":7})),
        ok(private(7),json!({"id":5})),ok(json!({"@type":"searchPublicChat","username":"demo"}),json!({"id":5})),
        ok(json!({"@type":"translateText","text":ft("Chào"),"to_language_code":"en"}),json!({"text":"Hi"})),
    ]);
    assert_eq!(post(&chat,"/api/resolve","",json!({"peer":" -900 "})).await.unwrap(),json!({"chat_id":-900}));
    for peer in ["7"," +000 999 ","@demo"] {
        assert_eq!(post(&chat,"/api/resolve","",json!({"peer":peer})).await.unwrap(),json!({"chat_id":5}));
    }
    assert_eq!(post(&chat,"/api/translate","",json!({"text":"Chào","to_language_code":" EN "})).await.unwrap(),json!({"text":"Hi"}));
}

#[tokio::test]
async fn uploads_use_tdlib_wrappers_and_one_album_with_per_item_caption() {
    let photo = json!({"@type":"inputMessagePhoto","caption":ft("ảnh"),"photo":{"@type":"inputPhoto","photo":file("a.jpg","photo"),"width":0,"height":0,"added_sticker_file_ids":[]}});
    let video = json!({"@type":"inputMessageVideo","caption":ft("video"),"video":{"@type":"inputVideo","video":file("b.mp4","video"),"supports_streaming":true}});
    let document = json!({"@type":"inputMessageDocument","caption":ft("mô tả"),"document":{"@type":"inputDocument","document":file("demo.pdf","pdf")}});
    let mut send_file = send(document);
    send_file["reply_to"] = json!({"@type":"inputMessageReplyToMessage","message_id":1<<20});
    let (chat, _) = setup(vec![ok(send_file,td_message(20)),
        ok(json!({"@type":"sendMessageAlbum","chat_id":5,"input_message_contents":[photo,video]}),json!({"messages":[td_message(30),td_message(31)]})),
    ]);
    assert_eq!(post(&chat,"/api/chats/5/send_file","name=..%2F..%2Fdemo.pdf&caption=m%C3%B4+t%E1%BA%A3&reply_to=1048576",json!({"data_b64":"cGRm"})).await.unwrap(),normalized(20,""));
    let b = json!({"items":[{"kind":"photo","name":"a.jpg","caption":"ảnh","data_b64":"cGhvdG8="},
        {"kind":"video","name":"b.mp4","caption":"video","data_b64":"dmlkZW8="}]});
    assert_eq!(post(&chat,"/api/chats/5/send_album","",b).await.unwrap(),json!({"messages":[normalized(30,""),normalized(31,"")],"count":2}));
}

#[tokio::test]
async fn legacy_voice_media_and_message_preserve_request_and_envelope() {
    let voice = json!({"@type":"inputMessageVoiceNote","caption":ft(""),"voice_note":{"@type":"inputVoiceNote","voice_note":file("voice.ogg","OggSfake"),"duration":0,"waveform":""}});
    let media = json!({"@type":"inputMessageDocument","caption":ft("caption"),"document":{"@type":"inputDocument","document":file("report.txt","ok")}});
    let txt = json!({"@type":"inputMessageText","text":ft("msg"),"link_preview_options":{"@type":"linkPreviewOptions","is_disabled":true}});
    let (chat, _) = setup(vec![ok(private(7),json!({"id":5})),ok(send(voice),td_message(20)),
        ok(private(7),json!({"id":5})),ok(send(media),td_message(21)),ok(private(7),json!({"id":5})),ok(send(txt),td_message(22))]);
    for (path, id, b) in [("/api/send_voice","20",json!({"peer":"7","audio_b64":"T2dnU2Zha2U=","account":"+000123"})),
        ("/api/send_media","21",json!({"peer":"7","media_b64":"b2s=","filename":"report.txt","caption":"caption"})),
        ("/api/send_message","22",json!({"peer":"7","message":"msg"}))] {
        assert_eq!(post(&chat,path,"",b).await.unwrap(),json!({"ok":true,"tin_id":id,"account":"+000123","peer":"7","mock":false}));
    }
}

#[tokio::test]
async fn invalid_writes_do_not_send_any_tdlib_request() {
    let (chat, _) = setup(vec![]);
    for (path, b) in [("/api/chats/5/send",json!({"text":" "})),("/api/chats/5/send",json!({"text":"x","parse_mode":"wiki"})),
        ("/api/chats/5/send",json!({"text":"x","reply_to":"no"})),("/api/chats/5/send_file",json!({"data_b64":"?"})),
        ("/api/chats/5/send_album",json!({"items":[{"data_b64":"eA==","kind":"photo"},{"data_b64":"eA==","kind":"document"}]})),
        ("/api/chats/5/read",json!({"message_ids":["oops"]})),("/api/messages/edit",json!({"chat_id":5,"text":"new"})),
        ("/api/messages/delete",json!({"chat_id":5,"message_ids":4})),("/api/messages/forward",json!({"to_chat_id":5})),
        ("/api/messages/react",json!({"chat_id":5})),("/api/resolve",json!({"peer":"+oops"})),
        ("/api/translate",json!({"text":"x"})),("/api/send_voice",json!({"peer":"7","audio_b64":"?"}))] {
        let err = post(&chat,path,"",b).await.unwrap_err();
        assert_eq!((err.status,err.code.as_str()),(400,"BAD_REQUEST"),"{path}");
    }
}

#[tokio::test]
async fn send_and_edit_support_structured_entities_with_utf16_offsets() {
    let real = 1_i64 << 20;
    let text = "🌙 Chào code pre";
    let ft = json!({
        "@type":"formattedText",
        "text":text,
        "entities":[
            {"@type":"textEntity","offset":3,"length":4,"type":{"@type":"textEntityTypeBold"}},
            {"@type":"textEntity","offset":3,"length":4,"type":{"@type":"textEntityTypeItalic"}},
            {"@type":"textEntity","offset":8,"length":4,"type":{"@type":"textEntityTypeCode"}},
            {"@type":"textEntity","offset":13,"length":3,"type":{"@type":"textEntityTypePreCode","language":"sh"}}
        ]
    });
    let edit_ft = json!({
        "@type":"formattedText",
        "text":"Mới",
        "entities":[{"@type":"textEntity","offset":0,"length":3,"type":{"@type":"textEntityTypeSpoiler"}}]
    });
    let (chat, _) = setup(vec![
        ok(json!({"@type":"sendMessage","chat_id":5,"input_message_content":{"@type":"inputMessageText","text":ft,
            "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}}), td_message(20)),
        ok(json!({"@type":"editMessageText","chat_id":5,"message_id":real,
            "input_message_content":{"@type":"inputMessageText","text":edit_ft}}), td_message(real)),
    ]);
    let sent = post(&chat, "/api/chats/5/send", "", json!({
        "text": text,
        "entities": [
            {"type":"bold","offset":3,"length":4},
            {"type":"italic","offset":3,"length":4},
            {"type":"code","offset":8,"length":4},
            {"type":"pre","offset":13,"length":3,"language":"sh"}
        ]
    })).await.unwrap();
    assert_eq!(sent["id"], 20);
    let edited = post(&chat, "/api/messages/edit", "", json!({
        "chat_id":5, "message_id":real, "text":"Mới",
        "entities":[{"type":"spoiler","offset":0,"length":3}]
    })).await.unwrap();
    assert_eq!(edited["id"], real);
}

#[tokio::test]
async fn invalid_entities_and_parse_mode_conflict_fail_without_calling_tdlib() {
    let (chat, _) = setup(vec![]);
    let msg_id = 1_i64 << 20;
    for (path, body) in [
        ("/api/chats/5/send", json!({"text":"🌙 a","entities":[{"type":"bold","offset":1,"length":1}]})),
        ("/api/chats/5/send", json!({"text":"🌙 a","entities":[{"type":"bold","offset":0,"length":1}]})),
        ("/api/chats/5/send", json!({"text":"abc","entities":[{"type":"bold","offset":2,"length":2}]})),
        ("/api/chats/5/send", json!({"text":"abc","entities":[{"type":"bold","offset":0,"length":0}]})),
        ("/api/chats/5/send", json!({"text":"abc","entities":[{"type":"script","offset":0,"length":1}]})),
        ("/api/chats/5/send", json!({"text":"abc","entities":"bad"})),
        ("/api/chats/5/send", json!({"text":"abc","parse_mode":"markdown","entities":[{"type":"bold","offset":0,"length":1}]})),
        ("/api/chats/5/send", json!({"text":"abc","entities":[{"type":"pre","offset":0,"length":3,"language":"a\nb"}]})),
        ("/api/messages/edit", json!({"chat_id":5,"message_id":msg_id,"text":"abc","entities":[{"type":"bold","offset":99,"length":1}]})),
    ] {
        let err = post(&chat, path, "", body).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_REQUEST"), "{path}");
    }
}
