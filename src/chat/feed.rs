use std::{collections::{HashMap, VecDeque}, sync::{Arc, Mutex}, time::{Duration, SystemTime, UNIX_EPOCH}};

use serde_json::{json, Value};
use tokio::{sync::{broadcast, Notify}, time::{timeout_at, Instant}};

use crate::events::{mask_peer_id, EventSink};
use super::{
    array, field,
    shape::{message_with_comments, user_avatar, user_name, CommentsContext, MessageContext},
    text, truth,
};

#[derive(Default)]
pub(super) struct State {
    inner: Mutex<Inner>,
    changed: Notify,
}

#[derive(Clone, Copy)]
struct AutoDeleteEntry {
    current: i64,
    last_chat_value: Option<i64>,
}

#[derive(Clone)]
pub(super) struct ChatMeta {
    pub title: String,
    pub is_private: bool,
    pub is_channel: bool,
    pub avatar_file: Option<i64>,
}

fn lru_put<V>(map: &mut HashMap<i64, V>, order: &mut VecDeque<i64>, id: i64, value: V) {
    if id == 0 { return; }
    map.insert(id, value);
    order.retain(|old| *old != id);
    order.push_back(id);
    if order.len() > 2000 {
        if let Some(evicted) = order.pop_front() {
            map.remove(&evicted);
        }
    }
}

fn lru_touch(order: &mut VecDeque<i64>, id: i64) {
    order.retain(|old| *old != id);
    order.push_back(id);
}

#[derive(Default)]
struct Inner {
    items: VecDeque<Value>,
    cursor: i64,
    users: HashMap<i64, Value>,
    user_order: VecDeque<i64>,
    supergroups: HashMap<i64, Value>,
    supergroup_order: VecDeque<i64>,
    basic_groups: HashMap<i64, Value>,
    basic_group_order: VecDeque<i64>,
    chats: HashMap<i64, ChatMeta>,
    chat_order: VecDeque<i64>,
    scope_mute_for: HashMap<&'static str, i64>,
    auto_delete: HashMap<i64, AutoDeleteEntry>,
    auto_delete_order: VecDeque<i64>,
    sent: VecDeque<(i64, i64)>,
}

impl Inner {
    fn remember_user(&mut self, user: &Value) {
        if let Some(id) = user["id"].as_i64().filter(|id| *id != 0) {
            lru_put(&mut self.users, &mut self.user_order, id, user.clone());
        }
    }

    fn update_user_status(&mut self, user_id: i64, status: &Value) {
        if user_id == 0 { return; }
        if let Some(user) = self.users.get_mut(&user_id) {
            user["status"] = status.clone();
            lru_touch(&mut self.user_order, user_id);
        }
    }

    fn remember_supergroup(&mut self, sg: &Value) {
        if let Some(id) = sg["id"].as_i64().filter(|id| *id != 0) {
            lru_put(&mut self.supergroups, &mut self.supergroup_order, id, sg.clone());
        }
    }

    fn remember_basic_group(&mut self, bg: &Value) {
        if let Some(id) = bg["id"].as_i64().filter(|id| *id != 0) {
            lru_put(&mut self.basic_groups, &mut self.basic_group_order, id, bg.clone());
        }
    }

    fn remember_chat(&mut self, chat: &Value) {
        let Some(id) = chat["id"].as_i64().filter(|id| *id != 0) else { return; };
        let ct = &chat["type"];
        let is_private = matches!(text(&ct["@type"]), "chatTypePrivate" | "chatTypeSecret");
        let is_channel = text(&ct["@type"]) == "chatTypeSupergroup" && truth(&ct["is_channel"]);
        let title = text(&chat["title"]).to_owned();
        let avatar_file = chat["photo"]["small"]["id"].as_i64().filter(|&fid| fid > 0);
        lru_put(&mut self.chats, &mut self.chat_order, id, ChatMeta { title, is_private, is_channel, avatar_file });
    }

    fn touch_auto_delete(&mut self, chat_id: i64) {
        lru_touch(&mut self.auto_delete_order, chat_id);
        if self.auto_delete_order.len() > 2000 {
            if let Some(evicted) = self.auto_delete_order.pop_front() {
                self.auto_delete.remove(&evicted);
            }
        }
    }

    fn set_auto_delete(&mut self, chat_id: i64, seconds: i64) {
        if chat_id == 0 { return; }
        let sec = seconds.max(0);
        let last_chat_value = self.auto_delete.get(&chat_id).and_then(|e| e.last_chat_value);
        self.auto_delete.insert(chat_id, AutoDeleteEntry { current: sec, last_chat_value });
        self.touch_auto_delete(chat_id);
    }

    fn sync_chat_auto_delete(&mut self, chat_id: i64, chat_seconds: i64) -> i64 {
        let sec = chat_seconds.max(0);
        if chat_id == 0 { return sec; }
        let out = match self.auto_delete.get_mut(&chat_id) {
            None => {
                self.auto_delete.insert(chat_id, AutoDeleteEntry { current: sec, last_chat_value: Some(sec) });
                sec
            }
            Some(entry) => {
                match entry.last_chat_value {
                    None => {
                        entry.last_chat_value = Some(sec);
                    }
                    Some(prev) if prev != sec => {
                        entry.last_chat_value = Some(sec);
                        entry.current = sec;
                    }
                    Some(_) => {}
                }
                entry.current
            }
        };
        self.touch_auto_delete(chat_id);
        out
    }

    fn normalize(&self, m: &Value) -> Value {
        let sender_obj = &m["sender_id"];
        let sender = match text(&sender_obj["@type"]) {
            "messageSenderUser" => sender_obj["user_id"]
                .as_i64()
                .and_then(|id| self.users.get(&id))
                .map(user_name)
                .unwrap_or_default(),
            "messageSenderChat" => sender_obj["chat_id"]
                .as_i64()
                .and_then(|id| self.chats.get(&id))
                .map(|c| c.title.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let chat_meta = m["chat_id"].as_i64().and_then(|id| self.chats.get(&id));
        let is_channel = chat_meta.map_or_else(|| truth(&m["is_channel_post"]), |c| c.is_channel);
        let resolver = |uid: i64| self.users.get(&uid).map(user_name);
        let user_replier = |uid: i64| self.users.get(&uid).map(|u| (user_name(&u), user_avatar(&u)));
        let chat_replier = |cid: i64| self.chats.get(&cid).map(|c| (c.title.clone(), c.avatar_file));
        let ctx = MessageContext {
            sender: &sender,
            chat_title: chat_meta.map_or("", |c| c.title.as_str()),
            is_private: chat_meta.is_some_and(|c| c.is_private),
            resolve_user: Some(&resolver),
        };
        let comments_ctx = CommentsContext {
            is_channel,
            resolve_user: Some(&user_replier),
            resolve_chat: Some(&chat_replier),
        };
        message_with_comments(m, &ctx, Some(&comments_ctx))
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
    pub(super) fn remember_supergroup(&self, sg: &Value) { self.inner.lock().unwrap().remember_supergroup(sg); }
    pub(super) fn remember_basic_group(&self, bg: &Value) { self.inner.lock().unwrap().remember_basic_group(bg); }
    pub(super) fn remember_chat(&self, chat: &Value) { self.inner.lock().unwrap().remember_chat(chat); }

    pub(super) fn set_auto_delete(&self, chat_id: i64, seconds: i64) {
        self.inner.lock().unwrap().set_auto_delete(chat_id, seconds);
    }

    pub(super) fn sync_chat_auto_delete(&self, chat_id: i64, chat_seconds: i64) -> i64 {
        self.inner.lock().unwrap().sync_chat_auto_delete(chat_id, chat_seconds)
    }

    pub(super) fn user(&self, id: i64) -> Option<Value> {
        let mut s = self.inner.lock().unwrap();
        let user = s.users.get(&id)?.clone();
        lru_touch(&mut s.user_order, id);
        Some(user)
    }

    pub(super) fn supergroup(&self, id: i64) -> Option<Value> {
        let mut s = self.inner.lock().unwrap();
        let sg = s.supergroups.get(&id)?.clone();
        lru_touch(&mut s.supergroup_order, id);
        Some(sg)
    }

    pub(super) fn basic_group(&self, id: i64) -> Option<Value> {
        let mut s = self.inner.lock().unwrap();
        let bg = s.basic_groups.get(&id)?.clone();
        lru_touch(&mut s.basic_group_order, id);
        Some(bg)
    }

    pub(super) fn chat_meta(&self, id: i64) -> Option<ChatMeta> {
        let mut s = self.inner.lock().unwrap();
        let meta = s.chats.get(&id)?.clone();
        lru_touch(&mut s.chat_order, id);
        Some(meta)
    }

    pub(super) fn scope_mute_for(&self, scope: &str) -> Option<i64> {
        self.inner.lock().unwrap().scope_mute_for.get(scope).copied()
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
            "updateMessageInteractionInfo" => json!({"type":"message_interaction","chat_id":u["chat_id"],"message_id":u["message_id"]}),
            "updateDeleteMessages" if truth(&u["is_permanent"]) => json!({"type":"deleted","chat_id":u["chat_id"],"ids":array(&u["message_ids"])}),
            "updateChatLastMessage" | "updateChatReadInbox" | "updateChatTitle" | "updateChatPhoto" | "updateChatPosition"
            | "updateChatNotificationSettings" | "updateChatUnreadMentionCount" | "updateChatIsMarkedAsUnread" => {
                let id = if truth(&u["chat_id"]) { &u["chat_id"] } else { &u["chat"]["id"] };
                if !truth(id) { return; }
                json!({"type":"chat","chat_id":id})
            }
            "updateChatReadOutbox" => {
                let Some(chat_id) = u["chat_id"].as_i64().filter(|id| *id != 0) else { return; };
                let last_id = u["last_read_outbox_message_id"].as_i64().unwrap_or(0).max(0);
                json!({"type":"chat_read_outbox","chat_id":chat_id,"last_read_outbox_message_id":last_id})
            }
            "updateNewChat" => {
                let id = &u["chat"]["id"];
                if !truth(id) { return; }
                s.remember_chat(&u["chat"]);
                if let (Some(chat_id), Some(sec)) = (id.as_i64(), u["chat"]["message_auto_delete_time"].as_i64()) {
                    s.sync_chat_auto_delete(chat_id, sec);
                }
                json!({"type":"chat","chat_id":id})
            }
            "updateChatMessageAutoDeleteTime" => {
                let Some(chat_id) = u["chat_id"].as_i64().filter(|id| *id != 0) else { return; };
                let seconds = u["message_auto_delete_time"].as_i64().unwrap_or(0).max(0);
                s.set_auto_delete(chat_id, seconds);
                json!({"type":"chat_auto_delete","chat_id":chat_id,"seconds":seconds})
            }
            "updateScopeNotificationSettings" => {
                let scope = match text(&u["scope"]["@type"]) {
                    "notificationSettingsScopePrivateChats" => "private",
                    "notificationSettingsScopeGroupChats" => "group",
                    "notificationSettingsScopeChannelChats" => "channel",
                    _ => return,
                };
                let mute_for = u["notification_settings"]["mute_for"].as_i64().unwrap_or(0).max(0);
                s.scope_mute_for.insert(scope, mute_for);
                json!({"type":"scope_notification","scope":scope,"mute_for":mute_for})
            }
            "updateUser" => { s.remember_user(&u["user"]); return; }
            "updateUserStatus" => {
                if let Some(uid) = u["user_id"].as_i64().filter(|id| *id != 0) {
                    s.update_user_status(uid, &u["status"]);
                }
                return;
            }
            "updateSupergroup" => { s.remember_supergroup(&u["supergroup"]); return; }
            "updateBasicGroup" => { s.remember_basic_group(&u["basic_group"]); return; }
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
        "updateChatMessageAutoDeleteTime" => {
            let Some(chat_id) = update["chat_id"].as_i64().filter(|id| *id != 0) else { return };
            let seconds = update["message_auto_delete_time"].as_i64().unwrap_or(0).max(0);
            events.emit("chat_auto_delete", json!({"chat_id":chat_id,"seconds":seconds}));
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
            "outgoing":false,"sender":"","kind":"text","text":"Xin chào","entities":[],"summary":"Xin chào","reply_to":0,"reactions":[],"file":null}));
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
            "edit_date":110,"outgoing":true,"sender":"An","kind":"photo","text":" chú thích ","entities":[],
            "summary":"🖼 Ảnh: chú thích","reply_to":8,"reactions":[{"emoji":"👍","count":2,"mine":true},
            {"emoji":"?","count":0,"mine":false}],"file":{"id":12,"size":1000,"path":"x.jpg",
                "width":800,"height":600,"mime":"image/jpeg"}}));
        let edit = &feed["items"][1];
        assert_eq!((&edit["type"], &edit["chat_id"], &edit["message_id"]), (&json!("message_changed"), &json!(3), &json!(9)));
    }

    #[tokio::test]
    async fn auto_delete_update_updates_state_and_emits_feed_event() {
        let s = State::default();
        assert_eq!(s.sync_chat_auto_delete(42, 0), 0);
        s.update(&json!({"@type":"updateChatMessageAutoDeleteTime","chat_id":42,"message_auto_delete_time":86400}));
        let feed = s.since(0, Duration::ZERO).await;
        assert_eq!(feed["cursor"], 1);
        assert_eq!(feed["items"][0]["type"], "chat_auto_delete");
        assert_eq!(feed["items"][0]["chat_id"], 42);
        assert_eq!(feed["items"][0]["seconds"], 86400);
        // Nếu getChat vẫn trả giá trị tĩnh cũ sau khi có sự kiện -> giữ giá trị từ sự kiện.
        assert_eq!(s.sync_chat_auto_delete(42, 0), 86400);
        // Nếu TDLib trả giá trị getChat mới khác lần đọc trước -> cập nhật theo TDLib.
        assert_eq!(s.sync_chat_auto_delete(42, 604800), 604800);
        // Sự kiện đến trước lần getChat đầu tiên vẫn giữ được giá trị mới.
        s.update(&json!({"@type":"updateChatMessageAutoDeleteTime","chat_id":99,"message_auto_delete_time":2678400}));
        assert_eq!(s.sync_chat_auto_delete(99, 0), 2678400);
        // updateNewChat đồng bộ giá trị khởi tạo.
        s.update(&json!({"@type":"updateNewChat","chat":{"id":105,"message_auto_delete_time":172800}}));
        assert_eq!(s.sync_chat_auto_delete(105, 172800), 172800);
        s.set_auto_delete(99, 0);
        assert_eq!(s.sync_chat_auto_delete(99, 0), 0);
    }

    #[tokio::test]
    async fn feed_updates_scope_notifications_user_status_supergroups_and_read_outbox() {
        let s = State::default();
        s.update(&json!({"@type":"updateUser","user":{"id":7,"first_name":"Đại","last_name":"ca","status":{"@type":"userStatusOffline","was_online":100}}}));
        s.update(&json!({"@type":"updateUserStatus","user_id":7,"status":{"@type":"userStatusOnline"}}));
        assert_eq!(s.user(7).unwrap()["status"]["@type"], "userStatusOnline");

        s.update(&json!({"@type":"updateSupergroup","supergroup":{"id":55,"member_count":1200,"is_verified":true}}));
        assert_eq!(s.supergroup(55).unwrap()["member_count"], 1200);
        s.update(&json!({"@type":"updateBasicGroup","basic_group":{"id":22,"member_count":8}}));
        assert_eq!(s.basic_group(22).unwrap()["member_count"], 8);

        s.update(&json!({"@type":"updateNewChat","chat":{"id":42,"title":"Đại ca","type":{"@type":"chatTypePrivate","user_id":7}}}));
        assert_eq!(s.chat_meta(42).unwrap().title, "Đại ca");
        assert!(s.chat_meta(42).unwrap().is_private);

        s.update(&json!({"@type":"updateScopeNotificationSettings","scope":{"@type":"notificationSettingsScopePrivateChats"},"notification_settings":{"mute_for":2147483647}}));
        assert_eq!(s.scope_mute_for("private"), Some(2147483647));

        s.update(&json!({"@type":"updateChatReadOutbox","chat_id":42,"last_read_outbox_message_id":999}));
        s.update(&json!({"@type":"updateNewMessage","message":{"id":100,"chat_id":42,"is_outgoing":false,"content":{"@type":"messageContactRegistered"}}}));

        let feed = s.since(0, Duration::ZERO).await;
        assert_eq!(feed["items"][0]["type"], "chat");
        assert_eq!(feed["items"][1]["type"], "scope_notification");
        assert_eq!(feed["items"][1]["scope"], "private");
        assert_eq!(feed["items"][2]["type"], "chat_read_outbox");
        assert_eq!(feed["items"][2]["chat_id"], 42);
        assert_eq!(feed["items"][2]["last_read_outbox_message_id"], 999);
        assert_eq!(feed["items"][3]["message"]["kind"], "service");
        assert_eq!(feed["items"][3]["message"]["summary"], "Đại ca đã tham gia Telegram");
    }
}
