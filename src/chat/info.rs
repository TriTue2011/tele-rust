//! Thông tin chi tiết, cài đặt riêng và danh sách thành viên của từng hội thoại.
use serde_json::{json, Map, Value};

use super::{array, field, mask_phone, text, truth, user_name, ChatService, GwError, Reply};

const DAY: i64 = 86_400;
const MAX_AUTO_DELETE_SECONDS: i64 = 365 * DAY;
const MAX_MEMBERS: i64 = 50;

fn chat_list(archived: bool) -> Value {
    json!({"@type": if archived { "chatListArchive" } else { "chatListMain" }})
}

fn chat_kind(chat: &Value) -> &'static str {
    let ct = &chat["type"];
    match text(&ct["@type"]) {
        "chatTypePrivate" => "private",
        "chatTypeSecret" => "secret",
        "chatTypeSupergroup" if truth(&ct["is_channel"]) => "channel",
        "chatTypeSupergroup" | "chatTypeBasicGroup" => "group",
        _ => "other",
    }
}

fn member_role(status: &Value) -> (&'static str, bool) {
    match text(&status["@type"]) {
        "chatMemberStatusCreator" => ("creator", true),
        "chatMemberStatusAdministrator" => ("admin", true),
        "chatMemberStatusRestricted" => ("restricted", false),
        "chatMemberStatusLeft" => ("left", false),
        "chatMemberStatusBanned" => ("banned", false),
        _ => ("member", false),
    }
}

fn role_rank(role: &str) -> u8 {
    match role {
        "creator" => 0,
        "admin" => 1,
        "member" => 2,
        "restricted" => 3,
        "left" => 4,
        _ => 5,
    }
}

fn collect_bool_flags(target: &mut Map<String, Value>, source: &Value) {
    if let Some(map) = source.as_object() {
        for (key, val) in map {
            if let Some(flag) = val.as_bool() {
                target.insert(key.clone(), Value::Bool(flag));
            }
        }
    }
}

fn member_role_and_perms(status: &Value, chat_perms: &Value) -> (&'static str, bool, Value) {
    let (role, is_admin) = member_role(status);
    let mut perms = Map::new();
    match role {
        "left" | "banned" => return (role, false, Value::Object(perms)),
        "creator" => {
            collect_bool_flags(&mut perms, chat_perms);
            collect_bool_flags(&mut perms, &status["rights"]);
            for right in [
                "can_change_info",
                "can_invite_users",
                "can_pin_messages",
                "can_delete_messages",
                "can_ban_users",
                "can_restrict_members",
                "can_promote_members",
                "can_manage_video_chats",
            ] {
                perms.entry(right).or_insert(Value::Bool(true));
            }
        }
        "admin" => {
            collect_bool_flags(&mut perms, chat_perms);
            collect_bool_flags(&mut perms, &status["rights"]);
        }
        "restricted" => {
            collect_bool_flags(&mut perms, chat_perms);
            collect_bool_flags(&mut perms, &status["permissions"]);
        }
        _ => {
            collect_bool_flags(&mut perms, chat_perms);
        }
    }
    (role, is_admin, Value::Object(perms))
}

fn extract_invite_link(full: &Value) -> Option<String> {
    full["invite_link"]["invite_link"]
        .as_str()
        .or_else(|| full["invite_link"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn parse_bool(value: &Value, name: &str) -> Result<bool, GwError> {
    value
        .as_bool()
        .ok_or_else(|| GwError::bad_request(format!("{name} phải là boolean.")))
}

fn parse_int_range(value: &Value, name: &str, low: i64, high: i64) -> Result<i64, GwError> {
    let n = value
        .as_i64()
        .ok_or_else(|| GwError::bad_request(format!("{name} phải là số nguyên.")))?;
    if n < low || n > high {
        return Err(GwError::bad_request(format!("{name} phải trong khoảng {low}..{high}.")));
    }
    Ok(n)
}

fn parse_auto_delete_seconds(value: &Value) -> Result<i64, GwError> {
    let sec = value
        .as_i64()
        .ok_or_else(|| GwError::bad_request("seconds phải là số nguyên."))?;
    if sec == 0 || ((DAY..=MAX_AUTO_DELETE_SECONDS).contains(&sec) && sec % DAY == 0) {
        Ok(sec)
    } else {
        Err(GwError::bad_request(
            "Telegram chỉ hỗ trợ tắt (0) hoặc bội số tròn ngày từ 1 đến 365 ngày (86400..31536000 giây).",
        ))
    }
}

struct NotificationUpdate {
    use_default_mute_for: Option<bool>,
    mute_for: Option<i64>,
    use_default_show_preview: Option<bool>,
    show_preview: Option<bool>,
}

struct PinUpdate {
    pinned: bool,
    archived: Option<bool>,
}

struct SettingsPlan {
    notification: Option<NotificationUpdate>,
    auto_delete_seconds: Option<i64>,
    archive: Option<bool>,
    pin: Option<PinUpdate>,
    mark_unread: Option<bool>,
}

fn parse_settings_plan(body: &Value) -> Result<SettingsPlan, GwError> {
    let Some(map) = body.as_object() else {
        return Err(GwError::bad_request("Thân POST phải là JSON object."));
    };
    if map.is_empty() {
        return Err(GwError::bad_request("Không có cài đặt nào để cập nhật."));
    }
    let allowed = ["notification", "auto_delete", "pin", "archive", "mark_unread"];
    if let Some(bad) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(GwError::bad_request(format!("Trường cài đặt hội thoại không hợp lệ: {bad}")));
    }

    let notification = if let Some(val) = map.get("notification") {
        let Some(nmap) = val.as_object() else {
            return Err(GwError::bad_request("notification phải là JSON object."));
        };
        if nmap.is_empty() {
            return Err(GwError::bad_request("notification không được trống."));
        }
        let n_allowed = ["use_default_mute_for", "mute_for", "use_default_show_preview", "show_preview"];
        if let Some(bad) = nmap.keys().find(|k| !n_allowed.contains(&k.as_str())) {
            return Err(GwError::bad_request(format!("Trường notification không hợp lệ: {bad}")));
        }
        let use_default_mute_for = nmap.get("use_default_mute_for").map(|v| parse_bool(v, "use_default_mute_for")).transpose()?;
        let mute_for = nmap.get("mute_for").map(|v| parse_int_range(v, "mute_for", 0, i64::from(i32::MAX))).transpose()?;
        let use_default_show_preview = nmap.get("use_default_show_preview").map(|v| parse_bool(v, "use_default_show_preview")).transpose()?;
        let show_preview = nmap.get("show_preview").map(|v| parse_bool(v, "show_preview")).transpose()?;
        Some(NotificationUpdate { use_default_mute_for, mute_for, use_default_show_preview, show_preview })
    } else {
        None
    };

    let auto_delete_seconds = if let Some(val) = map.get("auto_delete") {
        let Some(amap) = val.as_object() else {
            return Err(GwError::bad_request("auto_delete phải là JSON object."));
        };
        let a_allowed = ["seconds", "confirm"];
        if let Some(bad) = amap.keys().find(|k| !a_allowed.contains(&k.as_str())) {
            return Err(GwError::bad_request(format!("Trường auto_delete không hợp lệ: {bad}")));
        }
        if amap.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(GwError::bad_request("Đổi thời gian tự xoá tin nhắn cần confirm=true."));
        }
        let raw_sec = amap.get("seconds").ok_or_else(|| GwError::bad_request("Thiếu seconds cho auto_delete."))?;
        Some(parse_auto_delete_seconds(raw_sec)?)
    } else {
        None
    };

    let archive = map.get("archive").map(|v| parse_bool(v, "archive")).transpose()?;

    let pin = if let Some(val) = map.get("pin") {
        if let Some(b) = val.as_bool() {
            Some(PinUpdate { pinned: b, archived: None })
        } else if let Some(pmap) = val.as_object() {
            let p_allowed = ["pinned", "archived"];
            if let Some(bad) = pmap.keys().find(|k| !p_allowed.contains(&k.as_str())) {
                return Err(GwError::bad_request(format!("Trường pin không hợp lệ: {bad}")));
            }
            let pinned = pmap.get("pinned").ok_or_else(|| GwError::bad_request("Thiếu pinned."))?;
            let pinned = parse_bool(pinned, "pinned")?;
            let archived = pmap.get("archived").map(|v| parse_bool(v, "archived")).transpose()?;
            Some(PinUpdate { pinned, archived })
        } else {
            return Err(GwError::bad_request("pin phải là boolean hoặc JSON object."));
        }
    } else {
        None
    };

    let mark_unread = map.get("mark_unread").map(|v| parse_bool(v, "mark_unread")).transpose()?;

    Ok(SettingsPlan { notification, auto_delete_seconds, archive, pin, mark_unread })
}

impl ChatService {
    pub(super) async fn chat_info(&self, chat_id: i64) -> Reply {
        let ch = self.request(json!({"@type": "getChat", "chat_id": chat_id})).await?;
        self.state.remember_chat(&ch);
        let ct = &ch["type"];
        let kind = chat_kind(&ch);
        let positions = array(&ch["positions"]);
        let archived = positions.iter().any(|p| p["list"]["@type"] == "chatListArchive");
        let list_type = if archived { "chatListArchive" } else { "chatListMain" };
        let pinned = positions.iter().any(|p| {
            truth(&p["is_pinned"]) && (p["list"].is_null() || p["list"]["@type"] == list_type)
        });
        let ns = &ch["notification_settings"];
        let notification = json!({
            "use_default_mute_for": truth(&ns["use_default_mute_for"]),
            "mute_for": ns["mute_for"].as_i64().unwrap_or(0),
            "use_default_show_preview": truth(&ns["use_default_show_preview"]),
            "show_preview": ns.get("show_preview").and_then(Value::as_bool).unwrap_or(true),
        });
        let auto_delete_seconds = self.state.sync_chat_auto_delete(
            ch["id"].as_i64().unwrap_or(chat_id),
            ch["message_auto_delete_time"].as_i64().unwrap_or(0),
        );
        let mut out = json!({
            "chat_id": field(&ch, "id", json!(chat_id)),
            "title": field(&ch, "title", json!("")),
            "kind": kind,
            "unread": field(&ch, "unread_count", json!(0)),
            "marked_unread": truth(&ch["is_marked_as_unread"]),
            "pinned": pinned,
            "archived": archived,
            "auto_delete_seconds": auto_delete_seconds,
            "last_read_outbox_message_id": ch["last_read_outbox_message_id"].as_i64().unwrap_or(0).max(0),
            "has_protected_content": truth(&ch["has_protected_content"]),
            "avatar_file": ch["photo"]["small"]["id"],
            "notification": notification,
        });

        match text(&ct["@type"]) {
            "chatTypePrivate" | "chatTypeSecret" => {
                let uid = ct["user_id"].as_i64().unwrap_or(0);
                let user = if uid != 0 {
                    match self.request(json!({"@type": "getUser", "user_id": uid})).await {
                        Ok(u) => {
                            self.state.remember_user(&u);
                            Some(u)
                        }
                        Err(_) => None,
                    }
                } else {
                    None
                };
                let full = if uid != 0 {
                    self.request(json!({"@type": "getUserFullInfo", "user_id": uid})).await.ok()
                } else {
                    None
                };
                let u = user.as_ref().unwrap_or(&Value::Null);
                let f = full.as_ref().unwrap_or(&Value::Null);
                let username = u["usernames"]["active_usernames"][0]
                    .as_str()
                    .or_else(|| u["usernames"]["editable_username"].as_str())
                    .unwrap_or_default();
                let bio = f["bio"]["text"].as_str().unwrap_or_default();
                let (online, status, was_online) = super::shape::user_status_info(&u["status"]);
                let is_blocked = ch["block_list"]["@type"] == "blockListMain"
                    || f["block_list"]["@type"] == "blockListMain";
                out["user_id"] = json!(uid);
                out["first_name"] = json!(text(&u["first_name"]));
                out["last_name"] = json!(text(&u["last_name"]));
                out["username"] = json!(username);
                out["bio"] = json!(bio);
                out["phone_masked"] = json!(mask_phone(text(&u["phone_number"])));
                out["status"] = json!(status);
                out["online"] = json!(online);
                out["was_online"] = json!(was_online);
                out["verified"] = json!(super::shape::is_verified(u));
                out["is_contact"] = json!(truth(&u["is_contact"]));
                out["is_blocked"] = json!(is_blocked);
                out["is_bot"] = json!(u["type"]["@type"] == "userTypeBot");
                if out["title"].as_str().map_or(true, str::is_empty) && !u.is_null() {
                    out["title"] = json!(user_name(u));
                }
                if out["avatar_file"].is_null() && !u["profile_photo"]["small"]["id"].is_null() {
                    out["avatar_file"] = u["profile_photo"]["small"]["id"].clone();
                }
            }
            "chatTypeBasicGroup" => {
                let bg_id = ct["basic_group_id"].as_i64().unwrap_or(0);
                let bg = if bg_id != 0 {
                    self.request(json!({"@type": "getBasicGroup", "basic_group_id": bg_id})).await.ok()
                } else {
                    None
                };
                let full = if bg_id != 0 {
                    self.request(json!({"@type": "getBasicGroupFullInfo", "basic_group_id": bg_id})).await.ok()
                } else {
                    None
                };
                let bg_val = bg.as_ref().unwrap_or(&Value::Null);
                if !bg_val.is_null() {
                    self.state.remember_basic_group(bg_val);
                }
                let full_val = full.as_ref().unwrap_or(&Value::Null);
                let (my_role, is_admin, my_permissions) = member_role_and_perms(&bg_val["status"], &ch["permissions"]);
                let member_count = bg_val["member_count"]
                    .as_i64()
                    .filter(|&n| n > 0)
                    .or_else(|| full_val["members"].as_array().map(|m| m.len() as i64))
                    .unwrap_or(0);
                let invite_link = if is_admin { extract_invite_link(full_val) } else { None };
                out["member_count"] = json!(member_count);
                out["bio"] = json!(text(&full_val["description"]));
                out["my_role"] = json!(my_role);
                out["is_admin"] = json!(is_admin);
                out["my_permissions"] = my_permissions;
                out["invite_link"] = invite_link.map_or(Value::Null, Value::String);
                out["can_get_members"] = json!(true);
            }
            "chatTypeSupergroup" => {
                let sg_id = ct["supergroup_id"].as_i64().unwrap_or(0);
                let sg = if sg_id != 0 {
                    self.request(json!({"@type": "getSupergroup", "supergroup_id": sg_id})).await.ok()
                } else {
                    None
                };
                let full = if sg_id != 0 {
                    self.request(json!({"@type": "getSupergroupFullInfo", "supergroup_id": sg_id})).await.ok()
                } else {
                    None
                };
                let sg_val = sg.as_ref().unwrap_or(&Value::Null);
                if !sg_val.is_null() {
                    self.state.remember_supergroup(sg_val);
                }
                let full_val = full.as_ref().unwrap_or(&Value::Null);
                let username = sg_val["usernames"]["active_usernames"][0]
                    .as_str()
                    .or_else(|| sg_val["usernames"]["editable_username"].as_str())
                    .unwrap_or_default();
                let (my_role, is_admin, my_permissions) = member_role_and_perms(&sg_val["status"], &ch["permissions"]);
                let member_count = full_val["member_count"]
                    .as_i64()
                    .filter(|&n| n > 0)
                    .or_else(|| sg_val["member_count"].as_i64())
                    .unwrap_or(0);
                let invite_link = if is_admin { extract_invite_link(full_val) } else { None };
                let can_get_members = full_val["can_get_members"]
                    .as_bool()
                    .unwrap_or(is_admin || kind == "group");
                out["username"] = json!(username);
                out["member_count"] = json!(member_count);
                out["verified"] = json!(super::shape::is_verified(sg_val));
                out["bio"] = json!(text(&full_val["description"]));
                out["my_role"] = json!(my_role);
                out["is_admin"] = json!(is_admin);
                out["my_permissions"] = my_permissions;
                out["invite_link"] = invite_link.map_or(Value::Null, Value::String);
                out["can_get_members"] = json!(can_get_members);
            }
            _ => {}
        }
        Ok(out)
    }

    pub(super) async fn update_chat_settings(&self, chat_id: i64, body: &Value) -> Reply {
        let plan = parse_settings_plan(body)?;
        let mut out = json!({"ok": true, "chat_id": chat_id});

        if let Some(notif) = plan.notification {
            let ch = self.request(json!({"@type": "getChat", "chat_id": chat_id})).await?;
            let mut ns = ch["notification_settings"].as_object().cloned().unwrap_or_default();
            ns.insert("@type".into(), json!("chatNotificationSettings"));
            if let Some(use_def) = notif.use_default_mute_for {
                ns.insert("use_default_mute_for".into(), json!(use_def));
                if use_def && notif.mute_for.is_none() {
                    ns.insert("mute_for".into(), json!(0));
                }
            }
            if let Some(mute_for) = notif.mute_for {
                ns.insert("mute_for".into(), json!(mute_for));
                if notif.use_default_mute_for.is_none() {
                    ns.insert("use_default_mute_for".into(), json!(false));
                }
            }
            if let Some(use_def) = notif.use_default_show_preview {
                ns.insert("use_default_show_preview".into(), json!(use_def));
                if use_def && notif.show_preview.is_none() {
                    ns.insert("show_preview".into(), json!(true));
                }
            }
            if let Some(preview) = notif.show_preview {
                ns.insert("show_preview".into(), json!(preview));
                if notif.use_default_show_preview.is_none() {
                    ns.insert("use_default_show_preview".into(), json!(false));
                }
            }
            self.request(json!({
                "@type": "setChatNotificationSettings",
                "chat_id": chat_id,
                "notification_settings": ns,
            })).await?;
            out["notification"] = json!({
                "use_default_mute_for": truth(ns.get("use_default_mute_for").unwrap_or(&Value::Null)),
                "mute_for": ns.get("mute_for").and_then(Value::as_i64).unwrap_or(0),
                "use_default_show_preview": truth(ns.get("use_default_show_preview").unwrap_or(&Value::Null)),
                "show_preview": ns.get("show_preview").and_then(Value::as_bool).unwrap_or(true),
            });
        }

        if let Some(seconds) = plan.auto_delete_seconds {
            self.request(json!({
                "@type": "setChatMessageAutoDeleteTime",
                "chat_id": chat_id,
                "message_auto_delete_time": seconds,
            })).await?;
            self.state.set_auto_delete(chat_id, seconds);
            out["auto_delete_seconds"] = json!(seconds);
        }

        if let Some(archived) = plan.archive {
            self.request(json!({
                "@type": "addChatToList",
                "chat_id": chat_id,
                "chat_list": chat_list(archived),
            })).await?;
            out["archived"] = json!(archived);
        }

        if let Some(pin) = plan.pin {
            let archived = pin.archived.or(plan.archive).unwrap_or(false);
            self.request(json!({
                "@type": "toggleChatIsPinned",
                "chat_list": chat_list(archived),
                "chat_id": chat_id,
                "is_pinned": pin.pinned,
            })).await?;
            out["pinned"] = json!(pin.pinned);
        }

        if let Some(unread) = plan.mark_unread {
            self.request(json!({
                "@type": "toggleChatIsMarkedAsUnread",
                "chat_id": chat_id,
                "is_marked_as_unread": unread,
            })).await?;
            out["marked_unread"] = json!(unread);
        }

        Ok(out)
    }

    pub(super) async fn clear_chat_history(&self, chat_id: i64, body: &Value) -> Reply {
        let Some(map) = body.as_object() else {
            return Err(GwError::bad_request("Thân POST phải là JSON object."));
        };
        let allowed = ["confirm", "revoke", "remove_from_list"];
        if let Some(bad) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(GwError::bad_request(format!("Trường clear_history không hợp lệ: {bad}")));
        }
        if map.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(GwError::bad_request("Xoá lịch sử hội thoại cần confirm=true."));
        }
        let revoke = map.get("revoke").map(|v| parse_bool(v, "revoke")).transpose()?.unwrap_or(false);
        let remove_from_list = map.get("remove_from_list").map(|v| parse_bool(v, "remove_from_list")).transpose()?.unwrap_or(false);
        self.request(json!({
            "@type": "deleteChatHistory",
            "chat_id": chat_id,
            "remove_from_chat_list": remove_from_list,
            "revoke": revoke,
        })).await?;
        Ok(json!({"ok": true, "chat_id": chat_id, "deleted": true}))
    }

    pub(super) async fn delete_chat(&self, chat_id: i64, body: &Value) -> Reply {
        let Some(map) = body.as_object() else {
            return Err(GwError::bad_request("Thân POST phải là JSON object."));
        };
        let allowed = ["confirm", "revoke"];
        if let Some(bad) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(GwError::bad_request(format!("Trường delete không hợp lệ: {bad}")));
        }
        if map.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(GwError::bad_request("Xoá hội thoại cần confirm=true."));
        }
        let revoke = map.get("revoke").map(|v| parse_bool(v, "revoke")).transpose()?.unwrap_or(false);
        let ch = self.request(json!({"@type": "getChat", "chat_id": chat_id})).await?;
        let is_group = matches!(
            text(&ch["type"]["@type"]),
            "chatTypeSupergroup" | "chatTypeBasicGroup"
        );
        if is_group {
            self.request(json!({"@type": "leaveChat", "chat_id": chat_id})).await?;
        } else {
            self.request(json!({
                "@type": "deleteChatHistory",
                "chat_id": chat_id,
                "remove_from_chat_list": true,
                "revoke": revoke,
            })).await?;
        }
        Ok(json!({"ok": true, "chat_id": chat_id, "deleted": true}))
    }

    pub(super) async fn leave_chat(&self, chat_id: i64, body: &Value) -> Reply {
        let Some(map) = body.as_object() else {
            return Err(GwError::bad_request("Thân POST phải là JSON object."));
        };
        if let Some(bad) = map.keys().find(|k| k.as_str() != "confirm") {
            return Err(GwError::bad_request(format!("Trường leave không hợp lệ: {bad}")));
        }
        if map.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(GwError::bad_request("Rời nhóm/kênh cần confirm=true."));
        }
        self.request(json!({"@type": "leaveChat", "chat_id": chat_id})).await?;
        Ok(json!({"ok": true, "chat_id": chat_id, "left": true}))
    }

    async fn build_member_item(&self, uid: i64, status: &Value) -> Value {
        let user = self.user(uid).await.ok();
        let u = user.as_ref().unwrap_or(&Value::Null);
        let (role, _) = member_role(status);
        let name = user_name(u);
        let username = u["usernames"]["active_usernames"][0]
            .as_str()
            .or_else(|| u["usernames"]["editable_username"].as_str())
            .unwrap_or_default();
        let user_status = text(&u["status"]["@type"]).replace("userStatus", "").to_lowercase();
        json!({
            "user_id": uid,
            "name": name,
            "username": username,
            "avatar_file": u["profile_photo"]["small"]["id"],
            "role": role,
            "custom_title": text(&status["custom_title"]),
            "is_bot": u["type"]["@type"] == "userTypeBot",
            "status": user_status,
        })
    }

    pub(super) async fn chat_members(&self, chat_id: i64, query: &str, limit: i64) -> Reply {
        let q = query.trim();
        if q.chars().count() > 128 {
            return Err(GwError::bad_request("Từ khoá tìm thành viên tối đa 128 ký tự."));
        }
        let limit = limit.clamp(1, MAX_MEMBERS) as usize;
        let ch = self.request(json!({"@type": "getChat", "chat_id": chat_id})).await?;
        let ct = &ch["type"];
        let mut members = Vec::new();
        let mut total_count = match text(&ct["@type"]) {
            "chatTypeSupergroup" => {
                let sg_id = ct["supergroup_id"].as_i64().unwrap_or(0);
                let filter = if q.is_empty() {
                    json!({"@type": "supergroupMembersFilterRecent"})
                } else {
                    json!({"@type": "supergroupMembersFilterSearch", "query": q})
                };
                let res = self.request(json!({
                    "@type": "getSupergroupMembers",
                    "supergroup_id": sg_id,
                    "filter": filter,
                    "offset": 0,
                    "limit": limit,
                })).await?;
                for row in array(&res["members"]) {
                    if let Some(uid) = row["member_id"]["user_id"].as_i64().filter(|&n| n != 0) {
                        members.push(self.build_member_item(uid, &row["status"]).await);
                    }
                }
                res["total_count"].as_i64().unwrap_or(0)
            }
            "chatTypeBasicGroup" => {
                let bg_id = ct["basic_group_id"].as_i64().unwrap_or(0);
                let full = self.request(json!({
                    "@type": "getBasicGroupFullInfo",
                    "basic_group_id": bg_id,
                })).await?;
                let q_lower = q.to_lowercase();
                for row in array(&full["members"]) {
                    let Some(uid) = row["member_id"]["user_id"]
                        .as_i64()
                        .or_else(|| row["user_id"].as_i64())
                        .filter(|&n| n != 0)
                    else {
                        continue;
                    };
                    let item = self.build_member_item(uid, &row["status"]).await;
                    if q_lower.is_empty()
                        || text(&item["name"]).to_lowercase().contains(&q_lower)
                        || text(&item["username"]).to_lowercase().contains(&q_lower)
                    {
                        members.push(item);
                    }
                }
                members.len() as i64
            }
            _ => return Ok(json!({"chat_id": chat_id, "total_count": 0, "members": []})),
        };

        members.sort_by_key(|m| role_rank(text(&m["role"])));
        members.truncate(limit);
        if total_count < members.len() as i64 {
            total_count = members.len() as i64;
        }
        Ok(json!({
            "chat_id": chat_id,
            "total_count": total_count,
            "members": members,
        }))
    }
}
