use serde_json::{json, Value};

use super::{array, field, text, truth};

pub(super) const UNKNOWN_USER_LABEL: &str = "Người dùng";

pub(super) fn display_user_name(user: Option<&Value>) -> String {
    if let Some(u) = user.filter(|u| !u.is_null()) {
        let name = format!("{} {}", text(&u["first_name"]), text(&u["last_name"]));
        if !name.trim().is_empty() {
            return name.trim().into();
        }
        if let Some(username) = u["usernames"]["active_usernames"][0]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| u["usernames"]["editable_username"].as_str().filter(|s| !s.is_empty()))
        {
            return format!("@{username}");
        }
    }
    UNKNOWN_USER_LABEL.into()
}

pub(super) fn user_name(u: &Value) -> String {
    display_user_name((!u.is_null()).then_some(u))
}

pub(super) fn is_verified(v: &Value) -> bool {
    truth(&v["is_verified"]) || truth(&v["verification_status"]["is_verified"])
}

pub(super) fn effective_muted(ns: &Value, scope_mute_for: Option<i64>) -> bool {
    if truth(&ns["use_default_mute_for"]) {
        if let Some(default_mute) = scope_mute_for {
            return default_mute > 0;
        }
    }
    ns["mute_for"].as_i64().unwrap_or(0) > 0
}

pub(super) fn user_status_info(status: &Value) -> (bool, String, i64) {
    let ty = text(&status["@type"]);
    let name = ty.strip_prefix("userStatus").unwrap_or(ty).to_lowercase();
    let online = name == "online";
    let was_online = status["was_online"].as_i64().unwrap_or(0).max(0);
    (online, name, was_online)
}

pub(super) fn format_call_duration(seconds: i64) -> String {
    let sec = seconds.max(0);
    if sec == 0 {
        return String::new();
    }
    if sec < 60 {
        return format!("{sec} giây");
    }
    if sec < 3600 {
        let m = sec / 60;
        let s = sec % 60;
        return if s == 0 { format!("{m} phút") } else { format!("{m} phút {s} giây") };
    }
    let h = sec / 3600;
    let m = (sec % 3600) / 60;
    if m == 0 { format!("{h} giờ") } else { format!("{h} giờ {m} phút") }
}

pub(super) fn format_auto_delete_vi(seconds: i64) -> String {
    let sec = seconds.max(0);
    if sec == 0 {
        return "Tắt".into();
    }
    if sec == 86_400 {
        return "1 ngày".into();
    }
    if sec == 604_800 {
        return "1 tuần".into();
    }
    if sec == 2_678_400 {
        return "1 tháng".into();
    }
    let d = sec / 86_400;
    let h = (sec % 86_400) / 3600;
    let m = (sec % 3600) / 60;
    let s = sec % 60;
    let mut parts = Vec::new();
    if d > 0 { parts.push(format!("{d} ngày")); }
    if h > 0 { parts.push(format!("{h} giờ")); }
    if d == 0 && m > 0 { parts.push(format!("{m} phút")); }
    if d == 0 && h == 0 && s > 0 { parts.push(format!("{s} giây")); }
    if parts.is_empty() { format!("{sec} giây") } else { parts.join(" ") }
}

fn call_info(c: &Value, outgoing: bool) -> (Value, String) {
    let is_video = truth(&c["is_video"]);
    let duration = c["duration"].as_i64().unwrap_or(0).max(0);
    let reason = match text(&c["discard_reason"]["@type"]) {
        "callDiscardReasonMissed" => "missed",
        "callDiscardReasonDeclined" => "declined",
        "callDiscardReasonDisconnected" => "disconnected",
        "callDiscardReasonHungUp" => "hung_up",
        "callDiscardReasonEmpty" | "" => "empty",
        _ => "other",
    };
    let prefix = if is_video { "Cuộc gọi video" } else { "Cuộc gọi" };
    let (status, missed, label) = if reason == "missed" && !outgoing {
        ("missed", true, format!("{prefix} nhỡ"))
    } else if reason == "declined" {
        ("declined", false, format!("{prefix} bị từ chối"))
    } else if outgoing {
        ("outgoing", false, format!("{prefix} đi"))
    } else {
        ("incoming", false, format!("{prefix} đến"))
    };
    let duration_text = format_call_duration(duration);
    let summary = if duration_text.is_empty() {
        label.clone()
    } else {
        format!("{label} ({duration_text})")
    };
    let meta = json!({
        "is_video": is_video,
        "outgoing": outgoing,
        "duration": duration,
        "reason": reason,
        "status": status,
        "missed": missed,
        "label": label,
        "duration_text": duration_text,
    });
    (meta, summary)
}

fn content_thumb_file(c: &Value) -> Option<i64> {
    let ty = text(&c["@type"]);
    if ty == "messagePhoto" {
        return array(&c["photo"]["sizes"])
            .iter()
            .filter_map(|s| s["photo"]["id"].as_i64().filter(|&id| id > 0).map(|id| (s["width"].as_i64().unwrap_or(i64::MAX), id)))
            .min_by_key(|(w, _)| *w)
            .map(|(_, id)| id);
    }
    let obj = match ty {
        "messageVideo" => &c["video"],
        "messageAnimation" => &c["animation"],
        "messageVideoNote" => &c["video_note"],
        "messageDocument" => &c["document"],
        "messageSticker" => &c["sticker"],
        _ => return None,
    };
    let thumb = &obj["thumbnail"];
    if matches!(
        text(&thumb["format"]["@type"]),
        "thumbnailFormatMpeg4" | "thumbnailFormatTgs" | "thumbnailFormatWebm"
    ) {
        return None;
    }
    thumb["file"]["id"].as_i64().filter(|&id| id > 0)
}

pub(super) struct MessageContext<'a> {
    pub sender: &'a str,
    pub chat_title: &'a str,
    pub is_private: bool,
    pub resolve_user: Option<&'a dyn Fn(i64) -> Option<String>>,
}

impl<'a> MessageContext<'a> {
    pub(super) fn plain(sender: &'a str) -> Self {
        Self { sender, chat_title: "", is_private: false, resolve_user: None }
    }

    fn user_label(&self, uid: i64) -> String {
        if uid != 0 {
            if let Some(resolver) = self.resolve_user {
                if let Some(name) = resolver(uid) {
                    if !name.trim().is_empty() {
                        return name;
                    }
                }
            }
        }
        UNKNOWN_USER_LABEL.into()
    }

    fn actor(&self, msg: &Value) -> String {
        if truth(&msg["is_outgoing"]) {
            return "Bạn".into();
        }
        if !self.sender.trim().is_empty() {
            return self.sender.trim().into();
        }
        let sender = &msg["sender_id"];
        if text(&sender["@type"]) == "messageSenderUser" {
            if let Some(uid) = sender["user_id"].as_i64().filter(|&id| id != 0) {
                if let Some(resolver) = self.resolve_user {
                    if let Some(name) = resolver(uid) {
                        if !name.trim().is_empty() {
                            return name;
                        }
                    }
                }
            }
        }
        if self.is_private && !self.chat_title.trim().is_empty() {
            return self.chat_title.trim().into();
        }
        UNKNOWN_USER_LABEL.into()
    }
}

pub(super) const DEFAULT_SERVICE_LABEL: &str = "ℹ️ Tin nhắn hệ thống";

fn with_name(prefix: &str, name: &str, fallback: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() { fallback.into() } else { format!("{prefix} «{trimmed}»") }
}

fn service_label(ty: &str) -> Option<&'static str> {
    Some(match ty {
        // Tin hết hạn & chưa hỗ trợ
        "messageExpiredPhoto" => "🖼 Ảnh tự huỷ đã hết hạn",
        "messageExpiredVideo" => "🎬 Video tự huỷ đã hết hạn",
        "messageExpiredVoiceNote" => "🎤 Tin nhắn thoại tự huỷ đã hết hạn",
        "messageExpiredVideoNote" => "⏺ Video tròn tự huỷ đã hết hạn",
        "messageUnsupported" => "⚠️ Tin nhắn chưa được hỗ trợ",
        // Cuộc gọi nhóm & trò chuyện video
        "messageGroupCall" => "👥 Cuộc gọi nhóm",
        "messageVideoChatScheduled" => "📅 Đã lên lịch trò chuyện video",
        "messageInviteVideoChatParticipants" => "🎥 Đã mời tham gia trò chuyện video",
        // Quản trị nhóm / hội thoại
        "messageChatSetBackground" => "🎨 Đã đổi hình nền hội thoại",
        "messageChatSetTheme" => "🎨 Đã đổi chủ đề hội thoại",
        "messageChatBoost" => "⚡ Đã tăng cường cho hội thoại",
        "messageChatShared" => "💬 Đã chia sẻ hội thoại",
        "messageUsersShared" | "messageUserShared" => "👤 Đã chia sẻ người dùng",
        "messageChatOwnerLeft" => "👑 Chủ hội thoại đã rời đi",
        "messageChatOwnerChanged" => "👑 Đã chuyển quyền chủ hội thoại",
        "messageChatHasProtectedContentDisableRequested" => "🛡 Đã yêu cầu tắt bảo vệ nội dung",
        // Hệ thống chung / bot / web app / passport / gợi ý
        "messageProximityAlertTriggered" => "📍 Đã đến gần vị trí",
        "messageGameScore" => "🏆 Điểm trò chơi",
        "messageBotWriteAccessAllowed" => "🤖 Đã cấp quyền nhắn tin cho bot",
        "messageManagedBotCreated" => "🤖 Đã tạo bot được quản lý",
        "messageWebsiteConnected" => "🌐 Đã kết nối trang web",
        "messageWebAppDataSent" => "🧩 Đã gửi dữ liệu ứng dụng web",
        "messageWebAppDataReceived" => "🧩 Đã nhận dữ liệu ứng dụng web",
        "messagePassportDataSent" => "🪪 Đã gửi dữ liệu Telegram Passport",
        "messagePassportDataReceived" => "🪪 Đã nhận dữ liệu Telegram Passport",
        "messageSuggestProfilePhoto" => "🖼 Đã gợi ý ảnh đại diện",
        "messageSuggestBirthdate" => "🎂 Đã gợi ý ngày sinh",
        // Thanh toán, Stars, TON, quà tặng, tặng quà
        "messagePaymentSuccessful" => "💳 Thanh toán thành công",
        "messagePaymentSuccessfulBot" => "💳 Thanh toán qua bot thành công",
        "messagePaymentRefunded" => "↩️ Đã hoàn tiền thanh toán",
        "messageGiftedPremium" => "⭐ Đã tặng Telegram Premium",
        "messagePremiumGiftCode" => "🎁 Mã quà tặng Telegram Premium",
        "messageGiftedStars" => "⭐ Đã tặng Sao Telegram",
        "messageGiftedTon" => "💎 Đã tặng TON",
        "messageGift" => "🎁 Đã gửi quà tặng",
        "messageUpgradedGift" => "🎁 Đã nhận quà tặng nâng cấp",
        "messageRefundedUpgradedGift" => "↩️ Đã hoàn quà tặng nâng cấp",
        "messageUpgradedGiftPurchaseOffer" => "🎁 Đề nghị mua quà tặng nâng cấp",
        "messageUpgradedGiftPurchaseOfferRejected" => "❌ Đề nghị mua quà tặng bị từ chối",
        "messageGiveawayCreated" => "🎉 Đã tạo chương trình tặng quà",
        "messageGiveaway" => "🎉 Chương trình tặng quà",
        "messageGiveawayCompleted" => "🎉 Đã kết thúc tặng quà",
        "messageGiveawayWinners" => "🏆 Công bố người trúng quà",
        "messageGiveawayPrizeStars" => "⭐ Giải thưởng Sao từ tặng quà",
        "messagePaidMessagePriceChanged" => "💫 Đã đổi giá tin nhắn trả phí",
        "messagePaidMessagesRefunded" => "↩️ Đã hoàn phí tin nhắn trả phí",
        "messageDirectMessagePriceChanged" => "💫 Đã đổi giá tin nhắn trực tiếp",
        // Công việc, bình chọn, bài đề xuất
        "messageChecklistTasksDone" => "☑️ Đã hoàn thành mục công việc",
        "messageChecklistTasksAdded" => "☑️ Đã thêm mục công việc",
        "messagePollOptionAdded" => "📊 Đã thêm lựa chọn bình chọn",
        "messagePollOptionDeleted" => "📊 Đã xoá lựa chọn bình chọn",
        "messageSuggestedPostApprovalFailed" => "❌ Không duyệt được bài đề xuất",
        "messageSuggestedPostApproved" => "✅ Đã duyệt bài đề xuất",
        "messageSuggestedPostDeclined" => "❌ Đã từ chối bài đề xuất",
        "messageSuggestedPostPaid" => "💫 Đã thanh toán bài đề xuất",
        "messageSuggestedPostRefunded" => "↩️ Đã hoàn tiền bài đề xuất",
        _ => return None,
    })
}

fn content_with_ctx(m: &Value, ctx: &MessageContext<'_>) -> (&'static str, String, Option<Value>) {
    let c = &m["content"];
    let outgoing = truth(&m["is_outgoing"]);
    let cap = text(&c["caption"]["text"]).trim();
    let caption = |label: &str| if cap.is_empty() { label.into() } else { format!("{label}: {cap}") };
    let sender_uid = (text(&m["sender_id"]["@type"]) == "messageSenderUser")
        .then(|| m["sender_id"]["user_id"].as_i64())
        .flatten();
    let (kind, summary, call_meta) = match text(&c["@type"]) {
        "messageText" => ("text", text(&c["text"]["text"]).into(), None),
        "messagePhoto" => ("photo", caption("🖼 Ảnh"), None),
        "messageVideo" => ("video", caption("🎬 Video"), None),
        "messageAnimation" => ("animation", caption("GIF"), None),
        "messageVoiceNote" => ("voice", caption("🎤 Tin nhắn thoại"), None),
        "messageVideoNote" => ("video_note", caption("⏺ Video tròn"), None),
        "messageAudio" => ("audio", caption(&format!("🎵 {}", nonempty(&c["audio"]["title"], "Âm thanh"))), None),
        "messageDocument" => ("document", caption(&format!("📎 {}", nonempty(&c["document"]["file_name"], "Tệp"))), None),
        "messageSticker" => ("sticker", format!("{} Nhãn dán", text(&c["sticker"]["emoji"])), None),
        "messageCall" => {
            let (meta, text_summary) = call_info(c, outgoing);
            ("call", text_summary, Some(meta))
        }
        "messagePoll" => ("poll", "📊 Bình chọn".into(), None),
        "messageLocation" => ("location", "📍 Vị trí".into(), None),
        "messageVenue" => ("location", with_name("📍 Địa điểm", text(&c["venue"]["title"]), "📍 Địa điểm"), None),
        "messageContact" => ("contact", "👤 Danh bạ".into(), None),
        "messageAnimatedEmoji" => ("other", nonempty(&c["emoji"], "✨ Biểu tượng động").into(), None),
        "messageDice" => ("other", nonempty(&c["emoji"], "🎲 Xúc xắc").into(), None),
        "messageStakeDice" => ("other", "🎲 Xúc xắc".into(), None),
        "messageGame" => ("other", with_name("🎮 Trò chơi", text(&c["game"]["title"]), "🎮 Trò chơi"), None),
        "messagePaidMedia" => ("other", caption("🔒 Nội dung trả phí"), None),
        "messageStory" => ("other", "⭕ Tin 24 giờ".into(), None),
        "messageLiveStory" => ("other", "🔴 Phát trực tiếp".into(), None),
        "messageChecklist" => (
            "other",
            with_name(
                "☑️ Danh sách việc",
                nonempty(&c["list"]["title"]["text"], text(&c["list"]["title"])),
                "☑️ Danh sách việc",
            ),
            None,
        ),
        "messageChatSetMessageAutoDeleteTime" => {
            let sec = c["message_auto_delete_time"].as_i64().unwrap_or(0).max(0);
            let from_uid = c["from_user_id"].as_i64().unwrap_or(0);
            let who = if outgoing {
                "Bạn".to_string()
            } else if from_uid != 0 {
                ctx.resolve_user
                    .and_then(|r| r(from_uid))
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| ctx.actor(m))
            } else {
                ctx.actor(m)
            };
            let msg = if sec == 0 {
                format!("{who} đã tắt hẹn giờ tự hủy tin nhắn")
            } else if from_uid != 0 {
                format!("{who} đã đặt hẹn giờ tự hủy cho mọi cuộc trò chuyện sau {}", format_auto_delete_vi(sec))
            } else {
                format!("{who} đã đặt hẹn giờ tự hủy tin nhắn sau {}", format_auto_delete_vi(sec))
            };
            ("service", msg, None)
        }
        "messageChatSetTtl" => {
            let who = ctx.actor(m);
            let msg = match c["ttl"].as_i64() {
                Some(sec) if sec <= 0 => format!("{who} đã tắt hẹn giờ tự hủy tin nhắn"),
                Some(sec) => format!("{who} đã đặt hẹn giờ tự hủy tin nhắn sau {}", format_auto_delete_vi(sec)),
                None => format!("{who} đã đổi hẹn giờ tự hủy tin nhắn"),
            };
            ("service", msg, None)
        }
        "messageContactRegistered" => ("service", format!("{} đã tham gia Telegram", ctx.actor(m)), None),
        "messageChatJoinByLink" | "messageChatJoinByRequest" => ("service", format!("{} đã tham gia nhóm", ctx.actor(m)), None),
        "messageChatAddMembers" => {
            let member_ids: Vec<i64> = array(&c["member_user_ids"]).iter().filter_map(Value::as_i64).collect();
            let actor = ctx.actor(m);
            let msg = if member_ids.len() == 1 && sender_uid == Some(member_ids[0]) {
                format!("{actor} đã tham gia nhóm")
            } else if member_ids.is_empty() {
                format!("{actor} đã thêm thành viên")
            } else {
                let names: Vec<String> = member_ids.into_iter().map(|uid| ctx.user_label(uid)).collect();
                format!("{actor} đã thêm {}", names.join(", "))
            };
            ("service", msg, None)
        }
        "messageChatDeleteMember" => {
            let target_uid = c["user_id"].as_i64().unwrap_or(0);
            let actor = ctx.actor(m);
            let msg = if target_uid == 0 || sender_uid == Some(target_uid) {
                format!("{actor} đã rời nhóm")
            } else {
                format!("{actor} đã xóa {} khỏi nhóm", ctx.user_label(target_uid))
            };
            ("service", msg, None)
        }
        "messageChatChangeTitle" => {
            let t = text(&c["title"]).trim();
            let actor = ctx.actor(m);
            let msg = if t.is_empty() {
                format!("{actor} đã đổi tên nhóm")
            } else {
                format!("{actor} đã đổi tên nhóm thành «{t}»")
            };
            ("service", msg, None)
        }
        "messageChatChangePhoto" => ("service", format!("{} đã đổi ảnh nhóm", ctx.actor(m)), None),
        "messageChatDeletePhoto" => ("service", format!("{} đã xóa ảnh nhóm", ctx.actor(m)), None),
        "messagePinMessage" => ("service", format!("{} đã ghim một tin nhắn", ctx.actor(m)), None),
        "messageBasicGroupChatCreate" | "messageSupergroupChatCreate" => {
            let actor = ctx.actor(m);
            let t = text(&c["title"]).trim();
            let msg = if t.is_empty() {
                format!("{actor} đã tạo nhóm")
            } else {
                format!("{actor} đã tạo nhóm «{t}»")
            };
            ("service", msg, None)
        }
        "messageChatUpgradeTo" | "messageChatUpgradeFrom" => ("service", "Nhóm đã được nâng cấp lên siêu nhóm".into(), None),
        "messageVideoChatStarted" => ("service", format!("{} đã bắt đầu cuộc trò chuyện video", ctx.actor(m)), None),
        "messageVideoChatEnded" => ("service", "Cuộc trò chuyện video đã kết thúc".into(), None),
        "messageScreenshotTaken" => ("service", format!("{} đã chụp ảnh màn hình", ctx.actor(m)), None),
        "messageForumTopicCreated" => ("service", with_name("💬 Đã tạo chủ đề", text(&c["name"]), "💬 Đã tạo chủ đề"), None),
        "messageForumTopicEdited" => ("service", with_name("💬 Đã sửa chủ đề thành", text(&c["name"]), "💬 Đã sửa chủ đề"), None),
        "messageForumTopicIsClosedToggled" => ("service", match c["is_closed"].as_bool() {
            Some(true) => "🔒 Đã đóng chủ đề",
            Some(false) => "🔓 Đã mở lại chủ đề",
            None => "💬 Đã đổi trạng thái chủ đề",
        }.into(), None),
        "messageForumTopicIsHiddenToggled" => ("service", match c["is_hidden"].as_bool() {
            Some(true) => "🙈 Đã ẩn chủ đề",
            Some(false) => "👁 Đã hiện chủ đề",
            None => "💬 Đã đổi hiển thị chủ đề",
        }.into(), None),
        "messageChatHasProtectedContentToggled" => ("service", match c["has_protected_content"].as_bool() {
            Some(true) => "🔒 Đã bật bảo vệ nội dung",
            Some(false) => "🔓 Đã tắt bảo vệ nội dung",
            None => "🛡 Đã đổi bảo vệ nội dung",
        }.into(), None),
        "messageCustomServiceAction" => {
            let action = text(&c["text"]).trim();
            ("service", if action.is_empty() { "ℹ️ Thông báo hệ thống".into() } else { format!("ℹ️ {action}") }, None)
        }
        "messageInvoice" => (
            "service",
            with_name("🧾 Hoá đơn", nonempty(&c["product_info"]["title"], text(&c["title"])), "🧾 Hoá đơn"),
            None,
        ),
        "" => ("other", String::new(), None),
        t => ("service", service_label(t).unwrap_or(DEFAULT_SERVICE_LABEL).into(), None),
    };
    (kind, summary.trim().into(), call_meta)
}

fn nonempty<'a>(v: &'a Value, default: &'a str) -> &'a str {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(default)
}

fn file_info(f: &Value) -> Option<Value> {
    if !truth(f) { return None; }
    let size = [&f["size"], &f["expected_size"]].into_iter().find(|v| truth(v)).cloned().unwrap_or(json!(0));
    let local = &f["local"];
    let path = if truth(&local["is_downloading_completed"]) { local["path"].clone() } else { json!("") };
    Some(json!({"id":f["id"],"size":size,"path":path}))
}

fn message_entity(e: &Value) -> Option<Value> {
    let offset = e["offset"].as_u64()?;
    let length = e["length"].as_u64().filter(|n| *n > 0)?;
    let ty = &e["type"];
    let kind = match text(&ty["@type"]) {
        "textEntityTypeBold" => "bold",
        "textEntityTypeItalic" => "italic",
        "textEntityTypeUnderline" => "underline",
        "textEntityTypeStrikethrough" => "strikethrough",
        "textEntityTypeCode" => "code",
        "textEntityTypePre" | "textEntityTypePreCode" => "pre",
        "textEntityTypeBlockQuote" | "textEntityTypeExpandableBlockQuote" => "blockquote",
        "textEntityTypeSpoiler" => "spoiler",
        "textEntityTypeUrl" => "url",
        "textEntityTypeTextUrl" => "text_link",
        "textEntityTypeMention" => "mention",
        "textEntityTypeMentionName" => "text_mention",
        _ => return None,
    };
    // TDLib và chuỗi JavaScript đều dùng vị trí UTF-16.
    let mut entity = json!({"type":kind,"offset":offset,"length":length});
    match kind {
        "text_link" => entity["url"] = json!(ty["url"].as_str()?),
        "text_mention" => entity["user_id"] = json!(ty["user_id"].as_i64()?),
        "pre" => if let Some(language) = ty["language"].as_str() { entity["language"] = json!(language); },
        _ => (),
    }
    Some(entity)
}

pub(super) fn message_with_ctx(m: &Value, ctx: &MessageContext<'_>) -> Value {
    let c = &m["content"];
    let (kind, summary, call_meta) = content_with_ctx(m, ctx);
    let formatted = &c[if kind == "text" { "text" } else { "caption" }];
    let entities: Vec<_> = array(&formatted["entities"]).iter().filter_map(message_entity).take(100).collect();
    let reactions: Vec<_> = array(&m["interaction_info"]["reactions"]["reactions"]).iter().map(|r| json!({
        "emoji":nonempty(&r["type"]["emoji"],"?"),"count":field(r,"total_count",json!(0)),"mine":truth(&r["is_chosen"])
    })).collect();
    let file = if kind == "photo" {
        let sizes = array(&c["photo"]["sizes"]);
        let width = |v: &Value| v["width"].as_i64().unwrap_or(0);
        let best = sizes.iter().rev().filter(|s| width(s) <= 1280).max_by_key(|s| width(s)).or_else(|| sizes.last());
        best.map(|s| {
            let mut info = file_info(&s["photo"]).unwrap_or_else(|| json!({}));
            info["width"] = s["width"].clone();
            info["height"] = s["height"].clone();
            info["mime"] = json!("image/jpeg");
            info
        })
    } else if matches!(kind, "video" | "animation" | "audio" | "document" | "voice" | "video_note" | "sticker") {
        let obj = &c[if kind == "voice" { "voice_note" } else { kind }];
        ["video", "animation", "audio", "document", "voice", "video_note", "sticker"].iter()
            .find_map(|k| file_info(&obj[*k])).map(|mut f| {
                for (key, source, default) in [("name","file_name",json!("")),("mime","mime_type",json!("")),
                    ("duration","duration",json!(0)),("width","width",json!(0)),("height","height",json!(0))] {
                    f[key] = field(obj, source, default);
                }
                f
            })
    } else { None };
    let mut out = json!({"id":m["id"],"chat_id":m["chat_id"],"date":field(m,"date",json!(0)),"edit_date":field(m,"edit_date",json!(0)),
        "outgoing":truth(&m["is_outgoing"]),"sender":ctx.sender,"kind":kind,
        "text":text(&formatted["text"]),"entities":entities,"summary":summary,
        "reply_to":m["reply_to"]["message_id"].as_i64().unwrap_or(0),"reactions":reactions,"file":file});
    if let Some(call) = call_meta {
        out["call"] = call;
    }
    out
}

pub(super) fn message(m: &Value, sender: &str) -> Value {
    message_with_ctx(m, &MessageContext::plain(sender))
}

#[derive(Default)]
pub(super) struct SummarizeContext<'a> {
    pub last_sender: &'a str,
    pub scope_mute_for: Option<i64>,
    pub verified: bool,
    pub online: bool,
    pub status: &'a str,
    pub was_online: i64,
    pub member_count: i64,
    pub resolve_user: Option<&'a dyn Fn(i64) -> Option<String>>,
}

pub(super) fn summarize_with_ctx(chat: &Value, ctx: &SummarizeContext<'_>) -> Value {
    let ct = &chat["type"];
    let kind = match text(&ct["@type"]) {
        "chatTypePrivate" => "private",
        "chatTypeSecret" => "secret",
        "chatTypeSupergroup" if truth(&ct["is_channel"]) => "channel",
        "chatTypeSupergroup" | "chatTypeBasicGroup" => "group",
        _ => "other",
    };
    let title = text(&chat["title"]);
    let is_private = matches!(kind, "private" | "secret");
    let positions = array(&chat["positions"]);
    let pinned = positions.iter().any(|p| truth(&p["is_pinned"]));
    let archived = positions.iter().any(|p| p["list"]["@type"] == "chatListArchive");
    let last_read_outbox = chat["last_read_outbox_message_id"].as_i64().unwrap_or(0).max(0);
    let lm = &chat["last_message"];
    let last = if truth(lm) {
        let msg_ctx = MessageContext {
            sender: ctx.last_sender,
            chat_title: title,
            is_private,
            resolve_user: ctx.resolve_user,
        };
        let (msg_kind, summary, call_meta) = content_with_ctx(lm, &msg_ctx);
        let outgoing = truth(&lm["is_outgoing"]);
        let lm_id = lm["id"].as_i64().unwrap_or(0);
        let read = outgoing && lm_id > 0 && last_read_outbox >= lm_id;
        let thumb_file = content_thumb_file(&lm["content"]);
        let cap = text(&lm["content"]["caption"]["text"]).trim();
        let last_sender = if (kind == "group" || kind == "channel")
            && !outgoing
            && msg_kind != "service"
            && msg_kind != "call"
            && !ctx.last_sender.is_empty()
            && ctx.last_sender != title
        {
            ctx.last_sender
        } else {
            ""
        };
        let mut obj = json!({
            "id": lm_id,
            "date": field(lm, "date", json!(0)),
            "outgoing": outgoing,
            "kind": msg_kind,
            "text": summary.chars().take(120).collect::<String>(),
            "sender": last_sender,
            "read": read,
            "thumb_file": thumb_file,
        });
        if !cap.is_empty() && msg_kind != "text" {
            obj["caption"] = json!(cap.chars().take(120).collect::<String>());
        }
        if let Some(call) = call_meta {
            obj["call"] = call;
        }
        obj
    } else {
        Value::Null
    };
    json!({
        "id": chat["id"],
        "title": field(chat, "title", json!("")),
        "kind": kind,
        "unread": field(chat, "unread_count", json!(0)),
        "unread_mentions": field(chat, "unread_mention_count", json!(0)),
        "pinned": pinned,
        "archived": archived,
        "marked_unread": truth(&chat["is_marked_as_unread"]),
        "muted": effective_muted(&chat["notification_settings"], ctx.scope_mute_for),
        "verified": ctx.verified || is_verified(chat),
        "online": ctx.online,
        "status": ctx.status,
        "was_online": ctx.was_online,
        "member_count": ctx.member_count,
        "auto_delete_seconds": chat["message_auto_delete_time"].as_i64().unwrap_or(0).max(0),
        "last_read_outbox_message_id": last_read_outbox,
        "last": last,
        "avatar_file": chat["photo"]["small"]["id"],
        "user_id": ct["user_id"],
    })
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn summarize(chat: &Value) -> Value {
    summarize_with_ctx(chat, &SummarizeContext::default())
}
