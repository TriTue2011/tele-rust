//! HTTP chat cho một tài khoản TDLib đã đăng nhập.
mod feed;
mod files;
mod info;
mod services;
mod shape;
mod write;
pub(crate) use write::synthesize_audio;
#[cfg(test)]
mod tests;

use std::{path::{Path, PathBuf}, sync::Arc, time::Duration};

use serde_json::{json, Value};

use crate::contract::{Binary, BoxFut, ChatControl, GwError, TdLink};
use crate::events::EventSink;
use feed::State;
use shape::{message, summarize, user_name};

type Reply = Result<Value, GwError>;

pub struct ChatService {
    td: Arc<dyn TdLink>,
    account: String,
    data_dir: PathBuf,
    uploads: PathBuf,
    state: Arc<State>,
    updates: tokio::task::JoinHandle<()>,
    call_updates: Option<tokio::task::JoinHandle<()>>,
}

impl ChatService {
    /// Tạo trong Tokio runtime của gateway, dùng số tài khoản mà tầng đăng nhập đã chọn.
    pub fn new(td: Arc<dyn TdLink>, account: impl Into<String>, data_dir: impl AsRef<Path>) -> Self {
        Self::with_events(td, account, data_dir, None)
    }

    pub(crate) fn with_events(td: Arc<dyn TdLink>, account: impl Into<String>, data_dir: impl AsRef<Path>, events: Option<EventSink>) -> Self {
        let state = Arc::new(State::default());
        let updates = tokio::spawn(state.clone().receive(td.updates(), events));
        let data_dir = data_dir.as_ref().to_path_buf();
        Self { td, account: account.into(), uploads: data_dir.join("uploads"), data_dir, state, updates, call_updates: None }
    }

    pub(crate) fn follow_calls(mut self, source: tokio::sync::broadcast::Receiver<Value>) -> Self {
        self.call_updates = Some(tokio::spawn(self.state.clone().receive_calls(source)));
        self
    }

    async fn request(&self, req: Value) -> Reply {
        self.td.request(req).await.map_err(telegram_error)
    }

    fn account(&self, selected: &str) -> Result<(), GwError> {
        if selected.is_empty() || selected == self.account {
            Ok(())
        } else {
            Err(GwError::not_found("Không rõ tài khoản."))
        }
    }

    async fn me(&self) -> Reply {
        let me = self.request(json!({"@type":"getMe"})).await?;
        self.state.remember_user(&me);
        Ok(json!({"id":me["id"],"name":user_name(&me),"phone":field(&me,"phone_number",json!(""))}))
    }

    async fn profile(&self) -> Reply {
        let me = self.request(json!({"@type":"getMe"})).await?;
        self.state.remember_user(&me);
        let uid = me["id"].as_i64().unwrap_or_default();
        let full = if uid != 0 {
            self.request(json!({"@type":"getUserFullInfo","user_id":uid})).await.ok()
        } else {
            None
        };
        let username = me["usernames"]["editable_username"].as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| me["usernames"]["active_usernames"][0].as_str())
            .unwrap_or_default();
        let bio = full.as_ref().and_then(|f| f["bio"]["text"].as_str()).unwrap_or_default();
        Ok(json!({
            "id": me["id"],
            "name": user_name(&me),
            "first_name": text(&me["first_name"]),
            "last_name": text(&me["last_name"]),
            "bio": bio,
            "username": username,
            "phone_masked": mask_phone(text(&me["phone_number"])),
            "avatar_file": me["profile_photo"]["small"]["id"],
        }))
    }

    async fn update_profile(&self, body: &Value) -> Reply {
        let Some(map) = body.as_object() else {
            return Err(GwError::bad_request("Thân POST phải là JSON object."));
        };
        if map.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(GwError::bad_request("Cần xác nhận trước khi lưu hồ sơ."));
        }
        let allowed = ["confirm", "first_name", "last_name", "bio", "username"];
        if let Some(bad) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(GwError::bad_request(format!("Trường hồ sơ không hợp lệ: {bad}")));
        }
        let first_name = match map.get("first_name") {
            Some(v) => {
                let s = v.as_str().ok_or_else(|| GwError::bad_request("first_name phải là chuỗi."))?.trim().to_owned();
                if !(1..=64).contains(&s.chars().count()) {
                    return Err(GwError::bad_request("Tên phải từ 1 đến 64 ký tự."));
                }
                Some(s)
            }
            None => None,
        };
        let last_name = match map.get("last_name") {
            Some(v) => {
                let s = v.as_str().ok_or_else(|| GwError::bad_request("last_name phải là chuỗi."))?.trim().to_owned();
                if s.chars().count() > 64 {
                    return Err(GwError::bad_request("Họ tối đa 64 ký tự."));
                }
                Some(s)
            }
            None => None,
        };
        let bio = match map.get("bio") {
            Some(v) => {
                let s = v.as_str().ok_or_else(|| GwError::bad_request("bio phải là chuỗi."))?.trim().to_owned();
                if s.chars().count() > 70 {
                    return Err(GwError::bad_request("Tiểu sử tối đa 70 ký tự."));
                }
                Some(s)
            }
            None => None,
        };
        let username = match map.get("username") {
            Some(v) => {
                let s = v.as_str().ok_or_else(|| GwError::bad_request("username phải là chuỗi."))?
                    .trim().trim_start_matches('@').to_owned();
                if !valid_username(&s) {
                    return Err(GwError::bad_request("Username phải dài 5–32 ký tự (a-z, 0-9, _), bắt đầu bằng chữ cái, không kết thúc bằng _ hoặc có __, hoặc để trống để xoá."));
                }
                Some(s)
            }
            None => None,
        };
        if first_name.is_none() && last_name.is_none() && bio.is_none() && username.is_none() {
            return Err(GwError::bad_request("Không có thay đổi hồ sơ nào để lưu."));
        }
        if first_name.is_some() || last_name.is_some() {
            let (first, last) = match (first_name, last_name) {
                (Some(f), Some(l)) => (f, l),
                (f_opt, l_opt) => {
                    let me = self.request(json!({"@type":"getMe"})).await?;
                    self.state.remember_user(&me);
                    let f = f_opt.unwrap_or_else(|| text(&me["first_name"]).trim().to_owned());
                    let l = l_opt.unwrap_or_else(|| text(&me["last_name"]).trim().to_owned());
                    if !(1..=64).contains(&f.chars().count()) {
                        return Err(GwError::bad_request("Tên phải từ 1 đến 64 ký tự."));
                    }
                    (f, l)
                }
            };
            self.request(json!({"@type":"setName","first_name":first,"last_name":last})).await?;
        }
        if let Some(bio) = bio {
            self.request(json!({"@type":"setBio","bio":bio})).await?;
        }
        if let Some(username) = username {
            self.request(json!({"@type":"setUsername","username":username})).await?;
        }
        self.profile().await
    }

    pub(crate) async fn account_name(&self) -> Result<String, GwError> {
        let me = self.request(json!({"@type":"getMe"})).await?;
        self.state.remember_user(&me);
        Ok(format!("{} {}", text(&me["first_name"]), text(&me["last_name"])).trim().into())
    }

    async fn user(&self, id: i64) -> Reply {
        if let Some(user) = self.state.user(id) {
            return Ok(user);
        }
        let user = self.request(json!({"@type":"getUser","user_id":id})).await?;
        self.state.remember_user(&user);
        Ok(user)
    }

    async fn chat_list(&self, ids: &Value) -> Value {
        let mut chats = Vec::new();
        for id in array(ids) {
            if let Ok(chat) = self.request(json!({"@type":"getChat","chat_id":id})).await {
                let mut item = summarize(&chat);
                if let Some(chat_id) = chat["id"].as_i64().or_else(|| id.as_i64()).filter(|cid| *cid != 0) {
                    let sec = self.state.sync_chat_auto_delete(chat_id, chat["message_auto_delete_time"].as_i64().unwrap_or(0));
                    item["auto_delete_seconds"] = json!(sec);
                }
                chats.push(item);
            }
        }
        json!({"chats":chats})
    }

    async fn chats(&self, limit: i64) -> Reply {
        let list = json!({"@type":"chatListMain"});
        let _ = tokio::time::timeout(Duration::from_secs(15),
            self.request(json!({"@type":"loadChats","chat_list":list,"limit":limit}))).await;
        let ids = self.request(json!({"@type":"getChats","chat_list":list,"limit":limit})).await?;
        Ok(self.chat_list(&ids["chat_ids"]).await)
    }

    async fn history(&self, chat: i64, mut before: i64, limit: usize) -> Reply {
        let mut got = Vec::new();
        for _ in 0..3 {
            let result = self.request(json!({"@type":"getChatHistory","chat_id":chat,"from_message_id":before,
                "offset":0,"limit":limit-got.len(),"only_local":false})).await?;
            let messages = array(&result["messages"]);
            let Some(last) = messages.last() else { break };
            before = int(&last["id"], "message_id")?;
            got.extend_from_slice(messages);
            if got.len() >= limit { break; }
        }
        let mut out = Vec::new();
        for m in got {
            let sender = &m["sender_id"];
            let name = match text(&sender["@type"]) {
                "messageSenderUser" => match sender["user_id"].as_i64() {
                    Some(id) => self.user(id).await.map(|u| user_name(&u)).unwrap_or_default(),
                    None => String::new(),
                },
                "messageSenderChat" => self.request(json!({"@type":"getChat","chat_id":sender["chat_id"]})).await
                    .map(|c| text(&c["title"]).to_owned()).unwrap_or_default(),
                _ => String::new(),
            };
            out.push(message(&m, &name));
        }
        out.sort_by_key(|m| m["id"].as_i64());
        Ok(json!({"messages":out}))
    }

    async fn contacts(&self) -> Reply {
        let ids = self.request(json!({"@type":"getContacts"})).await?;
        let mut contacts = Vec::new();
        for id in array(&ids["user_ids"]).iter().filter_map(Value::as_i64) {
            if let Ok(u) = self.user(id).await {
                contacts.push(json!({"id":id,"name":user_name(&u),"phone":field(&u,"phone_number",json!("")),
                    "username":text(&u["usernames"]["active_usernames"][0])}));
            }
        }
        contacts.sort_by_cached_key(|u| text(&u["name"]).to_lowercase());
        Ok(json!({"contacts":contacts}))
    }

    async fn open_private(&self, user: i64) -> Reply {
        let chat = self.request(json!({"@type":"createPrivateChat","user_id":user,"force":false})).await?;
        Ok(json!({"chat_id":chat["id"]}))
    }

    async fn route(&self, method: String, path: String, query: String, body: Value) -> Reply {
        let q = Query(&query);
        self.account(&q.get("account").unwrap_or_default())?;
        let segments: Vec<_> = path.split('/').collect();
        match (method.as_str(), segments.as_slice()) {
            ("GET", ["", "api", "accounts"]) => {
                let (name, status) = match self.me().await {
                    Ok(m) => (text(&m["name"]).to_owned(), "online"),
                    Err(_) => (String::new(), "offline"),
                };
                Ok(json!({"tai_khoan":[{"phone":self.account,"display_name":name,"status":status}]}))
            }
            ("GET", ["", "api", "me"]) => self.me().await,
            ("GET", ["", "api", "profile"]) => self.profile().await,
            ("POST", ["", "api", "profile"]) => self.update_profile(&body).await,
            ("GET", ["", "api", "chats"]) => self.chats(q.int("limit", 60)?.clamp(1, 200)).await,
            ("GET", ["", "api", "chats", id, "messages"]) => self.history(
                path_id(id)?, q.int("before", 0)?, q.int("limit", 30)?.clamp(1, 100) as usize).await,
            ("GET", ["", "api", "chats", id, "info"]) => self.chat_info(path_id(id)?).await,
            ("GET", ["", "api", "chats", id, "members"]) => self.chat_members(
                path_id(id)?, &q.get("q").unwrap_or_default(), q.int("limit", 50)?).await,
            ("POST", ["", "api", "chats", id, "settings"]) => self.update_chat_settings(path_id(id)?, &body).await,
            ("POST", ["", "api", "chats", id, "clear_history"]) => self.clear_chat_history(path_id(id)?, &body).await,
            ("POST", ["", "api", "chats", id, "leave"]) => self.leave_chat(path_id(id)?, &body).await,
            ("GET", ["", "api", "contacts"]) => self.contacts().await,
            ("POST", ["", "api", "contacts", "open"]) => self.open_private(int(&body["user_id"], "user_id")?).await,
            ("GET", ["", "api", "search"]) => {
                let term = q.get("q").unwrap_or_default();
                if term.trim().is_empty() { return Ok(json!({"chats":[]})); }
                let result = self.request(json!({"@type":"searchChats","query":term.trim(),"limit":20})).await?;
                Ok(self.chat_list(&result["chat_ids"]).await)
            }
            ("GET", ["", "api", "feed"]) => {
                let wait = q.get("wait").filter(|s| !s.is_empty()).unwrap_or_else(|| "0".into());
                let wait: f64 = wait.trim().parse().map_err(|_| bad_number("wait"))?;
                let wait = if wait.is_nan() { 25.0 } else { wait.clamp(0.0, 25.0) };
                Ok(self.state.since(q.int("since", 0)?, Duration::from_secs_f64(wait)).await)
            }
            ("GET", ["", "api", "calltrace"]) => {
                let dir = self.data_dir.clone();
                tokio::task::spawn_blocking(move || crate::calls::trace::list(&dir)).await.ok()
                    .and_then(Result::ok).ok_or_else(|| GwError::new(500, "CALL_STORAGE", "Không đọc được danh sách nhật ký."))
            }
            ("POST", ["", "api", "chats", id, op @ ("send" | "send_file" | "send_album" | "read")]) =>
                self.write_chat(path_id(id)?, op, &q, &body).await,
            ("POST", ["", "api", "messages", op @ ("edit" | "delete" | "forward" | "react")]) =>
                self.write_message(op, &body).await,
            ("POST", ["", "api", "resolve"]) => self.resolve(&body).await,
            ("POST", ["", "api", "translate"]) => self.translate(&body).await,
            ("POST", ["", "api", "do", name]) => self.service(name, &body).await,
            ("POST", ["", "api", op @ ("send_voice" | "send_message" | "send_media")]) => self.legacy_send(op, &body).await,
            _ => Err(GwError::not_found("Không có đường dẫn này.")),
        }
    }
}

impl ChatControl for ChatService {
    fn handle(&self, method: &str, path: &str, query: &str, body: Value) -> BoxFut<'_, Reply> {
        Box::pin(self.route(method.to_owned(), path.to_owned(), query.to_owned(), body))
    }

    fn bytes(&self, path: &str, query: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        Box::pin(self.binary(path.to_owned(), query.to_owned()))
    }
}

impl Drop for ChatService {
    fn drop(&mut self) {
        self.updates.abort();
        if let Some(task) = &self.call_updates { task.abort(); }
    }
}

struct Query<'a>(&'a str);

impl Query<'_> {
    fn get(&self, name: &str) -> Option<String> {
        form_urlencoded::parse(self.0.as_bytes()).filter(|(k, _)| k == name).last().map(|(_, v)| v.into_owned())
    }

    fn int(&self, name: &str, default: i64) -> Result<i64, GwError> {
        self.get(name).map_or(Ok(default), |s| s.trim().parse().map_err(|_| bad_number(name)))
    }
}

fn bad_number(name: &str) -> GwError { GwError::bad_request(format!("Tham số {name} phải là số.")) }
fn path_id(id: &str) -> Result<i64, GwError> { id.parse().map_err(|_| bad_number("chat_id/file_id")) }

fn int(value: &Value, name: &str) -> Result<i64, GwError> {
    value.as_i64().or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .or_else(|| value.as_bool().map(i64::from))
        .or_else(|| value.as_number().filter(|n| n.is_f64()).and_then(|n| {
            let f = n.as_f64()?;
            (f >= i64::MIN as f64 && f < -(i64::MIN as f64)).then_some(f as i64)
        })).ok_or_else(|| GwError::bad_request(format!("{name} phải là số nguyên")))
}

fn text(v: &Value) -> &str { v.as_str().unwrap_or_default() }
fn string(v: &Value) -> String {
    if !truth(v) { return String::new(); }
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        _ => v.to_string(),
    }
}
fn array(v: &Value) -> &[Value] { v.as_array().map_or(&[], Vec::as_slice) }
fn field(v: &Value, key: &str, default: Value) -> Value { v.get(key).cloned().unwrap_or(default) }
fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

pub(crate) fn mask_phone(raw: &str) -> String {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let len = digits.len();
    if len == 0 {
        return String::new();
    }
    if len <= 4 {
        return format!("+{}", "•".repeat(len));
    }
    if len <= 6 {
        return format!("+{}{}{}", &digits[..2], "•".repeat(len - 4), &digits[len - 2..]);
    }
    format!("+{}{}{}", &digits[..3], "•".repeat(len - 6), &digits[len - 3..])
}

fn valid_username(u: &str) -> bool {
    if u.is_empty() {
        return true;
    }
    let bytes = u.as_bytes();
    (5..=32).contains(&bytes.len())
        && bytes[0].is_ascii_alphabetic()
        && bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        && !u.ends_with('_')
        && !u.contains("__")
}

fn telegram_error(error: crate::contract::TdError) -> GwError {
    if error.code.is_none() && error.message.starts_with("TDLib không trả lời ") {
        return GwError::new(504, "TELEGRAM_TIMEOUT", "Telegram không trả lời kịp.");
    }
    if error.code.is_none() && error.message.starts_with("mất kết nối TDLib") {
        return GwError::new(503, "ACCOUNT_OFFLINE", "Tài khoản đang mất kết nối tới Telegram.");
    }
    GwError::from_tdlib(error)
}
