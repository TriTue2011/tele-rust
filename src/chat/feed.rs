use std::{collections::{HashMap, VecDeque}, sync::{Arc, Mutex}, time::{Duration, SystemTime, UNIX_EPOCH}};

use serde_json::{json, Value};
use tokio::{sync::{broadcast, Notify}, time::{timeout_at, Instant}};

use crate::events::{mask_peer_id, EventSink};
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
    pub(super) async fn receive_calls(self: Arc<Self>, mut source: broadcast::Receiver<Value>) {
        loop {
            match source.recv().await {
                Ok(event) => {
                    if !matches!(event["kind"].as_str(), Some("incoming" | "active" | "peer_video" | "ended")) { continue; }
                    let mut s = self.inner.lock().unwrap();
                    self.push(&mut s, json!({"type":"call","event":event["kind"],"call":event["call"]}));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

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

    pub(super) async fn receive(self: Arc<Self>, mut updates: broadcast::Receiver<Value>, events: Option<EventSink>) {
        loop {
            match updates.recv().await {
                Ok(update) => {
                    self.update(&update);
                    if let Some(events) = &events {
                        events.account_update(&update);
                        bridge_update(events, &update);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    fn update(&self, u: &Value) {
        let mut s = self.inner.lock().unwrap();
        let m = &u["message"];
        let old = &u["old_message_id"];
        let event = match text(&u["@type"]) {
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
        self.push(&mut s, event);
    }

    fn push(&self, s: &mut Inner, mut event: Value) {
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

fn bridge_update(events: &EventSink, update: &Value) {
    let message = &update["message"];
    match text(&update["@type"]) {
        "updateNewMessage" if !message["sending_state"].is_null() => (),
        "updateNewMessage" | "updateMessageSendSucceeded" => {
            let Some(id) = message["id"].as_i64() else { return };
            let peer = mask_peer_id(&message["chat_id"]);
            events.emit("message_new", json!({"peer":peer,"tin_id":id.to_string()}));
            if message["content"]["@type"] == "messageContactRegistered" {
                events.emit("contact_joined", json!({"peer":peer}));
            }
        }
        "updateMessageContent" | "updateMessageEdited" => {
            let Some(id) = update["message_id"].as_i64() else { return };
            events.emit("message_edited", json!({"chat_id":update["chat_id"],"message_id":id}));
        }
        "updateDeleteMessages" if update["is_permanent"] == true => {
            events.emit("message_deleted", json!({"chat_id":update["chat_id"],"ids":array(&update["message_ids"])}));
        }
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn call_feed_uses_python_envelope_and_chat_feed_keeps_all_message_keys() {
        let state = Arc::new(State::default());
        let (tx, rx) = broadcast::channel(8);
        let task = tokio::spawn(state.clone().receive_calls(rx));
        for kind in ["incoming", "started", "active", "peer_video", "ended"] {
            tx.send(json!({"kind":kind,"call":{"id":"abcdef","state":kind}})).unwrap();
        }
        drop(tx);
        task.await.unwrap();
        let feed = state.since(0, Duration::ZERO).await;
        assert_eq!(feed["cursor"], 4);
        for (i, kind) in ["incoming", "active", "peer_video", "ended"].iter().enumerate() {
            let item = &feed["items"][i];
            assert_eq!(item["type"], "call"); assert_eq!(item["event"], *kind);
            assert_eq!(item["call"], json!({"id":"abcdef","state":kind}));
            assert!(item["ts"].is_f64());
            assert_eq!(item.as_object().unwrap().len(), 5);
        }
        state.update(&json!({"@type":"updateNewMessage","message":{"id":17,"chat_id":5,"date":100,"edit_date":101,
            "is_outgoing":false,"content":{"@type":"messageText","text":{"text":"Xin chào"}}}}));
        let feed = state.since(4, Duration::ZERO).await;
        assert_eq!(feed["items"][0]["message"], json!({"id":17,"chat_id":5,"date":100,"edit_date":101,
            "outgoing":false,"sender":"","kind":"text","text":"Xin chào","summary":"Xin chào","reply_to":0,"reactions":[],"file":null}));
    }

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

    #[tokio::test]
    async fn photo_reactions_and_edit_metadata_match_python_feed() {
        let s = State::default();
        s.update(&json!({"@type":"updateUser","user":{"id":7,"first_name":"An"}}));
        s.update(&json!({"@type":"updateNewMessage","message":{
            "id":9,"chat_id":3,"date":100,"edit_date":110,"is_outgoing":true,
            "sender_id":{"@type":"messageSenderUser","user_id":7},"reply_to":{"message_id":8},
            "interaction_info":{"reactions":{"reactions":[
                {"type":{"emoji":"👍"},"total_count":2,"is_chosen":true}, {"type":{"@type":"reactionTypeCustomEmoji"}}]}},
            "content":{"@type":"messagePhoto","caption":{"text":" chú thích "},"photo":{"sizes":[
                {"width":90,"height":60,"photo":{"id":11}},
                {"width":800,"height":600,"photo":{"id":12,"size":0,"expected_size":1000,
                    "local":{"is_downloading_completed":true,"path":"x.jpg"}}},
                {"width":2000,"height":1500,"photo":{"id":13}}]}}
        }}));
        s.update(&json!({"@type":"updateMessageEdited","chat_id":3,"message_id":9}));
        let feed = s.since(0, Duration::ZERO).await;
        assert_eq!(feed["cursor"], 2);
        assert_eq!(feed["items"][0]["message"], json!({"id":9,"chat_id":3,"date":100,
            "edit_date":110,"outgoing":true,"sender":"An","kind":"photo","text":" chú thích ",
            "summary":"🖼 Ảnh: chú thích","reply_to":8,"reactions":[{"emoji":"👍","count":2,"mine":true},
            {"emoji":"?","count":0,"mine":false}],"file":{"id":12,"size":1000,"path":"x.jpg",
                "width":800,"height":600,"mime":"image/jpeg"}}));
        let edit = &feed["items"][1];
        assert_eq!((&edit["type"], &edit["chat_id"], &edit["message_id"]), (&json!("message_changed"), &json!(3), &json!(9)));
    }
}
