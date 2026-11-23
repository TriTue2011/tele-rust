//! Cảm xúc được phép + hình nền hội thoại (đồng bộ Telegram qua TDLib).
use super::*;

const MSG: i64 = 1_048_576;      // mã tin thật của máy chủ (bội của 2^20) ⇒ final_id trả ngay, không chờ

fn bg_json(id: &str) -> Value {
    json!({"@type":"background","id":id,"is_default":false,"is_dark":false,"name":"nui","type":{"@type":"backgroundTypeWallpaper","is_blurred":false,"is_moving":false},
        "document":{"@type":"document","thumbnail":{"file":{"id":8}},"document":{"id":9}}})
}

#[tokio::test]
async fn available_reactions_returns_only_plain_emoji_per_chat() {
    let (chat, _td) = setup(vec![ok(json!({"@type":"getMessageAvailableReactions","chat_id":5,"message_id":MSG,"row_size":8}), json!({
        "top_reactions":[{"type":{"@type":"reactionTypeEmoji","emoji":"👍"},"needs_premium":false},{"type":{"@type":"reactionTypeEmoji","emoji":"❤️"},"needs_premium":false},
            {"type":{"@type":"reactionTypeEmoji","emoji":"🔥"},"needs_premium":false}],
        "recent_reactions":[{"type":{"@type":"reactionTypeEmoji","emoji":"😢"},"needs_premium":false},{"type":{"@type":"reactionTypeCustomEmoji","custom_emoji_id":"77"}}],
        "popular_reactions":[{"type":{"@type":"reactionTypeEmoji","emoji":"🦄"},"needs_premium":true},{"type":{"@type":"reactionTypeEmoji","emoji":"🎉"},"needs_premium":false}],
        "allow_custom_emoji":true}))]);
    let r = get(&chat, "/api/messages/reactions", &format!("chat_id=5&message_id={MSG}")).await.unwrap();
    assert_eq!(r, json!({"top":["👍","❤️","🔥"],"recent":["😢"],"popular":["🎉"],"allow_custom_emoji":true}), "bỏ emoji Premium và emoji tuỳ biến");
    let (chat, _td) = setup(vec![err(json!({"@type":"getMessageAvailableReactions","chat_id":5,"message_id":MSG,"row_size":8}), 400, "MESSAGE_ID_INVALID")]);
    assert!(get(&chat, "/api/messages/reactions", &format!("chat_id=5&message_id={MSG}")).await.is_err(), "lỗi TDLib được trả ra, không nuốt");
}

#[tokio::test]
async fn installed_backgrounds_pass_the_theme_flag_and_shape_results() {
    let (chat, _td) = setup(vec![ok(json!({"@type":"getInstalledBackgrounds","for_dark_theme":false}), json!({"backgrounds":[bg_json("11"), bg_json("22")]})),
        ok(json!({"@type":"getInstalledBackgrounds","for_dark_theme":true}), json!({"backgrounds":[]}))]);
    let day = get(&chat, "/api/backgrounds", "").await.unwrap();
    assert_eq!((day["backgrounds"].as_array().unwrap().len(), day["backgrounds"][0]["id"].as_str(), day["backgrounds"][0]["thumb_file"].as_i64()), (2, Some("11"), Some(8)));
    assert_eq!(get(&chat, "/api/backgrounds", "dark=1").await.unwrap(), json!({"backgrounds":[]}));
}

#[tokio::test]
async fn chat_background_reads_current_or_null() {
    let (chat, _td) = setup(vec![
        ok(json!({"@type":"getChat","chat_id":42}), json!({"@type":"chat","id":42,"title":"A","type":{"@type":"chatTypePrivate","user_id":7},"background":{"@type":"chatBackground","dark_theme_dimming":35,"background":bg_json("33")}})),
        ok(json!({"@type":"getChat","chat_id":43}), json!({"@type":"chat","id":43,"title":"B","type":{"@type":"chatTypePrivate","user_id":8}}))]);
    let r = get(&chat, "/api/chats/42/background", "").await.unwrap();
    assert_eq!((r["background"]["dimming"].as_i64(), r["background"]["background"]["id"].as_str()), (Some(35), Some("33")));
    assert_eq!(get(&chat, "/api/chats/43/background", "").await.unwrap(), json!({"background":null}));
}

#[tokio::test]
async fn set_background_builds_the_right_tdlib_request_for_each_source() {
    let set = |bg: Value, kind: Value, dim: i64, only_self: bool| json!({"@type":"setChatBackground","chat_id":42,"background":bg,"type":kind,"dark_theme_dimming":dim,"only_for_self":only_self});
    let (chat, _td) = setup(vec![
        ok(set(json!({"@type":"inputBackgroundRemote","background_id":"5678901234567890123"}), Value::Null, 0, true), json!({"@type":"ok"})),
        ok(set(json!({"@type":"inputBackgroundRemote","background_id":"-77"}), Value::Null, 40, false), json!({"@type":"ok"})),
        ok(set(Value::Null, json!({"@type":"backgroundTypeFill","fill":{"@type":"backgroundFillSolid","color":0xE8F0FE}}), 0, true), json!({"@type":"ok"})),
        ok(set(Value::Null, json!({"@type":"backgroundTypeFill","fill":{"@type":"backgroundFillGradient","top_color":0xFF0000,"bottom_color":0x0000FF,"rotation_angle":45}}), 10, true), json!({"@type":"ok"})),
        ok(json!({"@type":"deleteChatBackground","chat_id":42,"restore_previous":false}), json!({"@type":"ok"})),
        ok(json!({"@type":"deleteChatBackground","chat_id":42,"restore_previous":true}), json!({"@type":"ok"})),
    ]);
    let go = |b: Value| { let chat = &chat; async move { post(chat, "/api/chats/42/background", "", b).await } };
    assert_eq!(go(json!({"background_id":"5678901234567890123"})).await.unwrap(), json!({"ok":true}));
    go(json!({"background_id":"-77","dimming":40,"for_both":true})).await.unwrap();
    go(json!({"color":"#E8F0FE"})).await.unwrap();
    go(json!({"gradient":{"from":"#ff0000","to":"#0000ff","angle":405},"dimming":"10"})).await.unwrap();           // 405° ⇒ 45°
    go(json!({"remove":true})).await.unwrap();
    go(json!({"remove":true,"restore_previous":true})).await.unwrap();
}

#[tokio::test]
async fn set_background_rejects_bad_input_before_touching_telegram() {
    let (chat, _td) = setup(vec![]);                      // không có lời gọi TDLib nào được phép
    for body in [json!({}), json!({"background_id":"abc"}), json!({"background_id":""}), json!({"color":"red"}), json!({"color":"#12345"}),
        json!({"gradient":{"from":"#000000","to":"nope"}}), json!({"color":"#000000","dimming":101}), json!({"color":"#000000","dimming":-1}),
        json!({"file_path":"../../etc/passwd"}), json!({"file_path":"x.jpg"}), json!({"file_path":format!("{}.jpg","a".repeat(32))})] {
        assert!(post(&chat, "/api/chats/42/background", "", body.clone()).await.is_err(), "{body}");
    }
}

#[tokio::test]
async fn wallpaper_upload_validates_stores_and_feeds_set_background() {
    use base64::Engine as _;
    let enc = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0]; jpeg.extend_from_slice(&[7u8; 200]);
    let (chat, _td) = setup(vec![]);
    for bad in [json!({"data_b64":""}), json!({"data_b64":"!!!"}), json!({"data_b64":enc(b"GIF89a....")}), json!({"data_b64":enc(&vec![0xFF; 3 * 1024 * 1024 + 1])})] {
        assert!(post(&chat, "/api/wallpapers/upload", "", bad.clone()).await.is_err(), "{bad}");
    }
    let up = post(&chat, "/api/wallpapers/upload", "", json!({"data_b64":enc(&jpeg)})).await.unwrap();
    let name = up["file"].as_str().unwrap().to_owned();
    assert!(name.len() == 36 && name.ends_with(".jpg"), "{name}");
    assert_eq!(post(&chat, "/api/wallpapers/upload", "", json!({"data_b64":enc(&jpeg)})).await.unwrap()["file"], name, "cùng nội dung ⇒ cùng tên");
    let path = std::path::Path::new("target/chat-tests/wallpapers").join(&name);
    assert_eq!(std::fs::read(&path).unwrap(), jpeg);
    { use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600); }
    let abs = std::fs::canonicalize(&path).unwrap();
    let want = |p: &str| json!({"@type":"setChatBackground","chat_id":42,"background":{"@type":"inputBackgroundLocal","background":{"@type":"inputFileLocal","path":p}},
        "type":{"@type":"backgroundTypeWallpaper","is_blurred":true,"is_moving":false},"dark_theme_dimming":25,"only_for_self":true});
    let (chat2, _td2) = setup(vec![ok(want(path.to_str().unwrap()), json!({"@type":"ok"}))]);
    // setup() dùng cùng thư mục dữ liệu `target/chat-tests` ⇒ tên ảnh hợp lệ.
    post(&chat2, "/api/chats/42/background", "", json!({"file_path":name,"blurred":true,"dimming":25})).await.unwrap();
    let _ = abs;
}
