//! Đọc và gửi bình luận cho bài đăng kênh qua nhóm thảo luận liên kết của TDLib.
use std::collections::HashSet;

use serde_json::{json, Value};

use super::write::opt_id;
use super::{array, message, string, telegram_error, text, truth, ChatService, GwError, Reply};

pub(crate) const MAX_COMMENTS_LIMIT: i64 = 100;

struct DiscussionThread {
    discussion_chat_id: i64,
    thread_id: i64,
    root_message_id: i64,
}

fn no_discussion(msg: &str) -> GwError {
    GwError::new(409, "NO_DISCUSSION", msg)
}

impl ChatService {
    async fn resolve_discussion_thread(&self, chat_id: i64, message_id: i64) -> Result<DiscussionThread, GwError> {
        if chat_id == 0 {
            return Err(GwError::bad_request("chat_id phải khác 0."));
        }
        if message_id <= 0 {
            return Err(GwError::bad_request("message_id phải là số nguyên dương."));
        }
        let ch = self.request(json!({"@type": "getChat", "chat_id": chat_id})).await?;
        self.state.remember_chat(&ch);
        let ct = &ch["type"];
        let is_channel = text(&ct["@type"]) == "chatTypeSupergroup" && truth(&ct["is_channel"]);
        let sg_id = ct["supergroup_id"].as_i64().unwrap_or(0);
        if !is_channel || sg_id == 0 {
            return Err(no_discussion("Chỉ hỗ trợ bình luận cho bài đăng trong kênh có nhóm thảo luận."));
        }
        let sg_full = self.request(json!({
            "@type": "getSupergroupFullInfo",
            "supergroup_id": sg_id,
        })).await?;
        if sg_full["linked_chat_id"].as_i64().unwrap_or(0) == 0 {
            return Err(no_discussion("Kênh này chưa liên kết nhóm thảo luận."));
        }
        let thread = match self.td.request(json!({
            "@type": "getMessageThread",
            "chat_id": chat_id,
            "message_id": message_id,
        })).await {
            Ok(v) => v,
            Err(e) if e.code == Some(400) => {
                return Err(no_discussion("Bài đăng này không có luồng bình luận."));
            }
            Err(e) => return Err(telegram_error(e)),
        };
        let discussion_chat_id = thread["chat_id"].as_i64().unwrap_or(0);
        let thread_id = thread["message_thread_id"].as_i64().unwrap_or(0);
        if discussion_chat_id == 0 || thread_id == 0 {
            return Err(no_discussion("Không tìm thấy nhóm thảo luận cho bài đăng này."));
        }
        let root_message_id = thread["messages"]
            .as_array()
            .and_then(|msgs| msgs.first())
            .and_then(|m| m["id"].as_i64())
            .filter(|&id| id > 0)
            .unwrap_or(thread_id);
        Ok(DiscussionThread { discussion_chat_id, thread_id, root_message_id })
    }

    pub(super) async fn channel_comments(&self, chat_id: i64, message_id: i64, from: i64, limit: i64) -> Reply {
        if from < 0 {
            return Err(GwError::bad_request("from phải là số nguyên không âm."));
        }
        let limit = limit.clamp(1, MAX_COMMENTS_LIMIT) as usize;
        let thread = self.resolve_discussion_thread(chat_id, message_id).await?;

        let want = limit + 1;
        let mut cursor = from;
        let mut got: Vec<Value> = Vec::new();
        let mut seen: HashSet<i64> = HashSet::new();
        for _ in 0..3 {
            let req_limit = (want - got.len()).clamp(1, MAX_COMMENTS_LIMIT as usize);
            let result = match self.request(json!({
                "@type": "getMessageThreadHistory",
                "chat_id": chat_id,
                "message_id": message_id,
                "from_message_id": cursor,
                "offset": 0,
                "limit": req_limit,
            })).await {
                Ok(result) => result,
                // Hết bình luận: bài gốc nằm ở KÊNH (chat khác nhóm thảo luận) nên khi TDLib đọc quá bình luận cũ nhất nó báo
                // «Receive messages in an unexpected chat». Đó là điểm cuối của luồng, không phải lỗi — chỉ chấp nhận khi đã có mốc phân trang.
                Err(e) if cursor != 0 && e.message.contains("unexpected chat") => break,
                Err(e) => return Err(e),
            };
            let messages = array(&result["messages"]);
            if messages.is_empty() {
                break;
            }
            let mut next_cursor = cursor;
            let mut added = 0usize;
            for msg in messages {
                let mid = msg["id"].as_i64().unwrap_or(0);
                if mid != 0 {
                    next_cursor = if next_cursor == 0 || mid < next_cursor { mid } else { next_cursor };
                    if !seen.insert(mid) {
                        continue;
                    }
                }
                got.push(msg.clone());
                added += 1;
            }
            if added == 0 || next_cursor == cursor {
                break;
            }
            cursor = next_cursor;
            if got.len() >= want {
                break;
            }
        }
        let has_more = got.len() > limit;
        if has_more {
            got.truncate(limit);
        }
        let out = self.format_messages(thread.discussion_chat_id, got).await;
        Ok(json!({
            "discussion_chat_id": thread.discussion_chat_id,
            "thread_id": thread.thread_id,
            "root_message_id": thread.root_message_id,
            "messages": out,
            "has_more": has_more,
        }))
    }

    pub(super) async fn send_channel_comment(&self, chat_id: i64, message_id: i64, body: &Value) -> Reply {
        if chat_id == 0 {
            return Err(GwError::bad_request("chat_id phải khác 0."));
        }
        if message_id <= 0 {
            return Err(GwError::bad_request("message_id phải là số nguyên dương."));
        }
        let Some(map) = body.as_object() else {
            return Err(GwError::bad_request("Thân POST phải là JSON object."));
        };
        let allowed = ["text", "entities", "parse_mode", "format", "auto_emphasis", "ai_markup", "reply_to", "account"];
        if let Some(bad) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(GwError::bad_request(format!("Trường bình luận không hợp lệ: {bad}")));
        }
        if let Some(b_acc) = body.get("account").and_then(Value::as_str) {
            self.account(b_acc)?;
        }
        let text_val = string(&body["text"]);
        if text_val.trim().is_empty() {
            return Err(GwError::bad_request("Thiếu nội dung bình luận."));
        }
        let reply_to = opt_id(&body["reply_to"], "reply_to")?;
        if reply_to < 0 {
            return Err(GwError::bad_request("reply_to không hợp lệ."));
        }
        let ft = self.formatted_input(&text_val, body).await?;

        let thread = self.resolve_discussion_thread(chat_id, message_id).await?;
        let target_reply = if reply_to > 0 {
            self.final_id(reply_to).await
        } else {
            thread.root_message_id
        };
        let req = json!({
            "@type": "sendMessage",
            "chat_id": thread.discussion_chat_id,
            "message_thread_id": thread.thread_id,
            "reply_to": {
                "@type": "inputMessageReplyToMessage",
                "message_id": target_reply,
            },
            "input_message_content": {
                "@type": "inputMessageText",
                "text": ft,
                "link_preview_options": {
                    "@type": "linkPreviewOptions",
                    "is_disabled": false,
                },
            },
        });
        let sent = self.request(req).await?;
        Ok(message(&sent, ""))
    }
}
