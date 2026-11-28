use std::collections::HashSet;

use serde_json::{json, Value};

use crate::chat::shape::{
    message, message_with_ctx, summarize, summarize_with_ctx, MessageContext, SummarizeContext,
    DEFAULT_SERVICE_LABEL,
};

const TDLIB_1_8_69_MESSAGE_TYPES: &[&str] = &[
    "messageAnimatedEmoji",
    "messageAnimation",
    "messageAudio",
    "messageBasicGroupChatCreate",
    "messageBotWriteAccessAllowed",
    "messageCall",
    "messageChatAddMembers",
    "messageChatBoost",
    "messageChatChangePhoto",
    "messageChatChangeTitle",
    "messageChatDeleteMember",
    "messageChatDeletePhoto",
    "messageChatHasProtectedContentDisableRequested",
    "messageChatHasProtectedContentToggled",
    "messageChatJoinByLink",
    "messageChatJoinByRequest",
    "messageChatOwnerChanged",
    "messageChatOwnerLeft",
    "messageChatSetBackground",
    "messageChatSetMessageAutoDeleteTime",
    "messageChatSetTheme",
    "messageChatSetTtl",
    "messageChatShared",
    "messageChatUpgradeFrom",
    "messageChatUpgradeTo",
    "messageChecklist",
    "messageChecklistTasksAdded",
    "messageChecklistTasksDone",
    "messageContact",
    "messageContactRegistered",
    "messageCustomServiceAction",
    "messageDice",
    "messageDirectMessagePriceChanged",
    "messageDocument",
    "messageExpiredPhoto",
    "messageExpiredVideo",
    "messageExpiredVideoNote",
    "messageExpiredVoiceNote",
    "messageForumTopicCreated",
    "messageForumTopicEdited",
    "messageForumTopicIsClosedToggled",
    "messageForumTopicIsHiddenToggled",
    "messageGame",
    "messageGameScore",
    "messageGift",
    "messageGiftedPremium",
    "messageGiftedStars",
    "messageGiftedTon",
    "messageGiveaway",
    "messageGiveawayCompleted",
    "messageGiveawayCreated",
    "messageGiveawayPrizeStars",
    "messageGiveawayWinners",
    "messageGroupCall",
    "messageInviteVideoChatParticipants",
    "messageInvoice",
    "messageLiveStory",
    "messageLocation",
    "messageManagedBotCreated",
    "messagePaidMedia",
    "messagePaidMessagePriceChanged",
    "messagePaidMessagesRefunded",
    "messagePassportDataReceived",
    "messagePassportDataSent",
    "messagePaymentRefunded",
    "messagePaymentSuccessful",
    "messagePaymentSuccessfulBot",
    "messagePhoto",
    "messagePinMessage",
    "messagePoll",
    "messagePollOptionAdded",
    "messagePollOptionDeleted",
    "messagePremiumGiftCode",
    "messageProximityAlertTriggered",
    "messageRefundedUpgradedGift",
    "messageScreenshotTaken",
    "messageStakeDice",
    "messageSticker",
    "messageStory",
    "messageSuggestBirthdate",
    "messageSuggestProfilePhoto",
    "messageSuggestedPostApprovalFailed",
    "messageSuggestedPostApproved",
    "messageSuggestedPostDeclined",
    "messageSuggestedPostPaid",
    "messageSuggestedPostRefunded",
    "messageSupergroupChatCreate",
    "messageText",
    "messageUnsupported",
    "messageUpgradedGift",
    "messageUpgradedGiftPurchaseOffer",
    "messageUpgradedGiftPurchaseOfferRejected",
    "messageUserShared",
    "messageUsersShared",
    "messageVenue",
    "messageVideo",
    "messageVideoChatEnded",
    "messageVideoChatScheduled",
    "messageVideoChatStarted",
    "messageVideoNote",
    "messageVoiceNote",
    "messageWebAppDataReceived",
    "messageWebAppDataSent",
    "messageWebsiteConnected",
];

fn sample_content(ty: &str) -> Value {
    if ty == "messageText" {
        json!({"@type": ty, "text": {"text": "Xin chào"}})
    } else {
        json!({"@type": ty})
    }
}

fn assert_explicit_label(ty: &str, text_label: &str) {
    assert!(!text_label.trim().is_empty(), "{ty} trả về nhãn rỗng");
    assert!(
        !(text_label.starts_with('[') && text_label.ends_with(']')),
        "{ty} bị in dạng thô: {text_label}"
    );
    let legacy_raw = format!("[{}]", ty.replacen("message", "", 1).to_lowercase());
    assert_ne!(text_label, legacy_raw, "{ty} vẫn dùng chuỗi thô {legacy_raw}");
    assert_ne!(
        text_label, DEFAULT_SERVICE_LABEL,
        "{ty} đã biết trong TDLib 1.8.69 nhưng rơi về nhãn mặc định"
    );
}

#[test]
fn all_tdlib_1_8_69_message_types_have_explicit_vietnamese_labels_never_raw_brackets() {
    let unique: HashSet<&str> = TDLIB_1_8_69_MESSAGE_TYPES.iter().copied().collect();
    assert_eq!(unique.len(), TDLIB_1_8_69_MESSAGE_TYPES.len());

    for &ty in TDLIB_1_8_69_MESSAGE_TYPES {
        let content = sample_content(ty);
        let msg = json!({"id": 1, "chat_id": 5, "content": content});
        let formatted = message(&msg, "An");
        let summary = formatted["summary"].as_str().expect("summary là chuỗi");
        assert_explicit_label(ty, summary);

        let chat = json!({
            "id": 5,
            "title": "Nhóm",
            "type": {"@type": "chatTypePrivate", "user_id": 7},
            "last_message": msg,
        });
        let summarized = summarize(&chat);
        let last_text = summarized["last"]["text"].as_str().expect("last.text là chuỗi");
        assert_explicit_label(ty, last_text);
    }

    if let Ok(path) = std::env::var("TD_API_TL") {
        let tl = std::fs::read_to_string(&path).expect("đọc được TD_API_TL");
        let mut count = 0usize;
        for line in tl.lines() {
            let trimmed = line.trim();
            if !trimmed.ends_with("= MessageContent;") {
                continue;
            }
            let Some(ty) = trimmed.split_whitespace().next() else {
                continue;
            };
            if !ty.starts_with("message") {
                continue;
            }
            count += 1;
            let content = sample_content(ty);
            let formatted = message(&json!({"id": 1, "chat_id": 5, "content": content}), "An");
            let summary = formatted["summary"].as_str().expect("summary là chuỗi");
            assert_explicit_label(ty, summary);
        }
        assert!(count >= 90, "TD_API_TL có quá ít MessageContent: {count}");
    }
}

#[test]
fn unknown_or_future_message_types_fallback_to_service_never_raw_type() {
    for unknown in [
        "messageFutureFeature2099",
        "messageSomeUnknownService",
        "messageBrandNewAction",
        "unexpectedType",
    ] {
        let msg = json!({"id": 2, "chat_id": 5, "content": {"@type": unknown}});
        let formatted = message(&msg, "An");
        assert_eq!(formatted["kind"], "service", "{unknown}");
        assert_eq!(formatted["summary"], DEFAULT_SERVICE_LABEL, "{unknown}");

        let chat = json!({
            "id": 5,
            "title": "Nhóm",
            "type": {"@type": "chatTypeBasicGroup"},
            "last_message": msg,
        });
        let summarized = summarize(&chat);
        assert_eq!(summarized["last"]["kind"], "service", "{unknown}");
        assert_eq!(summarized["last"]["text"], DEFAULT_SERVICE_LABEL, "{unknown}");
    }

    for empty_content in [json!({}), json!({"@type": ""})] {
        let formatted = message(&json!({"id": 3, "chat_id": 5, "content": empty_content}), "An");
        assert_eq!(formatted["kind"], "other");
        assert_eq!(formatted["summary"], "");
    }
}

#[test]
fn dynamic_content_and_service_messages_format_titles_booleans_and_actors() {
    let venue = message(
        &json!({"content": {"@type": "messageVenue", "venue": {"title": " Quán A "}}}),
        "",
    );
    assert_eq!((&venue["kind"], &venue["summary"]), (&json!("location"), &json!("📍 Địa điểm «Quán A»")));
    let venue_empty = message(&json!({"content": {"@type": "messageVenue"}}), "");
    assert_eq!(
        (&venue_empty["kind"], &venue_empty["summary"]),
        (&json!("location"), &json!("📍 Địa điểm"))
    );

    let anim_emoji = message(&json!({"content": {"@type": "messageAnimatedEmoji", "emoji": "🔥"}}), "");
    assert_eq!((&anim_emoji["kind"], &anim_emoji["summary"]), (&json!("other"), &json!("🔥")));
    let anim_fallback = message(&json!({"content": {"@type": "messageAnimatedEmoji", "emoji": "   "}}), "");
    assert_eq!(
        (&anim_fallback["kind"], &anim_fallback["summary"]),
        (&json!("other"), &json!("✨ Biểu tượng động"))
    );

    let dice = message(&json!({"content": {"@type": "messageDice", "emoji": "🎯"}}), "");
    assert_eq!((&dice["kind"], &dice["summary"]), (&json!("other"), &json!("🎯")));
    let dice_fallback = message(&json!({"content": {"@type": "messageDice"}}), "");
    assert_eq!((&dice_fallback["kind"], &dice_fallback["summary"]), (&json!("other"), &json!("🎲 Xúc xắc")));
    let stake_dice = message(&json!({"content": {"@type": "messageStakeDice"}}), "");
    assert_eq!((&stake_dice["kind"], &stake_dice["summary"]), (&json!("other"), &json!("🎲 Xúc xắc")));

    let game = message(&json!({"content": {"@type": "messageGame", "game": {"title": "Cờ vua"}}}), "");
    assert_eq!((&game["kind"], &game["summary"]), (&json!("other"), &json!("🎮 Trò chơi «Cờ vua»")));
    let game_empty = message(&json!({"content": {"@type": "messageGame"}}), "");
    assert_eq!((&game_empty["kind"], &game_empty["summary"]), (&json!("other"), &json!("🎮 Trò chơi")));

    let paid_cap = message(
        &json!({"content": {"@type": "messagePaidMedia", "caption": {"text": "Ảnh VIP"}}}),
        "",
    );
    assert_eq!(
        (&paid_cap["kind"], &paid_cap["summary"]),
        (&json!("other"), &json!("🔒 Nội dung trả phí: Ảnh VIP"))
    );
    let paid_plain = message(&json!({"content": {"@type": "messagePaidMedia"}}), "");
    assert_eq!(
        (&paid_plain["kind"], &paid_plain["summary"]),
        (&json!("other"), &json!("🔒 Nội dung trả phí"))
    );

    let story = message(&json!({"content": {"@type": "messageStory"}}), "");
    assert_eq!((&story["kind"], &story["summary"]), (&json!("other"), &json!("⭕ Tin 24 giờ")));
    let live = message(&json!({"content": {"@type": "messageLiveStory"}}), "");
    assert_eq!((&live["kind"], &live["summary"]), (&json!("other"), &json!("🔴 Phát trực tiếp")));

    let cl_text = message(
        &json!({"content": {"@type": "messageChecklist", "list": {"title": {"text": "Mua đồ"}}}}),
        "",
    );
    assert_eq!(
        (&cl_text["kind"], &cl_text["summary"]),
        (&json!("other"), &json!("☑️ Danh sách việc «Mua đồ»"))
    );
    let cl_str = message(
        &json!({"content": {"@type": "messageChecklist", "list": {"title": "Việc nhà"}}}),
        "",
    );
    assert_eq!(
        (&cl_str["kind"], &cl_str["summary"]),
        (&json!("other"), &json!("☑️ Danh sách việc «Việc nhà»"))
    );
    let cl_empty = message(&json!({"content": {"@type": "messageChecklist"}}), "");
    assert_eq!(
        (&cl_empty["kind"], &cl_empty["summary"]),
        (&json!("other"), &json!("☑️ Danh sách việc"))
    );

    let ttl_set = message(
        &json!({"is_outgoing": true, "content": {"@type": "messageChatSetTtl", "ttl": 86400}}),
        "Bình",
    );
    assert_eq!(
        (&ttl_set["kind"], &ttl_set["summary"]),
        (&json!("service"), &json!("Bạn đã đặt hẹn giờ tự hủy tin nhắn sau 1 ngày"))
    );
    let ttl_off = message(
        &json!({"is_outgoing": false, "content": {"@type": "messageChatSetTtl", "ttl": 0}}),
        "An",
    );
    assert_eq!(
        (&ttl_off["kind"], &ttl_off["summary"]),
        (&json!("service"), &json!("An đã tắt hẹn giờ tự hủy tin nhắn"))
    );
    let ttl_none = message(
        &json!({"is_outgoing": false, "content": {"@type": "messageChatSetTtl"}}),
        "An",
    );
    assert_eq!(
        (&ttl_none["kind"], &ttl_none["summary"]),
        (&json!("service"), &json!("An đã đổi hẹn giờ tự hủy tin nhắn"))
    );

    let bg_create = message(
        &json!({"is_outgoing": false, "content": {"@type": "messageBasicGroupChatCreate", "title": "Gia đình"}}),
        "An",
    );
    assert_eq!(
        (&bg_create["kind"], &bg_create["summary"]),
        (&json!("service"), &json!("An đã tạo nhóm «Gia đình»"))
    );
    let sg_create_no_title = message(
        &json!({"is_outgoing": true, "content": {"@type": "messageSupergroupChatCreate"}}),
        "An",
    );
    assert_eq!(
        (&sg_create_no_title["kind"], &sg_create_no_title["summary"]),
        (&json!("service"), &json!("Bạn đã tạo nhóm"))
    );

    for (content, want) in [
        (json!({"@type": "messageForumTopicCreated", "name": "Hỏi đáp"}), "💬 Đã tạo chủ đề «Hỏi đáp»"),
        (json!({"@type": "messageForumTopicEdited", "name": "Tin mới"}), "💬 Đã sửa chủ đề thành «Tin mới»"),
        (json!({"@type": "messageForumTopicIsClosedToggled", "is_closed": true}), "🔒 Đã đóng chủ đề"),
        (json!({"@type": "messageForumTopicIsClosedToggled", "is_closed": false}), "🔓 Đã mở lại chủ đề"),
        (json!({"@type": "messageForumTopicIsClosedToggled"}), "💬 Đã đổi trạng thái chủ đề"),
        (json!({"@type": "messageForumTopicIsHiddenToggled", "is_hidden": true}), "🙈 Đã ẩn chủ đề"),
        (json!({"@type": "messageForumTopicIsHiddenToggled", "is_hidden": false}), "👁 Đã hiện chủ đề"),
        (json!({"@type": "messageForumTopicIsHiddenToggled"}), "💬 Đã đổi hiển thị chủ đề"),
        (json!({"@type": "messageChatHasProtectedContentToggled", "has_protected_content": true}), "🔒 Đã bật bảo vệ nội dung"),
        (json!({"@type": "messageChatHasProtectedContentToggled", "has_protected_content": false}), "🔓 Đã tắt bảo vệ nội dung"),
        (json!({"@type": "messageChatHasProtectedContentToggled"}), "🛡 Đã đổi bảo vệ nội dung"),
        (json!({"@type": "messageCustomServiceAction", "text": " Bảo trì "}), "ℹ️ Bảo trì"),
        (json!({"@type": "messageCustomServiceAction", "text": "   "}), "ℹ️ Thông báo hệ thống"),
        (json!({"@type": "messageInvoice", "product_info": {"title": "Gói Pro"}, "title": "Cũ"}), "🧾 Hoá đơn «Gói Pro»"),
        (json!({"@type": "messageInvoice", "title": "Đơn hàng"}), "🧾 Hoá đơn «Đơn hàng»"),
        (json!({"@type": "messageInvoice"}), "🧾 Hoá đơn"),
        (json!({"@type": "messageExpiredPhoto"}), "🖼 Ảnh tự huỷ đã hết hạn"),
        (json!({"@type": "messageUnsupported"}), "⚠️ Tin nhắn chưa được hỗ trợ"),
        (json!({"@type": "messageGroupCall"}), "👥 Cuộc gọi nhóm"),
        (json!({"@type": "messageChatSetBackground"}), "🎨 Đã đổi hình nền hội thoại"),
        (json!({"@type": "messageChatBoost"}), "⚡ Đã tăng cường cho hội thoại"),
        (json!({"@type": "messagePaymentSuccessful"}), "💳 Thanh toán thành công"),
        (json!({"@type": "messageGiftedPremium"}), "⭐ Đã tặng Telegram Premium"),
        (json!({"@type": "messageGiveawayWinners"}), "🏆 Công bố người trúng quà"),
        (json!({"@type": "messageSuggestedPostApproved"}), "✅ Đã duyệt bài đề xuất"),
        (json!({"@type": "messageChecklistTasksDone"}), "☑️ Đã hoàn thành mục công việc"),
        (json!({"@type": "messageUsersShared"}), "👤 Đã chia sẻ người dùng"),
        (json!({"@type": "messageUserShared"}), "👤 Đã chia sẻ người dùng"),
    ] {
        let out = message(&json!({"content": content}), "");
        assert_eq!(out["kind"], "service", "{content}");
        assert_eq!(out["summary"], want, "{content}");
    }
}

#[test]
fn existing_macos_call_and_actor_service_labels_remain_intact() {
    let call_msg = json!({
        "id": 10,
        "chat_id": 42,
        "is_outgoing": false,
        "content": {
            "@type": "messageCall",
            "is_video": true,
            "duration": 18,
            "discard_reason": {"@type": "callDiscardReasonHungUp"}
        }
    });
    let formatted_call = message(&call_msg, "Đại ca");
    assert_eq!(formatted_call["kind"], "call");
    assert_eq!(formatted_call["summary"], "Cuộc gọi video đến (18 giây)");
    assert_eq!(formatted_call["call"]["is_video"], true);

    let resolver = |uid: i64| match uid {
        7 => Some("Bình Lê".to_string()),
        88 => Some("Chị Ngọc".to_string()),
        _ => None,
    };
    let ctx = MessageContext {
        sender: "",
        chat_title: "Nhóm thử",
        is_private: false,
        resolve_user: Some(&resolver),
    };

    let auto_del = message_with_ctx(
        &json!({
            "is_outgoing": true,
            "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {
                "@type": "messageChatSetMessageAutoDeleteTime",
                "message_auto_delete_time": 86400,
                "from_user_id": 7
            }
        }),
        &ctx,
    );
    assert_eq!(auto_del["kind"], "service");
    assert_eq!(
        auto_del["summary"],
        "Bạn đã đặt hẹn giờ tự hủy cho mọi cuộc trò chuyện sau 1 ngày"
    );

    let add_members = message_with_ctx(
        &json!({
            "is_outgoing": false,
            "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageChatAddMembers", "member_user_ids": [88]}
        }),
        &ctx,
    );
    assert_eq!(add_members["kind"], "service");
    assert_eq!(add_members["summary"], "Bình Lê đã thêm Chị Ngọc");
    assert!(!add_members["summary"].as_str().unwrap().contains("88"));

    let del_member = message_with_ctx(
        &json!({
            "is_outgoing": false,
            "sender_id": {"@type": "messageSenderUser", "user_id": 7},
            "content": {"@type": "messageChatDeleteMember", "user_id": 88}
        }),
        &ctx,
    );
    assert_eq!(del_member["summary"], "Bình Lê đã xóa Chị Ngọc khỏi nhóm");

    for (content, want) in [
        (json!({"@type": "messageContactRegistered"}), "Bình Lê đã tham gia Telegram"),
        (json!({"@type": "messageChatChangeTitle", "title": "Kênh Mới"}), "Bình Lê đã đổi tên nhóm thành «Kênh Mới»"),
        (json!({"@type": "messagePinMessage"}), "Bình Lê đã ghim một tin nhắn"),
        (json!({"@type": "messageVideoChatStarted"}), "Bình Lê đã bắt đầu cuộc trò chuyện video"),
        (json!({"@type": "messageScreenshotTaken"}), "Bình Lê đã chụp ảnh màn hình"),
    ] {
        let out = message_with_ctx(
            &json!({
                "is_outgoing": false,
                "sender_id": {"@type": "messageSenderUser", "user_id": 7},
                "content": content
            }),
            &ctx,
        );
        assert_eq!(out["kind"], "service");
        assert_eq!(out["summary"], want);
    }

    let sum_ctx = SummarizeContext {
        last_sender: "Chị Ngọc",
        resolve_user: Some(&resolver),
        ..SummarizeContext::default()
    };
    let summarized = summarize_with_ctx(
        &json!({
            "id": 9,
            "title": "Chị Ngọc",
            "type": {"@type": "chatTypePrivate", "user_id": 88},
            "last_message": {
                "id": 15,
                "is_outgoing": false,
                "sender_id": {"@type": "messageSenderUser", "user_id": 88},
                "content": {"@type": "messageContactRegistered"}
            }
        }),
        &sum_ctx,
    );
    assert_eq!(summarized["last"]["kind"], "service");
    assert_eq!(summarized["last"]["text"], "Chị Ngọc đã tham gia Telegram");
}

#[test]
fn cross_chat_reply_to_is_suppressed_while_same_chat_reply_is_kept() {
    let cross = message(
        &json!({
            "id": 10,
            "chat_id": -100100,
            "reply_to": {"@type": "messageReplyToMessage", "chat_id": -100999, "message_id": 5242880},
            "content": {"@type": "messageText", "text": {"text": "hi"}}
        }),
        "",
    );
    assert_eq!(cross["reply_to"], 0);

    let same = message(
        &json!({
            "id": 10,
            "chat_id": -100100,
            "reply_to": {"@type": "messageReplyToMessage", "chat_id": -100100, "message_id": 5242880},
            "content": {"@type": "messageText", "text": {"text": "hi"}}
        }),
        "",
    );
    assert_eq!(same["reply_to"], 5242880);

    let legacy = message(
        &json!({
            "id": 10,
            "chat_id": -100100,
            "reply_to": {"@type": "messageReplyToMessage", "message_id": 5242880},
            "content": {"@type": "messageText", "text": {"text": "hi"}}
        }),
        "",
    );
    assert_eq!(legacy["reply_to"], 5242880);
}
