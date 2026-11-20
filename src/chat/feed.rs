use std::{collections::{HashMap, VecDeque}, sync::{Arc, Mutex}, time::{Duration, SystemTime, UNIX_EPOCH}};

use serde_json::{json, Value};
use tokio::{sync::{broadcast, Notify}, time::{timeout_at, Instant}};

use super::{array, field, shape::{message, user_name}, text, truth};

#[derive(Default)]
pub(super) struct State {
    inner: Mutex<Inner>,
    changed: Notify,
}

#[derive(Default)]
struct Inner {
    items: VecDeque<Value>,
    cursor: i64,
    users: HashMap<i64, Value>,
    user_order: VecDeque<i64>,
    sent: VecDeque<(i64, i64)>,
}

impl Inner {
    fn remember_user(&mut self, user: &Value) {
        if let Some(id) = user["id"].as_i64().filter(|id| *id != 0) {
            self.users.insert(id, user.clone());
            self.touch_user(id);
            if self.user_order.len() > 2000 {
                self.users.remove(&self.user_order.pop_front().unwrap());
            }
        }
    }

    fn touch_user(&mut self, id: i64) {
        self.user_order.retain(|old| *old != id);
        self.user_order.push_back(id);
    }

    fn normalize(&self, m: &Value) -> Value {
        let sender = &m["sender_id"];
        let name = if sender["@type"] == "messageSenderUser" {
            sender["user_id"].as_i64().and_then(|id| self.users.get(&id)).map(user_name).unwrap_or_default()
        } else { String::new() };
        message(m, &name)
    }
}

impl State {
    pub(super) fn remember_user(&self, user: &Value) { self.inner.lock().unwrap().remember_user(user); }

    pub(super) fn user(&self, id: i64) -> Option<Value> {
        let mut s = self.inner.lock().unwrap();
        let user = s.users.get(&id)?.clone();
        s.touch_user(id);
        Some(user)
    }

    pub(super) async fn final_id(&self, id: i64, wait: Duration) -> i64 {
        let deadline = Instant::now() + wait;
        loop {
            let notified = self.changed.notified();
            if let Some((_, final_id)) = self.inner.lock().unwrap().sent.iter().find(|(old, _)| *old == id) {
                return *final_id;
            }
            if id & 0xfffff == 0 || timeout_at(deadline, notified).await.is_err() { return id; }
        }
    }

    pub(super) async fn receive(self: Arc<Self>, mut updates: broadcast::Receiver<Value>) {
        loop {
            match updates.recv().await {
                Ok(update) => self.update(&update),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    fn update(&self, u: &Value) {
        let mut s = self.inner.lock().unwrap();
        let m = &u["message"];
        let old = &u["old_message_id"];
        let mut event = match text(&u["@type"]) {
            "updateNewMessage" => json!({"type":"message","chat_id":m["chat_id"],"message":s.normalize(m)}),
            "updateMessageSendSucceeded" => {
                if let (Some(old), Some(new)) = (old.as_i64().filter(|i| *i != 0), m["id"].as_i64().filter(|i| *i != 0)) {
                    if let Some(entry) = s.sent.iter_mut().find(|(id, _)| *id == old) {
                        entry.1 = new;
                    } else {
                        s.sent.push_back((old, new));
                        if s.sent.len() > 500 { s.sent.pop_front(); }
                    }
                }
                json!({"type":"send_ok","chat_id":m["chat_id"],"old_id":old,"message":s.normalize(m)})
            }
            "updateMessageSendFailed" => json!({"type":"send_failed","chat_id":m["chat_id"],"old_id":old,
                "error":field(&u["error"],"message",json!("gửi không được"))}),
            "updateMessageContent" | "updateMessageEdited" => json!({"type":"message_changed","chat_id":u["chat_id"],"message_id":u["message_id"]}),
            "updateDeleteMessages" if truth(&u["is_permanent"]) => json!({"type":"deleted","chat_id":u["chat_id"],"ids":array(&u["message_ids"])}),
            "updateChatLastMessage" | "updateChatReadInbox" | "updateChatTitle" | "updateChatPhoto" | "updateChatPosition"
            | "updateNewChat" | "updateChatNotificationSettings" | "updateChatUnreadMentionCount" => {
                let id = if truth(&u["chat_id"]) { &u["chat_id"] } else { &u["chat"]["id"] };
                if !truth(id) { return; }
                json!({"type":"chat","chat_id":id})
            }
            "updateUser" => { s.remember_user(&u["user"]); return; }
            "updateChatAction" => json!({"type":"typing","chat_id":u["chat_id"],"action":text(&u["action"]["@type"])}),
            _ => return,
        };
        s.cursor += 1;
        event["id"] = json!(s.cursor);
        event["ts"] = json!(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64());
        s.items.push_back(event);
        if s.items.len() > 2000 { s.items.pop_front(); }
        self.changed.notify_waiters();
    }

    pub(super) async fn since(&self, cursor: i64, wait: Duration) -> Value {
        let deadline = Instant::now() + wait;
        loop {
            let notified = self.changed.notified();
            {
                let s = self.inner.lock().unwrap();
                if cursor < s.cursor || Instant::now() >= deadline {
                    let items: Vec<_> = s.items.iter().filter(|m| m["id"].as_i64().unwrap() > cursor).collect();
                    let first = s.cursor + 1 - s.items.len() as i64;
                    return json!({"items":items,"cursor":s.cursor,"reset":cursor>0 && cursor<first-1});
                }
            }
            let _ = timeout_at(deadline, notified).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reset_cursor_and_provisional_id_map_are_bounded() {
        let s = State::default();
        for n in 1..=2002 {
            s.update(&json!({"@type":"updateMessageSendSucceeded","old_message_id":n,
                "message":{"id":n<<20,"chat_id":5}}));
        }
        let out = s.since(1, Duration::ZERO).await;
        assert_eq!(out["cursor"],2002);
        assert_eq!(out["reset"],true);
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(),2000);
        assert_eq!(items[0]["id"],3);
        assert_eq!(s.final_id(2002, Duration::ZERO).await,2002<<20);
        assert_eq!(s.final_id(1, Duration::ZERO).await,1);
    }
}
