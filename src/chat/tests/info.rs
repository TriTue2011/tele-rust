use super::*;

#[tokio::test]
async fn chat_info_returns_private_group_and_channel_details_and_hides_invite_link_for_non_admins() {
    // 1. Cá nhân: che số điện thoại, lấy tiểu sử/trạng thái/chặn/danh bạ, giữ nguyên chuỗi XSS.
    let private_chat = json!({
        "id": 42,
        "title": "<script>alert(1)</script> An",
        "type": {"@type": "chatTypePrivate", "user_id": 77},
        "positions": [{"list": {"@type": "chatListMain"}, "is_pinned": true}],
        "is_marked_as_unread": true,
        "unread_count": 4,
        "message_auto_delete_time": 86400,
        "has_protected_content": false,
        "notification_settings": {
            "@type": "chatNotificationSettings",
            "use_default_mute_for": false,
            "mute_for": 3600,
            "use_default_show_preview": false,
            "show_preview": false
        }
    });
    let user = json!({
        "id": 77,
        "first_name": "An",
        "last_name": "Lê",
        "phone_number": "84901234567",
        "usernames": {"active_usernames": ["an_le"]},
        "status": {"@type": "userStatusOnline"},
        "type": {"@type": "userTypeRegular"},
        "is_contact": false,
        "profile_photo": {"small": {"id": 501}}
    });
    let user_full = json!({
        "@type": "userFullInfo",
        "bio": {"@type": "formattedText", "text": "<img src=x onerror=alert(1)>"},
        "block_list": {"@type": "blockListMain"}
    });

    // 2. Nhóm cơ bản với vai trò quản trị (có invite_link) và thành viên thường (ẩn invite_link).
    let basic_chat = json!({
        "id": -55,
        "title": "Nhóm gia đình",
        "type": {"@type": "chatTypeBasicGroup", "basic_group_id": 55},
        "positions": [{"list": {"@type": "chatListArchive"}, "is_pinned": false}],
        "notification_settings": {"@type": "chatNotificationSettings", "use_default_mute_for": true, "mute_for": 0, "use_default_show_preview": true, "show_preview": true},
        "permissions": {"can_send_basic_messages": true, "can_invite_users": false}
    });
    let bg_admin = json!({
        "id": 55,
        "member_count": 6,
        "status": {"@type": "chatMemberStatusAdministrator", "rights": {"can_change_info": true, "can_delete_messages": true, "can_invite_users": true}}
    });
    let bg_member = json!({
        "id": 55,
        "member_count": 6,
        "status": {"@type": "chatMemberStatusMember"}
    });
    let bg_full = json!({
        "description": "Nhóm thử",
        "invite_link": {"@type": "chatInviteLink", "invite_link": "https://t.me/+secret_group"}
    });

    // 3. Kênh supergroup ở vai trò member thường: không được trả invite_link.
    let channel_chat = json!({
        "id": -10099,
        "title": "Kênh tin tức",
        "type": {"@type": "chatTypeSupergroup", "supergroup_id": 99, "is_channel": true},
        "positions": [],
        "notification_settings": {"@type": "chatNotificationSettings", "use_default_mute_for": true, "mute_for": 0, "use_default_show_preview": true, "show_preview": true},
        "permissions": {"can_send_basic_messages": false}
    });
    let sg = json!({
        "id": 99,
        "usernames": {"active_usernames": ["kenh_tin"]},
        "member_count": 1200,
        "status": {"@type": "chatMemberStatusMember"}
    });
    let sg_full = json!({
        "description": "Kênh thông báo",
        "member_count": 1200,
        "can_get_members": false,
        "invite_link": {"@type": "chatInviteLink", "invite_link": "https://t.me/+should_hide"}
    });

    let (chat, _) = setup(vec![
        ok(json!({"@type":"getChat","chat_id":42}), private_chat),
        ok(json!({"@type":"getUser","user_id":77}), user),
        ok(json!({"@type":"getUserFullInfo","user_id":77}), user_full),
        ok(json!({"@type":"getChat","chat_id":-55}), basic_chat.clone()),
        ok(json!({"@type":"getBasicGroup","basic_group_id":55}), bg_admin),
        ok(json!({"@type":"getBasicGroupFullInfo","basic_group_id":55}), bg_full.clone()),
        ok(json!({"@type":"getChat","chat_id":-55}), basic_chat),
        ok(json!({"@type":"getBasicGroup","basic_group_id":55}), bg_member),
        ok(json!({"@type":"getBasicGroupFullInfo","basic_group_id":55}), bg_full),
        ok(json!({"@type":"getChat","chat_id":-10099}), channel_chat),
        ok(json!({"@type":"getSupergroup","supergroup_id":99}), sg),
        ok(json!({"@type":"getSupergroupFullInfo","supergroup_id":99}), sg_full),
    ]);

    let info_private = get(&chat, "/api/chats/42/info", "").await.unwrap();
    assert_eq!(info_private["kind"], "private");
    assert_eq!(info_private["title"], "<script>alert(1)</script> An");
    assert_eq!(info_private["bio"], "<img src=x onerror=alert(1)>");
    assert_eq!(info_private["phone_masked"], "+849•••••567");
    assert!(!info_private.to_string().contains("84901234567"));
    assert_eq!(info_private["is_blocked"], true);
    assert_eq!(info_private["is_contact"], false);
    assert_eq!(info_private["pinned"], true);
    assert_eq!(info_private["marked_unread"], true);
    assert_eq!(info_private["auto_delete_seconds"], 86400);
    assert_eq!(info_private["notification"], json!({
        "use_default_mute_for": false, "mute_for": 3600,
        "use_default_show_preview": false, "show_preview": false
    }));

    let info_bg_admin = get(&chat, "/api/chats/-55/info", "").await.unwrap();
    assert_eq!(info_bg_admin["kind"], "group");
    assert_eq!(info_bg_admin["archived"], true);
    assert_eq!(info_bg_admin["is_admin"], true);
    assert_eq!(info_bg_admin["my_role"], "admin");
    assert_eq!(info_bg_admin["invite_link"], "https://t.me/+secret_group");
    assert_eq!(info_bg_admin["my_permissions"]["can_delete_messages"], true);

    let info_bg_member = get(&chat, "/api/chats/-55/info", "").await.unwrap();
    assert_eq!(info_bg_member["is_admin"], false);
    assert_eq!(info_bg_member["my_role"], "member");
    assert_eq!(info_bg_member["invite_link"], Value::Null);
    assert_eq!(info_bg_member["my_permissions"]["can_send_basic_messages"], true);

    let info_channel = get(&chat, "/api/chats/-10099/info", "").await.unwrap();
    assert_eq!(info_channel["kind"], "channel");
    assert_eq!(info_channel["username"], "kenh_tin");
    assert_eq!(info_channel["member_count"], 1200);
    assert_eq!(info_channel["is_admin"], false);
    assert_eq!(info_channel["invite_link"], Value::Null);
    assert_eq!(info_channel["can_get_members"], false);
}

#[tokio::test]
async fn chat_info_and_actions_return_404_when_chat_does_not_exist() {
    let (chat, _) = setup(vec![
        err(json!({"@type":"getChat","chat_id":999}), 404, "Chat not found"),
        err(json!({"@type":"deleteChatHistory","chat_id":999,"remove_from_chat_list":false,"revoke":false}), 404, "Chat not found"),
        err(json!({"@type":"leaveChat","chat_id":999}), 404, "Chat not found"),
    ]);
    assert_eq!(get(&chat, "/api/chats/999/info", "").await.unwrap_err().status, 404);
    assert_eq!(post(&chat, "/api/chats/999/clear_history", "", json!({"confirm":true})).await.unwrap_err().status, 404);
    assert_eq!(post(&chat, "/api/chats/999/leave", "", json!({"confirm":true})).await.unwrap_err().status, 404);
}

#[tokio::test]
async fn chat_settings_sends_exact_tdlib_requests_for_notification_auto_delete_pin_archive_unread() {
    let base_chat = json!({
        "id": 5,
        "notification_settings": {
            "@type": "chatNotificationSettings",
            "use_default_mute_for": true,
            "mute_for": 0,
            "use_default_sound": true,
            "use_default_show_preview": true,
            "show_preview": true
        }
    });
    let (chat, _) = setup(vec![
        ok(json!({"@type":"getChat","chat_id":5}), base_chat.clone()),
        ok(json!({
            "@type": "setChatNotificationSettings",
            "chat_id": 5,
            "notification_settings": {
                "@type": "chatNotificationSettings",
                "use_default_mute_for": false,
                "mute_for": 3600,
                "use_default_sound": true,
                "use_default_show_preview": false,
                "show_preview": false
            }
        }), json!({"@type":"ok"})),
        ok(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":604800}), json!({"@type":"ok"})),
        ok(json!({"@type":"addChatToList","chat_id":5,"chat_list":{"@type":"chatListArchive"}}), json!({"@type":"ok"})),
        ok(json!({"@type":"toggleChatIsPinned","chat_id":5,"chat_list":{"@type":"chatListArchive"},"is_pinned":true}), json!({"@type":"ok"})),
        ok(json!({"@type":"toggleChatIsMarkedAsUnread","chat_id":5,"is_marked_as_unread":true}), json!({"@type":"ok"})),
        ok(json!({"@type":"getChat","chat_id":5}), base_chat),
        ok(json!({
            "@type": "setChatNotificationSettings",
            "chat_id": 5,
            "notification_settings": {
                "@type": "chatNotificationSettings",
                "use_default_mute_for": true,
                "mute_for": 0,
                "use_default_sound": true,
                "use_default_show_preview": true,
                "show_preview": true
            }
        }), json!({"@type":"ok"})),
    ]);

    let updated = post(&chat, "/api/chats/5/settings", "", json!({
        "notification": {
            "use_default_mute_for": false,
            "mute_for": 3600,
            "use_default_show_preview": false,
            "show_preview": false
        },
        "auto_delete": {"seconds": 604800, "confirm": true},
        "archive": true,
        "pin": true,
        "mark_unread": true
    })).await.unwrap();
    assert_eq!(updated["ok"], true);
    assert_eq!(updated["auto_delete_seconds"], 604800);
    assert_eq!(updated["archived"], true);
    assert_eq!(updated["pinned"], true);
    assert_eq!(updated["marked_unread"], true);

    let reset_notif = post(&chat, "/api/chats/5/settings", "", json!({
        "notification": {"use_default_mute_for": true}
    })).await.unwrap();
    assert_eq!(reset_notif["ok"], true);
}

#[tokio::test]
async fn chat_settings_clear_history_and_leave_reject_missing_confirm_or_invalid_args_without_calling_tdlib() {
    let (chat, _) = setup(vec![]);
    for (path, body) in [
        ("/api/chats/5/settings", json!({})),
        ("/api/chats/5/settings", json!({"extra": 1})),
        ("/api/chats/5/settings", json!({"notification": {}})),
        ("/api/chats/5/settings", json!({"notification": {"mute_for": -1}})),
        ("/api/chats/5/settings", json!({"notification": {"mute_for": i64::from(i32::MAX) + 1}})),
        ("/api/chats/5/settings", json!({"notification": {"show_preview": "yes"}})),
        ("/api/chats/5/settings", json!({"auto_delete": {"seconds": 86400}})),
        ("/api/chats/5/settings", json!({"auto_delete": {"seconds": 86400, "confirm": false}})),
        ("/api/chats/5/settings", json!({"auto_delete": {"seconds": 86400, "confirm": "true"}})),
        // Nếu auto_delete thiếu confirm, cả batch (gồm pin=true) phải bị từ chối trước khi gọi TDLib.
        ("/api/chats/5/settings", json!({"pin": true, "auto_delete": {"seconds": 86400}})),
        ("/api/chats/5/settings", json!({"auto_delete": {"seconds": -5, "confirm": true}})),
        ("/api/chats/5/settings", json!({"auto_delete": {"seconds": 40_000_000, "confirm": true}})),
        ("/api/chats/5/settings", json!({"pin": "true"})),
        ("/api/chats/5/settings", json!({"archive": 1})),
        ("/api/chats/5/settings", json!({"mark_unread": "no"})),
        ("/api/chats/5/clear_history", json!({})),
        ("/api/chats/5/clear_history", json!({"confirm": false})),
        ("/api/chats/5/clear_history", json!({"confirm": "true"})),
        ("/api/chats/5/clear_history", json!({"confirm": true, "revoke": "yes"})),
        ("/api/chats/5/clear_history", json!({"confirm": true, "unknown": 1})),
        ("/api/chats/5/leave", json!({})),
        ("/api/chats/5/leave", json!({"confirm": false})),
        ("/api/chats/5/leave", json!({"confirm": "true"})),
        ("/api/chats/5/leave", json!({"confirm": true, "extra": true})),
    ] {
        let err = post(&chat, path, "", body.clone()).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_REQUEST"), "{path}: {body}");
    }
    assert_eq!(get(&chat, "/api/chats/-1001/members", "limit=abc").await.unwrap_err().status, 400);
    assert_eq!(get(&chat, "/api/chats/-1001/members", &format!("q={}", "a".repeat(129))).await.unwrap_err().status, 400);
}

#[tokio::test]
async fn chat_clear_history_and_leave_call_tdlib_when_confirmed() {
    let (chat, _) = setup(vec![
        ok(json!({"@type":"deleteChatHistory","chat_id":5,"remove_from_chat_list":false,"revoke":true}), json!({"@type":"ok"})),
        ok(json!({"@type":"leaveChat","chat_id":-1001}), json!({"@type":"ok"})),
    ]);
    assert_eq!(
        post(&chat, "/api/chats/5/clear_history", "", json!({"confirm":true,"revoke":true})).await.unwrap(),
        json!({"ok":true,"chat_id":5,"deleted":true})
    );
    assert_eq!(
        post(&chat, "/api/chats/-1001/leave", "", json!({"confirm":true})).await.unwrap(),
        json!({"ok":true,"chat_id":-1001,"left":true})
    );
}

#[tokio::test]
async fn chat_members_supports_supergroup_search_basic_group_filter_limit_cap_and_role_badges() {
    let sg_chat = json!({"id":-1001,"type":{"@type":"chatTypeSupergroup","supergroup_id":1,"is_channel":false}});
    let bg_chat = json!({"id":-55,"type":{"@type":"chatTypeBasicGroup","basic_group_id":55}});
    let pm_chat = json!({"id":7,"type":{"@type":"chatTypePrivate","user_id":7}});
    let u1 = json!({"id":1,"first_name":"Minh","last_name":"Chủ","phone_number":"84901111111","usernames":{"active_usernames":["chu_minh"]},"status":{"@type":"userStatusOnline"}});
    let u2 = json!({"id":2,"first_name":"<script>An</script>","phone_number":"84902222222","usernames":{"active_usernames":["an_mod"]},"status":{"@type":"userStatusRecently"}});
    let u3 = json!({"id":3,"first_name":"Bình","phone_number":"84903333333","usernames":{"active_usernames":["binh_mem"]}});

    let (chat, _) = setup(vec![
        // 1. Supergroup không có q, ép limit=999 -> 50, sắp xếp creator -> admin -> member.
        ok(json!({"@type":"getChat","chat_id":-1001}), sg_chat.clone()),
        ok(json!({"@type":"getSupergroupMembers","supergroup_id":1,"filter":{"@type":"supergroupMembersFilterRecent"},"offset":0,"limit":50}), json!({
            "total_count": 3,
            "members": [
                {"member_id":{"@type":"messageSenderUser","user_id":3},"status":{"@type":"chatMemberStatusMember"}},
                {"member_id":{"@type":"messageSenderUser","user_id":2},"status":{"@type":"chatMemberStatusAdministrator","custom_title":"Quản trị"}},
                {"member_id":{"@type":"messageSenderUser","user_id":1},"status":{"@type":"chatMemberStatusCreator","custom_title":"Founder"}}
            ]
        })),
        ok(json!({"@type":"getUser","user_id":3}), u3),
        ok(json!({"@type":"getUser","user_id":2}), u2),
        ok(json!({"@type":"getUser","user_id":1}), u1),
        // 2. Supergroup tìm theo q -> dùng supergroupMembersFilterSearch và tái dùng cache user đã nhớ.
        ok(json!({"@type":"getChat","chat_id":-1001}), sg_chat),
        ok(json!({"@type":"getSupergroupMembers","supergroup_id":1,"filter":{"@type":"supergroupMembersFilterSearch","query":"an"},"offset":0,"limit":10}), json!({
            "total_count": 1,
            "members": [
                {"member_id":{"@type":"messageSenderUser","user_id":2},"status":{"@type":"chatMemberStatusAdministrator","custom_title":"Quản trị"}}
            ]
        })),
        // 3. BasicGroup lọc theo q.
        ok(json!({"@type":"getChat","chat_id":-55}), bg_chat),
        ok(json!({"@type":"getBasicGroupFullInfo","basic_group_id":55}), json!({
            "members": [
                {"user_id":1,"status":{"@type":"chatMemberStatusCreator"}},
                {"user_id":3,"status":{"@type":"chatMemberStatusMember"}}
            ]
        })),
        // 4. Private chat -> danh sách rỗng.
        ok(json!({"@type":"getChat","chat_id":7}), pm_chat),
    ]);

    let all = get(&chat, "/api/chats/-1001/members", "limit=999").await.unwrap();
    assert_eq!(all["total_count"], 3);
    let list = all["members"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0]["role"], "creator");
    assert_eq!(list[1]["role"], "admin");
    assert_eq!(list[1]["name"], "<script>An</script>");
    assert_eq!(list[2]["role"], "member");
    assert!(!all.to_string().contains("84901111111"));
    assert!(!all.to_string().contains("84902222222"));

    let searched = get(&chat, "/api/chats/-1001/members", "q=+an+&limit=10").await.unwrap();
    assert_eq!(searched["members"].as_array().unwrap().len(), 1);
    assert_eq!(searched["members"][0]["user_id"], 2);

    let bg = get(&chat, "/api/chats/-55/members", "q=minh").await.unwrap();
    assert_eq!(bg["members"].as_array().unwrap().len(), 1);
    assert_eq!(bg["members"][0]["user_id"], 1);
    assert_eq!(bg["members"][0]["role"], "creator");

    let pm = get(&chat, "/api/chats/7/members", "").await.unwrap();
    assert_eq!(pm, json!({"chat_id":7,"total_count":0,"members":[]}));
}

#[tokio::test]
async fn auto_delete_update_emits_feed_and_updates_chats_and_info() {
    let list = json!({"@type":"chatListMain"});
    let chat_42 = json!({
        "id": 42,
        "title": "Đại ca",
        "type": {"@type":"chatTypePrivate","user_id":77},
        "positions": [],
        "message_auto_delete_time": 0,
        "notification_settings": {"@type":"chatNotificationSettings","use_default_mute_for":true,"mute_for":0,"use_default_show_preview":true,"show_preview":true}
    });
    let user_77 = json!({"id":77,"first_name":"Đại","last_name":"ca","phone_number":"84900000001"});
    let full_77 = json!({"@type":"userFullInfo","bio":{"@type":"formattedText","text":""}});
    let (chat, td) = setup(vec![
        ok(json!({"@type":"loadChats","chat_list":list,"limit":60}), json!({"@type":"ok"})),
        ok(json!({"@type":"getChats","chat_list":list,"limit":60}), json!({"chat_ids":[42]})),
        ok(json!({"@type":"getChat","chat_id":42}), chat_42.clone()),
        ok(json!({"@type":"loadChats","chat_list":list,"limit":60}), json!({"@type":"ok"})),
        ok(json!({"@type":"getChats","chat_list":list,"limit":60}), json!({"chat_ids":[42]})),
        ok(json!({"@type":"getChat","chat_id":42}), chat_42.clone()),
        ok(json!({"@type":"getChat","chat_id":42}), chat_42.clone()),
        ok(json!({"@type":"getUser","user_id":77}), user_77.clone()),
        ok(json!({"@type":"getUserFullInfo","user_id":77}), full_77.clone()),
        ok(json!({"@type":"loadChats","chat_list":list,"limit":60}), json!({"@type":"ok"})),
        ok(json!({"@type":"getChats","chat_list":list,"limit":60}), json!({"chat_ids":[42]})),
        ok(json!({"@type":"getChat","chat_id":42}), chat_42.clone()),
        ok(json!({"@type":"getChat","chat_id":42}), chat_42),
        ok(json!({"@type":"getUser","user_id":77}), user_77),
        ok(json!({"@type":"getUserFullInfo","user_id":77}), full_77),
    ]);

    let initial = get(&chat, "/api/chats", "").await.unwrap();
    assert_eq!(initial["chats"][0]["auto_delete_seconds"], 0);

    let updater = async {
        tokio::task::yield_now().await;
        td.updates.send(json!({
            "@type": "updateChatMessageAutoDeleteTime",
            "chat_id": 42,
            "message_auto_delete_time": 86400
        })).unwrap();
    };
    let (feed_res, ()) = tokio::join!(get(&chat, "/api/feed", "since=0&wait=1"), updater);
    let feed = feed_res.unwrap();
    assert_eq!(feed["items"][0]["type"], "chat_auto_delete");
    assert_eq!(feed["items"][0]["chat_id"], 42);
    assert_eq!(feed["items"][0]["seconds"], 86400);

    let after = get(&chat, "/api/chats", "").await.unwrap();
    assert_eq!(after["chats"][0]["auto_delete_seconds"], 86400);
    let info = get(&chat, "/api/chats/42/info", "").await.unwrap();
    assert_eq!(info["auto_delete_seconds"], 86400);

    let turn_off = async {
        tokio::task::yield_now().await;
        td.updates.send(json!({
            "@type": "updateChatMessageAutoDeleteTime",
            "chat_id": 42,
            "message_auto_delete_time": 0
        })).unwrap();
    };
    let (feed_off, ()) = tokio::join!(get(&chat, "/api/feed", "since=1&wait=1"), turn_off);
    let feed_off = feed_off.unwrap();
    assert_eq!(feed_off["items"][0]["type"], "chat_auto_delete");
    assert_eq!(feed_off["items"][0]["seconds"], 0);

    assert_eq!(get(&chat, "/api/chats", "").await.unwrap()["chats"][0]["auto_delete_seconds"], 0);
    assert_eq!(get(&chat, "/api/chats/42/info", "").await.unwrap()["auto_delete_seconds"], 0);
}

#[tokio::test]
async fn auto_delete_validates_telegram_allowed_values_and_requires_confirm() {
    let (chat, _) = setup(vec![
        ok(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":86400}), json!({"@type":"ok"})),
        ok(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":604800}), json!({"@type":"ok"})),
        ok(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":2678400}), json!({"@type":"ok"})),
        ok(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":172800}), json!({"@type":"ok"})),
        ok(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":31_536_000}), json!({"@type":"ok"})),
        ok(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":0}), json!({"@type":"ok"})),
    ]);

    for bad in [
        json!({"auto_delete": {"seconds": 86400}}),
        json!({"auto_delete": {"seconds": 86400, "confirm": false}}),
        json!({"auto_delete": {"seconds": 86400, "confirm": "true"}}),
        json!({"auto_delete": {"seconds": 1, "confirm": true}}),
        json!({"auto_delete": {"seconds": 60, "confirm": true}}),
        json!({"auto_delete": {"seconds": 3600, "confirm": true}}),
        json!({"auto_delete": {"seconds": 90000, "confirm": true}}),
        json!({"auto_delete": {"seconds": -86400, "confirm": true}}),
        json!({"auto_delete": {"seconds": 31_622_400, "confirm": true}}),
        json!({"auto_delete": {"seconds": "86400", "confirm": true}}),
    ] {
        let err = post(&chat, "/api/chats/5/settings", "", bad.clone()).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_REQUEST"), "{bad}");
    }

    for seconds in [86400_i64, 604800, 2678400, 172800, 31_536_000, 0] {
        let res = post(
            &chat,
            "/api/chats/5/settings",
            "",
            json!({"auto_delete": {"seconds": seconds, "confirm": true}}),
        ).await.unwrap();
        assert_eq!(res["ok"], true);
        assert_eq!(res["auto_delete_seconds"], seconds);
    }
}

#[tokio::test]
async fn chat_delete_requires_confirm_and_calls_correct_tdlib_by_chat_kind() {
    let private_chat = json!({"id": 5, "type": {"@type": "chatTypePrivate", "user_id": 7}});
    let group_chat = json!({"id": -1001, "type": {"@type": "chatTypeSupergroup", "supergroup_id": 1001, "is_channel": false}});
    let (chat, _) = setup(vec![
        err(json!({"@type":"getChat","chat_id":999}), 404, "Chat not found"),
        ok(json!({"@type":"getChat","chat_id":5}), private_chat),
        ok(json!({"@type":"deleteChatHistory","chat_id":5,"remove_from_chat_list":true,"revoke":true}), json!({"@type":"ok"})),
        ok(json!({"@type":"getChat","chat_id":-1001}), group_chat),
        ok(json!({"@type":"leaveChat","chat_id":-1001}), json!({"@type":"ok"})),
    ]);

    for bad in [
        json!({}),
        json!({"confirm": false}),
        json!({"confirm": "true"}),
        json!({"confirm": true, "extra": 1}),
        json!({"confirm": true, "revoke": "yes"}),
    ] {
        let err = post(&chat, "/api/chats/5/delete", "", bad.clone()).await.unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (400, "BAD_REQUEST"), "{bad}");
    }

    assert_eq!(post(&chat, "/api/chats/999/delete", "", json!({"confirm":true})).await.unwrap_err().status, 404);
    assert_eq!(
        post(&chat, "/api/chats/5/delete", "", json!({"confirm":true,"revoke":true})).await.unwrap(),
        json!({"ok":true,"chat_id":5,"deleted":true})
    );
    assert_eq!(
        post(&chat, "/api/chats/-1001/delete", "", json!({"confirm":true})).await.unwrap(),
        json!({"ok":true,"chat_id":-1001,"deleted":true})
    );
}
