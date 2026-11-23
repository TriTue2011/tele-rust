mod uploads;
pub(crate) use uploads::synthesize_audio;

use serde_json::{json, Value};

use super::rich_text::{
    self, is_allowed_url, is_utf16_boundary, utf16_len, MAX_ENTITIES, MAX_TEXT_UTF16,
};
use super::{array, int, message, string, text, truth, ChatService, GwError, Query, Reply};
use crate::contract::TdError;
use std::time::Duration;
use uploads::{decode, voice_ogg, Upload, MAX_UPLOAD};

pub(super) fn formatted_text(text: &str) -> Value {
    json!({"@type":"formattedText","text":text,"entities":[]})
}

fn build_entities(text: &str, raw: &[Value]) -> Result<Vec<Value>, GwError> {
    if raw.len() > MAX_ENTITIES {
        return Err(GwError::bad_request("Tối đa 100 entity định dạng mỗi tin nhắn."));
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut out = Vec::with_capacity(raw.len());
    for item in raw {
        let Some(obj) = item.as_object() else {
            return Err(GwError::bad_request("Mỗi entity phải là đối tượng JSON."));
        };
        let offset = obj.get("offset").and_then(Value::as_u64)
            .ok_or_else(|| GwError::bad_request("offset của entity phải là số nguyên không âm."))?;
        let length = obj.get("length").and_then(Value::as_u64).filter(|&n| n >= 1)
            .ok_or_else(|| GwError::bad_request("length của entity phải là số nguyên dương."))?;
        let end = offset.checked_add(length)
            .ok_or_else(|| GwError::bad_request("Phạm vi entity vượt giới hạn."))?;
        let start_idx = usize::try_from(offset)
            .map_err(|_| GwError::bad_request("Phạm vi entity vượt giới hạn."))?;
        let end_idx = usize::try_from(end)
            .map_err(|_| GwError::bad_request("Phạm vi entity vượt giới hạn."))?;
        if end_idx > units.len() || !is_utf16_boundary(&units, start_idx) || !is_utf16_boundary(&units, end_idx) {
            return Err(GwError::bad_request("Phạm vi entity vượt quá độ dài tin hoặc cắt đôi ký tự UTF-16."));
        }
        let kind = obj.get("type").and_then(Value::as_str).unwrap_or_default();
        let td_type = match kind {
            "bold" => json!({"@type":"textEntityTypeBold"}),
            "italic" => json!({"@type":"textEntityTypeItalic"}),
            "underline" => json!({"@type":"textEntityTypeUnderline"}),
            "strikethrough" => json!({"@type":"textEntityTypeStrikethrough"}),
            "code" => json!({"@type":"textEntityTypeCode"}),
            "pre" => match obj.get("language") {
                None | Some(Value::Null) => json!({"@type":"textEntityTypePre"}),
                Some(Value::String(lang)) => {
                    let lang = lang.trim();
                    if lang.is_empty() {
                        json!({"@type":"textEntityTypePre"})
                    } else if lang.chars().count() > 64 || lang.chars().any(char::is_control) {
                        return Err(GwError::bad_request("language trong entity pre không hợp lệ."));
                    } else {
                        json!({"@type":"textEntityTypePreCode","language":lang})
                    }
                }
                Some(_) => return Err(GwError::bad_request("language trong entity pre phải là chuỗi.")),
            },
            "spoiler" => json!({"@type":"textEntityTypeSpoiler"}),
            "blockquote" => json!({"@type":"textEntityTypeBlockQuote"}),
            "text_link" => {
                let url = obj.get("url").and_then(Value::as_str)
                    .ok_or_else(|| GwError::bad_request("url trong entity text_link phải là chuỗi."))?;
                if !is_allowed_url(url) {
                    return Err(GwError::bad_request("url trong entity text_link không hợp lệ (chỉ hỗ trợ http, https, tg)."));
                }
                json!({"@type":"textEntityTypeTextUrl","url":url})
            }
            _ => return Err(GwError::bad_request("Loại entity không được hỗ trợ.")),
        };
        out.push(json!({"@type":"textEntity","offset":offset,"length":length,"type":td_type}));
    }
    Ok(out)
}

pub(super) fn opt_id(value: &Value, name: &str) -> Result<i64, GwError> {
    if truth(value) { int(value, name) } else { Ok(0) }
}

fn message_ids(body: &Value) -> Result<Vec<i64>, GwError> {
    let ids = &body["message_ids"];
    if !truth(ids) { return Ok(Vec::new()); }
    ids.as_array().ok_or_else(|| GwError::bad_request("message_ids phải là danh sách."))?
        .iter().map(|id| int(id, "message_ids")).collect()
}

fn file_content(file: &Upload, kind: &str, cap: Value) -> Value {
    let f = json!({"@type":"inputFileLocal","path":file.path()});
    match kind {
        "photo" => json!({"@type":"inputMessagePhoto","caption":cap,"photo":{"@type":"inputPhoto","photo":f,"width":0,"height":0,"added_sticker_file_ids":[]}}),
        "video" => json!({"@type":"inputMessageVideo","caption":cap,"video":{"@type":"inputVideo","video":f,"supports_streaming":true}}),
        "voice" => json!({"@type":"inputMessageVoiceNote","caption":cap,"voice_note":{"@type":"inputVoiceNote","voice_note":f,"duration":0,"waveform":""}}),
        "audio" => json!({"@type":"inputMessageAudio","caption":cap,"audio":{"@type":"inputAudio","audio":f}}),
        "document" => json!({"@type":"inputMessageDocument","caption":cap,"document":{"@type":"inputDocument","document":f}}),
        _ => unreachable!(),
    }
}

fn query_format_body(q: &Query<'_>) -> Result<Value, GwError> {
    let mut map = serde_json::Map::new();
    if let Some(pm) = q.get("parse_mode") {
        map.insert("parse_mode".into(), Value::String(pm));
    }
    if let Some(fmt) = q.get("format") {
        map.insert("format".into(), Value::String(fmt));
    }
    if let Some(ae) = q.get("auto_emphasis") {
        let trimmed = ae.trim().to_ascii_lowercase();
        if !trimmed.is_empty() {
            let b = match trimmed.as_str() {
                "true" | "1" => true,
                "false" | "0" => false,
                _ => return Err(GwError::bad_request("auto_emphasis phải là true hoặc false.")),
            };
            map.insert("auto_emphasis".into(), Value::Bool(b));
        }
    }
    if let Some(am) = q.get("ai_markup") {
        map.insert("ai_markup".into(), Value::String(am));
    }
    Ok(Value::Object(map))
}

fn album_caption_body(parent: &Value, item: &Value) -> Value {
    let mut map = serde_json::Map::new();
    for key in ["parse_mode", "format", "auto_emphasis", "ai_markup", "entities"] {
        if let Some(v) = item.get(key).filter(|v| !v.is_null()) {
            map.insert(key.into(), v.clone());
        } else if let Some(v) = parent.get(key).filter(|v| !v.is_null()) {
            map.insert(key.into(), v.clone());
        }
    }
    Value::Object(map)
}

impl ChatService {
    async fn formatted(&self, text: &str, mode: &str) -> Reply {
        let mode = if mode.is_empty() { "plain" } else { mode };
        let parser = match mode.trim().to_ascii_lowercase().as_str() {
            "plain" => return Ok(formatted_text(text)),
            "markdown" => json!({"@type":"textParseModeMarkdown","version":2}),
            "html" => json!({"@type":"textParseModeHTML"}),
            _ => return Err(GwError::bad_request("parse_mode phải là plain, markdown hoặc html")),
        };
        self.request(json!({"@type":"parseTextEntities","text":text,"parse_mode":parser})).await
    }

    pub(super) async fn formatted_input(&self, text: &str, body: &Value) -> Reply {
        let opts = rich_text::parse_options(body)?;
        let mode = string(&body["parse_mode"]);
        let mode_trimmed = mode.trim();
        let has_non_plain_mode = !mode_trimmed.is_empty() && !mode_trimmed.eq_ignore_ascii_case("plain");
        let has_explicit_format = body.get("format").is_some_and(|v| !v.is_null());

        if (opts.is_active() || has_explicit_format) && has_non_plain_mode {
            return Err(GwError::bad_request("Không dùng đồng thời parse_mode và format/auto_emphasis/ai_markup."));
        }

        if let Some(raw) = body.get("entities").filter(|v| !v.is_null()) {
            let list = raw.as_array().ok_or_else(|| GwError::bad_request("entities phải là danh sách."))?;
            if !list.is_empty() {
                if has_non_plain_mode {
                    return Err(GwError::bad_request("Không dùng đồng thời parse_mode và entities."));
                }
                if opts.is_active() || has_explicit_format {
                    return Err(GwError::bad_request("Không dùng đồng thời format/auto_emphasis/ai_markup và entities."));
                }
                if utf16_len(text) > MAX_TEXT_UTF16 {
                    return Err(GwError::bad_request("Tin quá dài (tối đa 4096 đơn vị UTF-16)."));
                }
                let entities = build_entities(text, list)?;
                return Ok(json!({"@type":"formattedText","text":text,"entities":entities}));
            }
        }

        if opts.is_active() {
            return rich_text::render_tdlib(text, &opts);
        }

        if !has_non_plain_mode && utf16_len(text) > MAX_TEXT_UTF16 {
            return Err(GwError::bad_request("Tin quá dài (tối đa 4096 đơn vị UTF-16)."));
        }
        self.formatted(text, &mode).await
    }

    pub(super) async fn final_id(&self, id: i64) -> i64 { self.state.final_id(id, Duration::from_secs(8)).await }

    async fn reply_to(&self, req: &mut Value, reply: i64) {
        if reply != 0 {
            req["reply_to"] = json!({"@type":"inputMessageReplyToMessage","message_id":self.final_id(reply).await});
        }
    }

    async fn send(&self, chat: i64, content: Value, reply: i64) -> Reply {
        let mut req = json!({"@type":"sendMessage","chat_id":chat,"input_message_content":content});
        self.reply_to(&mut req, reply).await;
        self.request(req).await
    }

    pub(super) async fn write_chat(&self, chat: i64, op: &str, q: &Query<'_>, b: &Value) -> Reply {
        match op {
            "send" => {
                let text = string(&b["text"]);
                if text.trim().is_empty() { return Err(GwError::bad_request("Thiếu nội dung tin.")); }
                let reply = opt_id(&b["reply_to"], "reply_to")?;
                let ft = self.formatted_input(&text, b).await?;
                let result = self.send(chat, json!({"@type":"inputMessageText","text":ft,
                    "link_preview_options":{"@type":"linkPreviewOptions","is_disabled":false}}), reply).await?;
                Ok(message(&result, ""))
            }
            "read" => {
                let mut ids = message_ids(b)?;
                ids.truncate(100);
                if !ids.is_empty() {
                    self.request(json!({"@type":"viewMessages","chat_id":chat,"message_ids":ids,
                        "source":{"@type":"messageSourceChatHistory"},"force_read":true})).await?;
                }
                Ok(json!({"ok":true}))
            }
            "send_file" => {
                let kind = q.get("kind").unwrap_or_else(|| "document".into());
                if !matches!(kind.as_str(), "photo" | "video" | "voice" | "audio" | "document") {
                    return Err(GwError::bad_request("kind phải là photo, video, voice, audio hoặc document."));
                }
                let raw = decode(&b["data_b64"], "data_b64")?;
                if raw.is_empty() { return Err(GwError::bad_request("Tệp rỗng.")); }
                if raw.len() > MAX_UPLOAD { return Err(GwError::bad_request("Tệp vượt 64 MB.")); }
                let reply = q.int("reply_to", 0)?;
                let cap_text = q.get("caption").unwrap_or_default();
                let cap_body = query_format_body(q)?;
                let cap = self.formatted_input(&cap_text, &cap_body).await?;
                let raw = if kind == "voice" { voice_ogg(raw).await? } else { raw };
                let file = Upload::save(&self.uploads, &q.get("name").unwrap_or_default(), raw).await?;
                let content = file_content(&file, &kind, cap);
                file.keep();
                Ok(message(&self.send(chat, content, reply).await?, ""))
            }
            "send_album" => self.album(chat, b).await,
            _ => unreachable!(),
        }
    }

    async fn album(&self, chat: i64, b: &Value) -> Reply {
        let items = array(&b["items"]);
        if !(2..=10).contains(&items.len()) { return Err(GwError::bad_request("items phải là danh sách 2-10 tệp.")); }
        let mut parsed = Vec::new();
        let (mut total, mut group) = (0, "");
        for item in items {
            if !item.is_object() { return Err(GwError::bad_request("Mỗi phần tử của items phải là một đối tượng.")); }
            let kind = if truth(&item["kind"]) { string(&item["kind"]) } else { "document".into() };
            let next_group = match kind.as_str() {
                "photo" | "video" => "media",
                "document" => "document",
                "audio" => "audio",
                _ => return Err(GwError::bad_request("kind trong album phải là photo, video, document hoặc audio.")),
            };
            if !group.is_empty() && group != next_group {
                return Err(GwError::bad_request("album không trộn ảnh/video với tệp hoặc âm thanh: gửi riêng từng nhóm"));
            }
            group = next_group;
            let data = decode(&item["data_b64"], "data_b64")?;
            if data.is_empty() { return Err(GwError::bad_request("Có tệp rỗng trong album.")); }
            total += data.len();
            if total > MAX_UPLOAD { return Err(GwError::bad_request("Album vượt 64 MB.")); }
            parsed.push((item, kind, data));
        }
        let reply = opt_id(&b["reply_to"], "reply_to")?;
        let mut parsed_items = Vec::with_capacity(parsed.len());
        for (item, kind, data) in parsed {
            let cap_body = album_caption_body(b, item);
            let cap = self.formatted_input(&string(&item["caption"]), &cap_body).await?;
            parsed_items.push((item, kind, data, cap));
        }
        let mut files = Vec::new();
        let mut contents = Vec::new();
        for (item, kind, data, cap) in parsed_items {
            let file = Upload::save(&self.uploads, &string(&item["name"]), data).await?;
            contents.push(file_content(&file, &kind, cap));
            files.push(file);
        }
        let mut req = json!({"@type":"sendMessageAlbum","chat_id":chat,"input_message_contents":contents});
        self.reply_to(&mut req, reply).await;
        for file in files { file.keep(); }
        let result = self.request(req).await?;
        let messages: Vec<_> = array(&result["messages"]).iter().map(|m| message(m, "")).collect();
        Ok(json!({"count":messages.len(),"messages":messages}))
    }

    pub(super) async fn write_message(&self, op: &str, b: &Value) -> Reply {
        let req = match op {
            "edit" => {
                let text = string(&b["text"]);
                if text.trim().is_empty() { return Err(GwError::bad_request("Thiếu nội dung mới.")); }
                let chat = int(&b["chat_id"], "chat_id")?;
                let id = self.final_id(int(&b["message_id"], "message_id")?).await;
                let ft = self.formatted_input(&text, b).await?;
                let m = self.request(json!({"@type":"editMessageText","chat_id":chat,"message_id":id,
                    "input_message_content":{"@type":"inputMessageText","text":ft}})).await?;
                return Ok(message(&m, ""));
            }
            "delete" | "forward" => {
                let chat = if op == "delete" { "chat_id" } else { "to_chat_id" };
                let chat = int(&b[chat], chat)?;
                let ids = message_ids(b)?;
                let mut req = if op == "delete" {
                    json!({"@type":"deleteMessages","chat_id":chat,"revoke":b.get("revoke").map_or(true, truth)})
                } else {
                    json!({"@type":"forwardMessages","chat_id":chat,"from_chat_id":int(&b["from_chat_id"], "from_chat_id")?})
                };
                let mut resolved = Vec::new();
                for id in ids { resolved.push(self.final_id(id).await); }
                req["message_ids"] = json!(resolved);
                req
            }
            "react" => {
                let chat = int(&b["chat_id"], "chat_id")?;
                let id = self.final_id(int(&b["message_id"], "message_id")?).await;
                let emoji = if truth(&b["emoji"]) { string(&b["emoji"]) } else { "👍".into() };
                let mut req = json!({"@type":"removeMessageReaction","chat_id":chat,"message_id":id,
                    "reaction_type":{"@type":"reactionTypeEmoji","emoji":emoji}});
                if b.get("on").map_or(true, truth) {
                    req["@type"] = json!("addMessageReaction");
                    req["is_big"] = json!(false);
                    req["update_recent_reactions"] = json!(true);
                }
                req
            }
            _ => unreachable!(),
        };
        self.request(req).await?;
        Ok(json!({"ok":true}))
    }

    async fn resolve_legacy(&self, peer: &str) -> Reply {
        let peer = peer.trim();
        let digits = peer.trim_start_matches('+');
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return self.open_private(parse_peer_id(digits)?).await;
        }
        let chat = self.request(json!({"@type":"searchPublicChat","username":peer.strip_prefix('@').unwrap_or(peer)})).await?;
        Ok(json!({"chat_id":chat["id"]}))
    }

    pub(super) async fn resolve(&self, b: &Value) -> Reply {
        let peer = string(&b["peer"]);
        let p = peer.trim();
        if p.is_empty() { return Err(GwError::bad_request("Thiếu peer (@username, +số điện thoại hoặc ID số).")); }
        if p.strip_prefix('-').is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())) {
            return Ok(json!({"chat_id":parse_peer_id(p)?}));
        }
        if let Some(phone) = p.strip_prefix('+') {
            let digits = phone.replace(' ', "");
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(GwError::bad_request("số điện thoại chỉ gồm chữ số sau dấu +"));
            }
            let user = self.request(json!({"@type":"searchUserByPhoneNumber","phone_number":digits,"only_local":false})).await?;
            if user["@type"] != "user" { return Err(GwError::from_tdlib(TdError::telegram(404, "PHONE_NOT_FOUND"))); }
            return self.open_private(int(&user["id"], "user_id")?).await;
        }
        self.resolve_legacy(p).await
    }

    pub(super) async fn translate(&self, b: &Value) -> Reply {
        let text = string(&b["text"]);
        let code = string(&b["to_language_code"]).trim().to_lowercase();
        if code.is_empty() || text.trim().is_empty() { return Err(GwError::bad_request("cần text và to_language_code (ví dụ vi, en)")); }
        let result = self.request(json!({"@type":"translateText","text":formatted_text(&text),"to_language_code":code})).await?;
        Ok(json!({"text":string(&result["text"])}))
    }

    pub(super) async fn legacy_send(&self, op: &str, b: &Value) -> Reply {
        self.account(text(&b["account"]))?;
        let peer = string(&b["peer"]);
        let file;
        let content = match op {
            "send_message" => {
                let msg = string(&b["message"]);
                if peer.is_empty() || msg.is_empty() { return Err(GwError::bad_request("Thiếu peer hoặc message.")); }
                let ft = self.formatted_input(&msg, b).await?;
                json!({"@type":"inputMessageText","text":ft,"link_preview_options":{"@type":"linkPreviewOptions","is_disabled":true}})
            }
            "send_media" => {
                if peer.is_empty() || !truth(&b["media_b64"]) { return Err(GwError::bad_request("Thiếu peer hoặc media_b64.")); }
                let raw = decode(&b["media_b64"], "media_b64")?;
                let name = b["filename"].as_str().unwrap_or("media.bin");
                let caption = text(&b["caption"]);
                let cap = if caption.is_empty()
                    && !rich_text::has_rich_params(b)
                    && !truth(&b["parse_mode"])
                    && !truth(&b["entities"])
                {
                    formatted_text("")
                } else {
                    self.formatted_input(caption, b).await?
                };
                file = Upload::save(&self.uploads, name, raw).await?;
                file_content(&file, "document", cap)
            }
            "send_voice" => {
                if peer.is_empty() { return Err(GwError::bad_request("Thiếu peer.")); }
                let raw = if truth(&b["audio_b64"]) {
                    voice_ogg(decode(&b["audio_b64"], "audio_b64")?).await?
                } else if truth(&b["tts_text"]) {
                    uploads::synthesize(&self.uploads, &string(&b["tts_text"]), text(&b["tts_voice"]), text(&b["tts_rate"])).await?
                } else {
                    return Err(GwError::bad_request("Thiếu audio_b64 hoặc tts_text."));
                };
                file = Upload::save(&self.uploads, "voice.ogg", raw).await?;
                file_content(&file, "voice", formatted_text(""))
            }
            _ => unreachable!(),
        };
        let chat_id = self.resolve_legacy(&peer).await?;
        let result = self.send(int(&chat_id["chat_id"], "chat_id")?, content, 0).await?;
        Ok(json!({"ok":true,"tin_id":int(&result["id"], "message_id")?.to_string(),"account":self.account,"peer":peer,"mock":false}))
    }
}

fn parse_peer_id(s: &str) -> Result<i64, GwError> {
    s.parse().map_err(|_| GwError::bad_request("ID người nhận phải là số nguyên."))
}
