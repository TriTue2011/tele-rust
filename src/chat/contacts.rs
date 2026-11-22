use std::{collections::HashSet, time::{SystemTime, UNIX_EPOCH}};

use icu_collator::{options::CollatorOptions, Collator};
use serde_json::{json, Value};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use super::{array, telegram_error, text, ChatService, GwError, Reply};

const BLOCK_PAGE: usize = 100;

fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }

fn status(raw: &Value, now: i64) -> (u8, i64, Value) {
    let (rank, kind, label, seen) = match text(&raw["@type"]) {
        "userStatusOnline" if raw["expires"].as_i64().is_some_and(|end| end <= now) =>
            (4, "offline", "Đã ngoại tuyến", raw["expires"].as_i64()),
        "userStatusOnline" => (5, "online", "đang trực tuyến", None),
        "userStatusOffline" => (4, "offline", "Đã ngoại tuyến", raw["was_online"].as_i64()),
        "userStatusRecently" => (3, "recently", "Truy cập gần đây", None),
        "userStatusLastWeek" => (2, "week", "Truy cập trong tuần qua", None),
        "userStatusLastMonth" => (1, "month", "Truy cập trong tháng qua", None),
        _ => (0, "hidden", "Không rõ lần cuối truy cập", None),
    };
    let seen = seen.filter(|t| *t > 0);
    let mut value = json!({"kind":kind,"text":label});
    if let Some(t) = seen { value["last_seen"] = json!(t); }
    (rank, seen.unwrap_or(0), value)
}

/// Tên hiển thị trong Danh bạ; giữ fallback theo ID khi thiếu tên để đồng nhất mục `#`.
fn contact_name(u: &Value, id: i64) -> String {
    let full = format!("{} {}", text(&u["first_name"]), text(&u["last_name"]));
    let trimmed = full.trim();
    if !trimmed.is_empty() {
        return trimmed.into();
    }
    let uname = text(&u["usernames"]["active_usernames"][0]);
    if !uname.is_empty() {
        return uname;
    }
    id.to_string()
}

fn entry(u: &Value, id: i64, active: Value) -> Value {
    json!({"id":id,"name":contact_name(u, id),"first_name":text(&u["first_name"]),"last_name":text(&u["last_name"]),
        "username":text(&u["usernames"]["active_usernames"][0]),"phone":text(&u["phone_number"]),
        "avatar_file":u["profile_photo"]["small"]["id"],"status":active,"is_mutual":u["is_mutual_contact"] == true})
}

fn name_key(name: &str) -> (bool, String, String) {
    let folded: String = name.nfd().filter(|&c| !is_combining_mark(c)).flat_map(char::to_lowercase)
        .map(|c| if c == 'đ' { 'd' } else { c }).collect();
    match folded.find(char::is_alphabetic) {
        Some(start) => {
            let key = &folded[start..];
            let section = key.chars().take(1).flat_map(char::to_uppercase).collect();
            (false, section, key.into())
        }
        None => (true, "#".into(), folded),
    }
}

fn user_id(v: &Value) -> Result<i64, GwError> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .filter(|id| (1..=9_007_199_254_740_991).contains(id))
        .ok_or_else(|| GwError::bad_request("ID liên hệ phải là số nguyên dương hợp lệ."))
}

fn names(body: &Value) -> Result<(&str, &str), GwError> {
    let first = text(&body["first_name"]).trim();
    let last = body.get("last_name").map_or(Some(""), Value::as_str).map(str::trim);
    if (1..=64).contains(&first.chars().count()) {
        if let Some(last) = last.filter(|s| s.chars().count() <= 64) { return Ok((first, last)); }
    }
    Err(GwError::bad_request("Tên phải có 1–64 ký tự và họ tối đa 64 ký tự."))
}

fn checked_flag(body: &Value, key: &str) -> Result<bool, GwError> {
    body.get(key).map_or(Some(false), Value::as_bool)
        .ok_or_else(|| GwError::bad_request(format!("{key} phải là true hoặc false.")))
}

fn phone(raw: &str) -> Result<String, GwError> {
    let bad = || GwError::new(422, "INVALID_PHONE", "Nhập số điện thoại hợp lệ, ví dụ +84 hoặc đầu số 0 của Việt Nam.");
    let mut clean = String::new();
    for c in raw.chars() {
        if c.is_ascii_digit() || (c == '+' && clean.is_empty()) { clean.push(c); }
        else if !c.is_whitespace() && !matches!(c, '-' | '–' | '.' | '(' | ')') { return Err(bad()); }
        if clean.len() > 17 { return Err(bad()); }
    }
    let number = if let Some(international) = clean.strip_prefix('+').or_else(|| clean.strip_prefix("00")) {
        international.to_owned()
    } else if let Some(national) = clean.strip_prefix('0') {
        if national.len() != 9 && national.len() != 10 { return Err(bad()); }
        format!("84{national}")
    } else {
        clean
    };
    if !(7..=15).contains(&number.len()) || number.starts_with('0') { return Err(bad()); }
    Ok(format!("+{number}"))
}

fn imported(phone: &str, first: &str, last: &str) -> Value {
    json!({"@type":"importedContact","phone_number":phone,"first_name":first,"last_name":last})
}

fn block_list() -> Value { json!({"@type":"blockListMain"}) }

impl ChatService {
    async fn contact_request(&self, request: Value) -> Reply {
        self.td.request(request).await.map_err(|error| {
            let mut public = telegram_error(error);
            if matches!(public.code.as_str(), "TELEGRAM_ERROR" | "MEDIA_ENGINE_ERROR") {
                public.message = "Telegram không thực hiện được thao tác danh bạ.".into();
            }
            public
        })
    }

    async fn contact_user(&self, id: i64) -> Reply {
        self.contact_request(json!({"@type":"getUser","user_id":id})).await
    }

    async fn blocked_users(&self) -> Result<HashSet<i64>, GwError> {
        let mut ids = HashSet::new();
        let mut offset = 0;
        loop {
            let page = self.contact_request(json!({"@type":"getBlockedMessageSenders","block_list":block_list(),
                "offset":offset,"limit":BLOCK_PAGE})).await?;
            let senders = array(&page["senders"]);
            ids.extend(senders.iter().filter(|s| s["@type"] == "messageSenderUser").filter_map(|s| s["user_id"].as_i64()));
            if senders.len() < BLOCK_PAGE { return Ok(ids); }
            offset += senders.len();
        }
    }

    pub(super) async fn contacts(&self, sort: &str, blocked_only: bool) -> Reply {
        if !matches!(sort, "last_seen" | "name") {
            return Err(GwError::bad_request("sort chỉ nhận last_seen hoặc name."));
        }
        let mut users = Vec::new();
        if !blocked_only {
            let result = self.contact_request(json!({"@type":"getContacts"})).await?;
            for id in array(&result["user_ids"]).iter().filter_map(Value::as_i64) {
                users.push((id, self.contact_user(id).await?));
            }
        }
        let blocked = self.blocked_users().await?;
        if blocked_only {
            for &id in &blocked { users.push((id, self.contact_user(id).await?)); }
        }

        let now = now();
        let mut rows: Vec<_> = users.into_iter().map(|(id, u)| {
            let (rank, seen, active) = status(&u["status"], now);
            let mut row = entry(&u, id, active);
            row["blocked"] = json!(blocked.contains(&id));
            (id, rank, seen, row)
        }).collect();

        if sort == "name" {
            let collator = Collator::try_new(Default::default(), CollatorOptions::default())
                .map_err(|_| GwError::new(500, "CONTACT_SORT", "Không thể sắp xếp tên liên hệ."))?;
            let mut names: Vec<_> = rows.into_iter().map(|(id, _, _, mut row)| {
                let key = name_key(text(&row["name"]));
                row["section"] = json!(key.1);
                (key, id, row)
            }).collect();
            names.sort_by(|(a, id_a, _), (b, id_b, _)| a.0.cmp(&b.0)
                .then_with(|| collator.compare(&a.1, &b.1))
                .then_with(|| collator.compare(&a.2, &b.2))
                .then_with(|| id_a.cmp(id_b)));
            return Ok(json!({"contacts":names.into_iter().map(|(_, _, row)| row).collect::<Vec<_>>()}));
        }

        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)).then_with(|| a.0.cmp(&b.0)));
        Ok(json!({"contacts":rows.into_iter().map(|(_, _, _, row)| row).collect::<Vec<_>>()}))
    }

    pub(super) async fn change_contact(&self, op: &str, body: &Value) -> Reply {
        match op {
            "add" => {
                let phone = phone(text(&body["phone"]))?;
                let (first, last) = names(body)?;
                let share = checked_flag(body, "share_my_phone")?;
                let contact = imported(&phone, first, last);
                let result = self.contact_request(json!({"@type":"importContacts","contacts":[contact]})).await?;
                let id = match result["user_ids"][0].as_i64() {
                    Some(0) => return Err(GwError::new(404, "not_telegram_user", "Số này không phải người dùng Telegram.")),
                    Some(id) if id > 0 => id,
                    _ => return Err(GwError::new(502, "TELEGRAM_ERROR", "Telegram không trả về liên hệ vừa thêm.")),
                };
                if share {
                    self.contact_request(json!({"@type":"addContact","user_id":id,"contact":contact,"share_phone_number":true}))
                        .await.map_err(|mut error| {
                            error.message = "Đã thêm liên hệ, nhưng chưa xác nhận được việc chia sẻ số của bạn.".into();
                            error
                        })?;
                }
                let user = self.contact_user(id).await?;
                Ok(json!({"contact":entry(&user, id, status(&user["status"], now()).2)}))
            }
            "edit" => {
                let id = user_id(&body["user_id"])?;
                let (first, last) = names(body)?;
                let user = self.contact_user(id).await?;
                if user["is_contact"] != true { return Err(GwError::not_found("Người này không có trong danh bạ.")); }
                let contact = imported(text(&user["phone_number"]), first, last);
                self.contact_request(json!({"@type":"addContact","user_id":id,"contact":contact,"share_phone_number":false})).await?;
                Ok(json!({"user_id":id,"edited":true}))
            }
            "remove" => {
                let ids = array(&body["user_ids"]);
                if !(1..=100).contains(&ids.len()) { return Err(GwError::bad_request("Cần chọn từ 1 đến 100 liên hệ để xoá.")); }
                let ids = ids.iter().map(user_id).collect::<Result<Vec<_>, _>>()?;
                self.contact_request(json!({"@type":"removeContacts","user_ids":ids})).await?;
                Ok(json!({"user_ids":ids,"removed":true}))
            }
            "block" | "unblock" => self.contact_block(user_id(&body["user_id"])?, op == "block", body).await,
            _ => Err(GwError::not_found("Không có thao tác danh bạ này.")),
        }
    }

    async fn contact_block(&self, id: i64, block: bool, body: &Value) -> Reply {
        let report = block && checked_flag(body, "report_spam")?;
        let delete = block && checked_flag(body, "delete_chat")?;
        let chat_id = if report || delete {
            let chat = self.contact_request(json!({"@type":"createPrivateChat","user_id":id,"force":false})).await?;
            chat["id"].clone()
        } else { Value::Null };
        if report {
            let result = self.contact_request(json!({"@type":"reportChat","chat_id":chat_id,
                "option_id":"","message_ids":[],"text":""})).await?;
            if result["@type"] != "reportChatResultOk" {
                return Err(GwError::new(422, "REPORT_NEEDS_DETAILS", "Telegram cần thêm thông tin để báo spam. Bạn có thể bỏ chọn Báo spam để chặn liên hệ."));
            }
        }
        self.contact_request(json!({"@type":"setMessageSenderBlockList","sender_id":{"@type":"messageSenderUser","user_id":id},
            "block_list":if block { block_list() } else { Value::Null }})).await?;
        if delete {
            self.contact_request(json!({"@type":"deleteChatHistory","chat_id":chat_id,"remove_from_chat_list":true,"revoke":false}))
                .await.map_err(|mut error| {
                    error.message = "Đã chặn liên hệ, nhưng Telegram chưa xoá hội thoại.".into();
                    error
                })?;
        }
        Ok(json!({"user_id":id,"blocked":block}))
    }
}
