use serde_json::{json, Value};

use super::{array, field, text, truth};

pub(super) fn user_name(u: &Value) -> String {
    let name = format!("{} {}", text(&u["first_name"]), text(&u["last_name"]));
    if !name.trim().is_empty() { return name.trim().into(); }
    if let Some(username) = u["usernames"]["active_usernames"][0].as_str() {
        return format!("@{username}");
    }
    u.get("id").map(Value::to_string).unwrap_or_default()
}

fn content(c: &Value) -> (&'static str, String) {
    let cap = text(&c["caption"]["text"]).trim();
    let caption = |label: &str| if cap.is_empty() { label.into() } else { format!("{label}: {cap}") };
    let (kind, summary) = match text(&c["@type"]) {
        "messageText" => ("text", text(&c["text"]["text"]).into()),
        "messagePhoto" => ("photo", caption("🖼 Ảnh")),
        "messageVideo" => ("video", caption("🎬 Video")),
        "messageAnimation" => ("animation", "GIF".into()),
        "messageVoiceNote" => ("voice", "🎤 Tin nhắn thoại".into()),
        "messageVideoNote" => ("video_note", "⏺ Video tròn".into()),
        "messageAudio" => ("audio", format!("🎵 {}", nonempty(&c["audio"]["title"], "Âm thanh"))),
        "messageDocument" => ("document", format!("📎 {}", nonempty(&c["document"]["file_name"], "Tệp"))),
        "messageSticker" => ("sticker", format!("{} Nhãn dán", text(&c["sticker"]["emoji"]))),
        "messageCall" => ("call", "📞 Cuộc gọi".into()),
        "messagePoll" => ("poll", "📊 Bình chọn".into()),
        "messageLocation" => ("location", "📍 Vị trí".into()),
        "messageContact" => ("contact", "👤 Danh bạ".into()),
        "" => ("other", String::new()),
        t => ("other", format!("[{}]", t.replacen("message", "", 1).to_lowercase())),
    };
    (kind, summary.trim().into())
}

fn nonempty<'a>(v: &'a Value, default: &'a str) -> &'a str {
    v.as_str().filter(|s| !s.is_empty()).unwrap_or(default)
}

fn file_info(f: &Value) -> Option<Value> {
    if !truth(f) { return None; }
    let size = [&f["size"], &f["expected_size"]].into_iter().find(|v| truth(v)).cloned().unwrap_or(json!(0));
    let local = &f["local"];
    let path = if truth(&local["is_downloading_completed"]) { local["path"].clone() } else { json!("") };
    Some(json!({"id":f["id"],"size":size,"path":path}))
}

fn message_entity(e: &Value) -> Option<Value> {
    let offset = e["offset"].as_u64()?;
    let length = e["length"].as_u64().filter(|n| *n > 0)?;
    let ty = &e["type"];
    let kind = match text(&ty["@type"]) {
        "textEntityTypeBold" => "bold",
        "textEntityTypeItalic" => "italic",
        "textEntityTypeUnderline" => "underline",
        "textEntityTypeStrikethrough" => "strikethrough",
        "textEntityTypeCode" => "code",
        "textEntityTypePre" | "textEntityTypePreCode" => "pre",
        "textEntityTypeBlockQuote" | "textEntityTypeExpandableBlockQuote" => "blockquote",
        "textEntityTypeSpoiler" => "spoiler",
        "textEntityTypeUrl" => "url",
        "textEntityTypeTextUrl" => "text_link",
        "textEntityTypeMention" => "mention",
        "textEntityTypeMentionName" => "text_mention",
        _ => return None,
    };
    // TDLib và chuỗi JavaScript đều dùng vị trí UTF-16.
    let mut entity = json!({"type":kind,"offset":offset,"length":length});
    match kind {
        "text_link" => entity["url"] = json!(ty["url"].as_str()?),
        "text_mention" => entity["user_id"] = json!(ty["user_id"].as_i64()?),
        "pre" => if let Some(language) = ty["language"].as_str() { entity["language"] = json!(language); },
        _ => (),
    }
    Some(entity)
}

pub(super) fn message(m: &Value, sender: &str) -> Value {
    let c = &m["content"];
    let (kind, summary) = content(c);
    let formatted = &c[if kind == "text" {"text"} else {"caption"}];
    let entities: Vec<_> = array(&formatted["entities"]).iter().filter_map(message_entity).take(100).collect();
    let reactions: Vec<_> = array(&m["interaction_info"]["reactions"]["reactions"]).iter().map(|r| json!({
        "emoji":nonempty(&r["type"]["emoji"],"?"),"count":field(r,"total_count",json!(0)),"mine":truth(&r["is_chosen"])
    })).collect();
    let file = if kind == "photo" {
        let sizes = array(&c["photo"]["sizes"]);
        let width = |v: &Value| v["width"].as_i64().unwrap_or(0);
        let best = sizes.iter().rev().filter(|s| width(s) <= 1280).max_by_key(|s| width(s)).or_else(|| sizes.last());
        best.map(|s| {
            let mut info = file_info(&s["photo"]).unwrap_or_else(|| json!({}));
            info["width"] = s["width"].clone();
            info["height"] = s["height"].clone();
            info["mime"] = json!("image/jpeg");
            info
        })
    } else if matches!(kind, "video" | "animation" | "audio" | "document" | "voice" | "video_note" | "sticker") {
        let obj = &c[if kind == "voice" { "voice_note" } else { kind }];
        ["video", "animation", "audio", "document", "voice", "video_note", "sticker"].iter()
            .find_map(|k| file_info(&obj[*k])).map(|mut f| {
                for (key, source, default) in [("name","file_name",json!("")),("mime","mime_type",json!("")),
                    ("duration","duration",json!(0)),("width","width",json!(0)),("height","height",json!(0))] {
                    f[key] = field(obj, source, default);
                }
                f
            })
    } else { None };
    json!({"id":m["id"],"chat_id":m["chat_id"],"date":field(m,"date",json!(0)),"edit_date":field(m,"edit_date",json!(0)),
        "outgoing":truth(&m["is_outgoing"]),"sender":sender,"kind":kind,
        "text":text(&formatted["text"]),"entities":entities,"summary":summary,
        "reply_to":m["reply_to"]["message_id"].as_i64().unwrap_or(0),"reactions":reactions,"file":file})
}

pub(super) fn summarize(chat: &Value) -> Value {
    let ct = &chat["type"];
    let kind = match text(&ct["@type"]) {
        "chatTypePrivate" => "private",
        "chatTypeSecret" => "secret",
        "chatTypeSupergroup" if truth(&ct["is_channel"]) => "channel",
        "chatTypeSupergroup" | "chatTypeBasicGroup" => "group",
        _ => "other",
    };
    let lm = &chat["last_message"];
    let last = if truth(lm) {
        let (kind, summary) = content(&lm["content"]);
        json!({"date":field(lm,"date",json!(0)),"outgoing":truth(&lm["is_outgoing"]),"kind":kind,
            "text":summary.chars().take(120).collect::<String>()})
    } else { Value::Null };
    json!({"id":chat["id"],"title":field(chat,"title",json!("")),"kind":kind,"unread":field(chat,"unread_count",json!(0)),
        "unread_mentions":field(chat,"unread_mention_count",json!(0)),"pinned":truth(&chat["positions"][0]["is_pinned"]),
        "muted":chat["notification_settings"]["mute_for"].as_i64().unwrap_or(0)>0,
        "auto_delete_seconds":chat["message_auto_delete_time"].as_i64().unwrap_or(0).max(0),
        "last":last,"avatar_file":chat["photo"]["small"]["id"],"user_id":ct["user_id"]})
}
