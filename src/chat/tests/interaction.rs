//! Số liệu tương tác tươi: gateway giữ chat mở với TDLib và biến updateMessageInteractionInfo thành sự kiện feed.
use super::*;

fn history_req(chat: i64) -> Value {
    json!({"@type":"getChatHistory","chat_id":chat,"from_message_id":0,"offset":0,"limit":3,"only_local":false})
}

fn opened(td: &FakeTd) -> Vec<(String, i64)> { td.background.lock().unwrap().clone() }

#[tokio::test]
async fn history_opens_each_chat_once_and_closes_the_oldest_beyond_three() {
    let empty = |chat| ok(history_req(chat), json!({"messages": []}));
    let (chat, td) = setup(vec![empty(1), empty(1), empty(2), empty(3), empty(4)]);
    for id in [1, 1, 2, 3, 4] {
        get(&chat, &format!("/api/chats/{id}/messages"), "limit=3").await.unwrap();
    }
    let seen = opened(&td);
    assert_eq!(seen.iter().filter(|(k, c)| k == "openChat" && *c == 1).count(), 1, "mở lại chat đã mở trong 120 s là thừa: {seen:?}");
    assert_eq!(seen.iter().filter(|(k, _)| k == "openChat").count(), 4);
    assert_eq!(seen.iter().filter(|(k, _)| k == "closeChat").cloned().collect::<Vec<_>>(), vec![("closeChat".to_owned(), 1)],
        "chat mở lâu nhất (1) phải bị đóng khi mở chat thứ tư: {seen:?}");
    assert_eq!(seen[0], ("openChat".to_owned(), 1), "openChat phải đi trước khi đọc lịch sử");
}

#[tokio::test]
async fn interaction_info_update_becomes_message_changed_feed_event() {
    let (chat, td) = setup(vec![]);
    let updater = async {
        tokio::task::yield_now().await;
        td.updates.send(json!({"@type":"updateMessageInteractionInfo","chat_id":-100,"message_id":42,
            "interaction_info":{"reactions":{"reactions":[{"type":{"@type":"reactionTypeEmoji","emoji":"👏"},"total_count":25}]}}})).unwrap();
    };
    let (feed, ()) = tokio::join!(get(&chat, "/api/feed", "since=0&wait=1"), updater);
    let feed = feed.unwrap();
    assert_eq!(feed["items"][0]["type"], "message_changed");
    assert_eq!((feed["items"][0]["chat_id"].as_i64(), feed["items"][0]["message_id"].as_i64()), (Some(-100), Some(42)));
}
