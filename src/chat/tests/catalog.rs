use super::*;

struct Case(&'static str, Value, Vec<FakeReply>, Value);

fn mutate(name: &'static str, body: Value, request: Value, response: Value) -> Case {
    Case(name, body, vec![ok(request,json!({"@type":"ok"}))], response)
}

fn read(name: &'static str, body: Value, request: Value, reply: Value, response: Value) -> Case {
    Case(name, body, vec![ok(request,reply)], response)
}

#[tokio::test]
async fn chat_and_message_catalog_matches_python_requests_and_responses() {
    let chat = json!({"id":5,"title":"Nhóm","type":{"@type":"chatTypeBasicGroup"},"unread_count":2,
        "notification_settings":{"@type":"chatNotificationSettings","mute_for":25,"use_default_sound":true}});
    let get_chat = json!({"@type":"getChat","chat_id":5});
    let ft = |s: &str| json!({"@type":"formattedText","text":s,"entities":[]});
    let poll = |kind: Value, anon, multiple| json!({"@type":"sendMessage","chat_id":5,"input_message_content":{"@type":"inputMessagePoll",
        "question":ft("Q"),"options":[{"@type":"inputPollOption","text":ft("A")},{"@type":"inputPollOption","text":ft("B")}],
        "is_anonymous":anon,"allows_multiple_answers":multiple,"type":kind}});
    let cases = [
        Case("set_mute",json!({"chat_id":5,"mute_for":-1}),vec![ok(get_chat.clone(),chat.clone()),ok(json!({"@type":"setChatNotificationSettings",
            "chat_id":5,"notification_settings":{"@type":"chatNotificationSettings","mute_for":0,"use_default_mute_for":true,"use_default_sound":true}}),json!({}))],
            json!({"chat_id":5,"mute_for":-1})),
        mutate("pin_chat",json!({"chat_id":"5","archived":true}),json!({"@type":"toggleChatIsPinned","chat_id":5,"chat_list":{"@type":"chatListArchive"},"is_pinned":true}),json!({"chat_id":5,"pinned":true})),
        mutate("archive_chat",json!({"chat_id":5,"archive":false}),json!({"@type":"addChatToList","chat_id":5,"chat_list":{"@type":"chatListMain"}}),json!({"chat_id":5,"archived":false})),
        mutate("mark_chat_unread",json!({"chat_id":5}),json!({"@type":"toggleChatIsMarkedAsUnread","chat_id":5,"is_marked_as_unread":true}),json!({"chat_id":5,"unread":true})),
        mutate("set_auto_delete",json!({"chat_id":5,"seconds":86400}),json!({"@type":"setChatMessageAutoDeleteTime","chat_id":5,"message_auto_delete_time":86400}),json!({"chat_id":5,"seconds":86400})),
        mutate("send_typing",json!({"chat_id":5,"action":"recording_voice"}),json!({"@type":"sendChatAction","chat_id":5,"business_connection_id":"","action":{"@type":"chatActionRecordingVoiceNote"}}),json!({"chat_id":5,"action":"recording_voice"})),
        mutate("delete_chat_history",json!({"chat_id":5,"confirm":true,"revoke":true}),json!({"@type":"deleteChatHistory","chat_id":5,"remove_from_chat_list":false,"revoke":true}),json!({"chat_id":5,"deleted":true})),
        mutate("rename_chat",json!({"chat_id":-5,"title":" Tên mới "}),json!({"@type":"setChatTitle","chat_id":-5,"title":"Tên mới"}),json!({"chat_id":-5,"title":"Tên mới"})),
        mutate("leave_chat",json!({"chat_id":-5}),json!({"@type":"leaveChat","chat_id":-5}),json!({"chat_id":-5,"left":true})),
        read("get_chat_info",json!({"chat_id":5}),get_chat,chat,json!({"chat_id":5,"title":"Nhóm","kind":"basicgroup","unread":2,"muted_for":25,"auto_delete_seconds":0,"has_protected_content":false})),
        mutate("pin_message",json!({"chat_id":5,"message_id":9,"notify":true}),json!({"@type":"pinChatMessage","chat_id":5,"message_id":9,"disable_notification":false,"only_for_self":false}),json!({"chat_id":5,"message_id":9,"pinned":true})),
        mutate("unpin_message",json!({"chat_id":5,"message_id":9}),json!({"@type":"unpinChatMessage","chat_id":5,"message_id":9}),json!({"chat_id":5,"message_id":9,"pinned":false})),
        read("send_poll",json!({"chat_id":5,"question":"Q","options":" A\n B ","anonymous":false,"multiple":true}),
            poll(json!({"@type":"inputPollTypeRegular","allow_adding_options":false}),false,true),json!({"id":99}),json!({"chat_id":5,"message_id":99,"options":2})),
        read("send_poll",json!({"chat_id":5,"question":"Q","options":["A","B"],"quiz_answer":2,"explanation":"why","multiple":true}),
            poll(json!({"@type":"inputPollTypeQuiz","correct_option_ids":[1],"explanation":ft("why")}),true,false),json!({"id":99}),json!({"chat_id":5,"message_id":99,"options":2})),
        mutate("vote_poll",json!({"chat_id":5,"message_id":9,"options":"1;3"}),json!({"@type":"setPollAnswer","chat_id":5,"message_id":9,"option_ids":[0,2]}),json!({"chat_id":5,"message_id":9,"voted":[1,3]})),
        mutate("stop_poll",json!({"chat_id":5,"message_id":9}),json!({"@type":"stopPoll","chat_id":5,"message_id":9}),json!({"chat_id":5,"message_id":9,"closed":true})),
    ];
    run(cases).await;
}

#[tokio::test]
async fn people_group_and_account_catalog_matches_python_requests_and_responses() {
    let user = json!({"id":7,"first_name":"Bình","last_name":"Lê","phone_number":"000999","usernames":{"active_usernames":["binh"]},
        "status":{"@type":"userStatusOnline"},"type":{"@type":"userTypeBot"}});
    let brief = json!({"user_id":7,"name":"Bình Lê","phone":"+000999","username":"binh","is_bot":true,"status":"online"});
    let get_user = json!({"@type":"getUser","user_id":7});
    let member = json!({"@type":"messageSenderUser","user_id":7});
    let cases = [
        read("find_user",json!({"phone":"+000 999"}),json!({"@type":"searchUserByPhoneNumber","phone_number":"000999","only_local":false}),user.clone(),brief.clone()),
        Case("find_user",json!({"username":"@binh"}),vec![ok(json!({"@type":"searchPublicChat","username":"binh"}),
            json!({"id":7,"type":{"@type":"chatTypePrivate","user_id":7}})),ok(get_user.clone(),user.clone())],brief.clone()),
        read("find_user",json!({"username":"kenh"}),json!({"@type":"searchPublicChat","username":"kenh"}),
            json!({"id":-7,"title":"Kênh","type":{"@type":"chatTypeSupergroup"}}),json!({"user_id":null,"name":"Kênh","username":"kenh","kind":"supergroup","chat_id":-7})),
        read("get_user_info",json!({"user_id":7}),get_user,user.clone(),brief.clone()),
        mutate("add_contact",json!({"user_id":7,"first_name":" Bình ","share_phone":true}),json!({"@type":"addContact","user_id":7,"share_phone_number":true,
            "contact":{"@type":"importedContact","first_name":"Bình","last_name":"","phone_number":"","note":{"@type":"formattedText","text":"","entities":[]}}}),json!({"user_id":7,"added":true})),
        mutate("remove_contact",json!({"user_id":7}),json!({"@type":"removeContacts","user_ids":[7]}),json!({"user_id":7,"removed":true})),
        mutate("block_user",json!({"user_id":7}),json!({"@type":"setMessageSenderBlockList","sender_id":member,"block_list":{"@type":"blockListMain"}}),json!({"user_id":7,"blocked":true})),
        mutate("unblock_user",json!({"user_id":7}),json!({"@type":"setMessageSenderBlockList","sender_id":member}),json!({"user_id":7,"blocked":false})),
        read("create_group",json!({"title":"Nhà","user_ids":"1, 2;3"}),json!({"@type":"createNewBasicGroupChat","user_ids":[1,2,3],"title":"Nhà","message_auto_delete_time":0}),
            json!({"chat_id":-10,"failed_to_add_members":{"failed_to_add_members":[{"user_id":3}]}}),json!({"chat_id":-10,"title":"Nhà","failed_user_ids":[3]})),
        read("add_chat_member",json!({"chat_id":-5,"user_id":7,"forward_limit":20}),json!({"@type":"addChatMember","chat_id":-5,"user_id":7,"forward_limit":20}),
            json!({"failed_to_add_members":[{"user_id":7}]}),json!({"chat_id":-5,"user_id":7,"added":false})),
        mutate("remove_chat_member",json!({"chat_id":-5,"user_id":7}),json!({"@type":"setChatMemberStatus","chat_id":-5,"member_id":member,
            "status":{"@type":"chatMemberStatusBanned","banned_until_date":"now+60"}}),json!({"chat_id":-5,"user_id":7,"removed":true})),
        read("create_invite_link",json!({"chat_id":-5,"expire_seconds":3600,"join_request":true}),json!({"@type":"createChatInviteLink","chat_id":-5,"name":"",
            "expiration_date":"now+3600","member_limit":0,"creates_join_request":true}),json!({"invite_link":"https://t.me/+demo"}),json!({"chat_id":-5,"invite_link":"https://t.me/+demo"})),
        read("join_chat",json!({"invite_link":"https://t.me/+demo"}),json!({"@type":"joinChatByInviteLink","invite_link":"https://t.me/+demo"}),json!({"chat_id":-5}),json!({"joined":true,"chat_id":-5})),
        read("get_me",json!({}),json!({"@type":"getMe"}),user,brief),
        mutate("set_name",json!({"first_name":" An "}),json!({"@type":"setName","first_name":"An","last_name":""}),json!({"first_name":"An","last_name":""})),
        mutate("set_bio",json!({"bio":" xin chào "}),json!({"@type":"setBio","bio":"xin chào"}),json!({"bio":"xin chào"})),
        mutate("set_username",json!({"username":"@an_nhi"}),json!({"@type":"setUsername","username":"an_nhi"}),json!({"username":"an_nhi"})),
    ];
    run(cases).await;
}

async fn run(cases: impl IntoIterator<Item = Case>) {
    for Case(name, body, requests, response) in cases {
        let (chat, _) = setup(requests);
        assert_eq!(post(&chat,&format!("/api/do/{name}"),"",body).await.unwrap(),response,"{name}");
    }
}

#[tokio::test]
async fn only_explicit_services_and_confirmed_history_deletion_reach_tdlib() {
    let (chat, _) = setup(vec![]);
    for name in ["unknown", "deleteAccount", "getOption", "sendMessage"] {
        let e = post(&chat,&format!("/api/do/{name}"),"",json!({"@type":name})).await.unwrap_err();
        assert_eq!((e.status,e.code.as_str()),(404,"NOT_FOUND"));
    }
    for (name, b) in [
        ("delete_chat_history",json!({"chat_id":5,"revoke":true})),("delete_chat_history",json!({"chat_id":5,"confirm":"true"})),
        ("set_mute",json!({"chat_id":5,"mute_for":1_000_000_000})),("pin_chat",json!({"chat_id":"abc"})),
        ("send_typing",json!({"chat_id":5,"action":"bad"})),("rename_chat",json!({"chat_id":5,"title":" "})),
        ("send_poll",json!({"chat_id":5,"question":"Q","options":["a"]})),("send_poll",json!({"chat_id":5,"question":"Q","options":["a","b"],"quiz_answer":3})),
        ("vote_poll",json!({"chat_id":5,"message_id":9,"options":[0]})),("find_user",json!({})),("find_user",json!({"phone":"0","username":"u"})),
        ("add_contact",json!({"user_id":7})),("create_group",json!({"title":"x","user_ids":[]})),
        ("join_chat",json!({"invite_link":"https://example.invalid/x"})),("set_bio",json!({"bio":"x".repeat(71)})),
    ] {
        assert_eq!(post(&chat,&format!("/api/do/{name}"),"",b).await.unwrap_err().status,400,"{name}");
    }
}

#[tokio::test]
async fn join_chat_reads_id_field_from_tdlib_chat_response() {
    let (chat, _) = setup(vec![ok(
        json!({"@type":"joinChatByInviteLink","invite_link":"https://t.me/+real"}),
        json!({"@type":"chat","id":-100999,"title":"Nhóm"}),
    )]);
    assert_eq!(
        post(&chat, "/api/do/join_chat", "", json!({"invite_link":"https://t.me/+real"})).await.unwrap(),
        json!({"joined":true,"chat_id":-100999})
    );
}
