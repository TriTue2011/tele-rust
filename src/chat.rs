//! HTTP chat cho một tài khoản TDLib đã đăng nhập.
mod feed;
mod services;
mod shape;
mod write;
#[cfg(test)]
mod tests;

use std::{path::{Path, PathBuf}, sync::Arc, time::Duration};

use serde_json::{json, Value};

use crate::contract::{BoxFut, ChatControl, GwError, TdLink};
use feed::State;
use shape::{message, summarize, user_name};

type Reply = Result<Value, GwError>;

pub struct ChatService {
    td: Arc<dyn TdLink>,
    account: String,
    uploads: PathBuf,
    state: Arc<State>,
    updates: tokio::task::JoinHandle<()>,
}

impl ChatService {
    /// Tạo trong Tokio runtime của gateway, dùng số tài khoản mà tầng đăng nhập đã chọn.
    pub fn new(td: Arc<dyn TdLink>, account: impl Into<String>, data_dir: impl AsRef<Path>) -> Self {
        let state = Arc::new(State::default());
        let updates = tokio::spawn(state.clone().receive(td.updates()));
        Self { td, account: account.into(), uploads: data_dir.as_ref().join("uploads"), state, updates }
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
                chats.push(summarize(&chat));
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
            ("GET", ["", "api", "chats"]) => self.chats(q.int("limit", 60)?.clamp(1, 200)).await,
            ("GET", ["", "api", "chats", id, "messages"]) => self.history(
                path_id(id)?, q.int("before", 0)?, q.int("limit", 30)?.clamp(1, 100) as usize).await,
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
            ("GET", ["", "api", "file" | "files", id]) => {
                path_id(id)?;
                Err(no_bytes())
            }
            ("GET", ["", "api", "live", "view" | "snapshot", _]) => Err(no_bytes()),
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
}

impl Drop for ChatService {
    fn drop(&mut self) { self.updates.abort(); }
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

fn no_bytes() -> GwError {
    GwError::new(501, "NOT_IMPLEMENTED", "ChatControl chỉ trả JSON; đường này cần phản hồi tệp nhị phân hoặc luồng.")
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
