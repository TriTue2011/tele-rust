//! Danh mục cố định từ `td_services.py`; không chuyển tiếp JSON tuỳ ý sang TDLib.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{array, field, int, string, text, truth, user_name, write::formatted_text, ChatService, GwError, Reply};

const YEAR: i64 = 366 * 24 * 3600;

struct Args<'a>(&'a Value);

impl Args<'_> {
    fn int(&self, key: &str) -> Result<i64, GwError> { int(&self.0[key], key) }
    fn bool(&self, key: &str, default: bool) -> bool { self.0.get(key).map_or(default, truth) }
    fn str(&self, key: &str) -> String { string(&self.0[key]) }
    fn range(&self, key: &str, default: Option<i64>, low: i64, high: Option<i64>) -> Result<i64, GwError> {
        let n = match (self.0.get(key), default) {
            (None, Some(n)) => n,
            _ => self.int(key)?,
        };
        if n < low || high.is_some_and(|h| n > h) {
            return Err(GwError::bad_request(format!("{key} phải trong khoảng {low}..{}", high.map_or_else(|| "None".into(), |h| h.to_string()))));
        }
        Ok(n)
    }
    fn ids(&self, key: &str) -> Result<Vec<i64>, GwError> {
        let value = &self.0[key];
        if value.is_null() || value == "" { return Ok(Vec::new()); }
        let values: Vec<_> = if let Some(items) = value.as_array() {
            items.iter().map(|v| match v.as_str() { Some(s) => s.to_owned(), None => v.to_string() }).collect()
        } else {
            value.as_str().map_or_else(|| value.to_string(), str::to_owned).split([',', ';']).map(str::to_owned).collect()
        };
        values.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| int(&json!(s), key)).collect()
    }
    fn chat(&self) -> Result<i64, GwError> { self.int("chat_id") }
    fn user(&self) -> Result<i64, GwError> { self.range("user_id", None, 1, None) }
    fn message(&self) -> Result<i64, GwError> { self.range("message_id", None, 1, None) }
}

fn chat_list(archive: bool) -> Value { json!({"@type":if archive {"chatListArchive"} else {"chatListMain"}}) }
fn chat_type(chat: &Value) -> String { text(&chat["type"]["@type"]).replace("chatType", "").to_lowercase() }
fn sender(id: i64) -> Value { json!({"@type":"messageSenderUser","user_id":id}) }
fn now() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }

fn title(a: &Args<'_>) -> Result<String, GwError> {
    let title = a.str("title").trim().to_owned();
    if !(1..=128).contains(&title.chars().count()) { return Err(GwError::bad_request("title phải 1-128 ký tự")); }
    Ok(title)
}

fn user_brief(u: &Value) -> Value {
    let mut out = json!({"user_id":u["id"],"name":user_name(u),"username":text(&u["usernames"]["active_usernames"][0]),
        "is_bot":u["type"]["@type"]=="userTypeBot"});
    if truth(&u["phone_number"]) { out["phone"] = json!(format!("+{}",string(&u["phone_number"]).trim_start_matches('+'))); }
    let status = text(&u["status"]["@type"]).replace("userStatus", "");
    if !status.is_empty() { out["status"] = json!(status.to_lowercase()); }
    out
}

impl ChatService {
    pub(super) async fn service(&self, name: &str, body: &Value) -> Reply {
        let a = Args(body);
        let (req, out) = match name {
            "set_mute" => {
                let chat = a.chat()?;
                let n = a.range("mute_for", Some(0), -1, Some(YEAR))?;
                let ch = self.request(json!({"@type":"getChat","chat_id":chat})).await?;
                let mut ns = ch["notification_settings"].as_object().cloned().unwrap_or_default();
                ns.insert("@type".into(),json!("chatNotificationSettings"));
                ns.insert("use_default_mute_for".into(),json!(n<0));
                ns.insert("mute_for".into(),json!(n.max(0)));
                (json!({"@type":"setChatNotificationSettings","chat_id":chat,"notification_settings":ns}),json!({"chat_id":chat,"mute_for":n}))
            }
            "pin_chat" => {
                let (chat, pinned) = (a.chat()?, a.bool("pinned", true));
                (json!({"@type":"toggleChatIsPinned","chat_list":chat_list(a.bool("archived",false)),"chat_id":chat,"is_pinned":pinned}),
                    json!({"chat_id":chat,"pinned":pinned}))
            }
            "archive_chat" => {
                let (chat, archive) = (a.chat()?,a.bool("archive",true));
                (json!({"@type":"addChatToList","chat_id":chat,"chat_list":chat_list(archive)}),json!({"chat_id":chat,"archived":archive}))
            }
            "mark_chat_unread" => {
                let (chat, unread) = (a.chat()?,a.bool("unread",true));
                (json!({"@type":"toggleChatIsMarkedAsUnread","chat_id":chat,"is_marked_as_unread":unread}),json!({"chat_id":chat,"unread":unread}))
            }
            "set_auto_delete" => {
                let (chat, seconds) = (a.chat()?,a.range("seconds",Some(0),0,Some(YEAR))?);
                self.request(json!({"@type":"setChatMessageAutoDeleteTime","chat_id":chat,"message_auto_delete_time":seconds})).await?;
                self.state.set_auto_delete(chat, seconds);
                return Ok(json!({"chat_id":chat,"seconds":seconds}));
            }
            "send_typing" => {
                let chat = a.chat()?;
                let act = a.str("action");
                let act = if act.is_empty() { "typing" } else { &act };
                let kind = match act {
                    "typing" => "chatActionTyping",
                    "recording_voice" => "chatActionRecordingVoiceNote",
                    "cancel" => "chatActionCancel",
                    _ => return Err(GwError::bad_request("action chỉ gồm: typing, recording_voice, cancel")),
                };
                (json!({"@type":"sendChatAction","chat_id":chat,"business_connection_id":"","action":{"@type":kind}}),json!({"chat_id":chat,"action":act}))
            }
            "delete_chat_history" => {
                let chat = a.chat()?;
                // Xoá toàn bộ lịch sử là không thể hoàn tác (td_services.py).
                if body["confirm"] != true { return Err(GwError::bad_request("Xoá lịch sử cần confirm=true.")); }
                (json!({"@type":"deleteChatHistory","chat_id":chat,"remove_from_chat_list":a.bool("remove_from_list",false),"revoke":a.bool("revoke",false)}),
                    json!({"chat_id":chat,"deleted":true}))
            }
            "rename_chat" => {
                let (chat, title) = (a.chat()?,title(&a)?);
                (json!({"@type":"setChatTitle","chat_id":chat,"title":title}),json!({"chat_id":chat,"title":title}))
            }
            "leave_chat" => {
                let chat = a.chat()?;
                (json!({"@type":"leaveChat","chat_id":chat}),json!({"chat_id":chat,"left":true}))
            }
            "get_chat_info" => {
                let id = a.chat()?;
                let ch = self.request(json!({"@type":"getChat","chat_id":id})).await?;
                let auto_delete_seconds = self.state.sync_chat_auto_delete(
                    ch["id"].as_i64().unwrap_or(id),
                    ch["message_auto_delete_time"].as_i64().unwrap_or(0),
                );
                return Ok(json!({"chat_id":field(&ch,"id",json!(id)),"title":field(&ch,"title",json!("")),"kind":chat_type(&ch),
                    "unread":field(&ch,"unread_count",json!(0)),"muted_for":field(&ch["notification_settings"],"mute_for",json!(0)),
                    "auto_delete_seconds":auto_delete_seconds,"has_protected_content":truth(&ch["has_protected_content"])}));
            }
            "pin_message" | "unpin_message" => {
                let (chat, id) = (a.chat()?,a.message()?);
                let pinned = name == "pin_message";
                let req = if pinned {
                    json!({"@type":"pinChatMessage","chat_id":chat,"message_id":id,"disable_notification":!a.bool("notify",false),"only_for_self":a.bool("only_for_self",false)})
                } else { json!({"@type":"unpinChatMessage","chat_id":chat,"message_id":id}) };
                (req,json!({"chat_id":chat,"message_id":id,"pinned":pinned}))
            }
            "send_poll" => return self.send_poll(&a).await,
            "vote_poll" => {
                let (chat, id, options) = (a.chat()?,a.message()?,a.ids("options")?);
                if options.iter().any(|&n| n < 1) { return Err(GwError::bad_request("options đánh số từ 1")); }
                let ids: Vec<_> = options.iter().map(|n| n-1).collect();
                (json!({"@type":"setPollAnswer","chat_id":chat,"message_id":id,"option_ids":ids}),json!({"chat_id":chat,"message_id":id,"voted":options}))
            }
            "stop_poll" => {
                let (chat, id) = (a.chat()?,a.message()?);
                (json!({"@type":"stopPoll","chat_id":chat,"message_id":id}),json!({"chat_id":chat,"message_id":id,"closed":true}))
            }
            "find_user" => return self.find_user(&a).await,
            "get_user_info" => return self.request(json!({"@type":"getUser","user_id":a.user()?})).await.map(|u| user_brief(&u)),
            "add_contact" => {
                let user = a.user()?;
                let first = a.str("first_name").trim().to_owned();
                if first.is_empty() { return Err(GwError::bad_request("first_name không được trống")); }
                (json!({"@type":"addContact","user_id":user,"share_phone_number":a.bool("share_phone",false),
                    "contact":{"@type":"importedContact","phone_number":a.str("phone"),"first_name":first,"last_name":a.str("last_name"),"note":formatted_text("")}}),
                    json!({"user_id":user,"added":true}))
            }
            "remove_contact" => {
                let user = a.user()?;
                (json!({"@type":"removeContacts","user_ids":[user]}),json!({"user_id":user,"removed":true}))
            }
            "block_user" | "unblock_user" => {
                let user = a.user()?;
                let blocked = name == "block_user";
                let mut req = json!({"@type":"setMessageSenderBlockList","sender_id":sender(user)});
                if blocked { req["block_list"] = json!({"@type":"blockListMain"}); }
                (req,json!({"user_id":user,"blocked":blocked}))
            }
            "create_group" => {
                let users = a.ids("user_ids")?;
                let title = title(&a)?;
                if users.is_empty() { return Err(GwError::bad_request("user_ids cần ít nhất một người")); }
                let r = self.request(json!({"@type":"createNewBasicGroupChat","user_ids":users,"title":title,"message_auto_delete_time":0})).await?;
                let failed: Vec<_> = array(&r["failed_to_add_members"]["failed_to_add_members"]).iter().map(|f| &f["user_id"]).filter(|id| truth(id)).collect();
                return Ok(json!({"chat_id":r["chat_id"],"title":title,"failed_user_ids":failed}));
            }
            "add_chat_member" => {
                let (chat, user, limit) = (a.chat()?,a.user()?,a.range("forward_limit",Some(0),0,Some(100))?);
                let r = self.request(json!({"@type":"addChatMember","chat_id":chat,"user_id":user,"forward_limit":limit})).await?;
                return Ok(json!({"chat_id":chat,"user_id":user,"added":!truth(&r["failed_to_add_members"])}));
            }
            "remove_chat_member" => {
                let (chat, user) = (a.chat()?,a.user()?);
                (json!({"@type":"setChatMemberStatus","chat_id":chat,"member_id":sender(user),"status":{"@type":"chatMemberStatusBanned","banned_until_date":now()+60}}),
                    json!({"chat_id":chat,"user_id":user,"removed":true}))
            }
            "create_invite_link" => {
                let (chat, exp, limit) = (a.chat()?,a.range("expire_seconds",Some(0),0,None)?,a.range("member_limit",Some(0),0,Some(99999))?);
                let expiration = if exp == 0 { 0 } else { now().checked_add(exp).ok_or_else(|| GwError::bad_request("expire_seconds quá lớn"))? };
                let link = self.request(json!({"@type":"createChatInviteLink","chat_id":chat,"name":a.str("name"),"expiration_date":expiration,
                    "member_limit":limit,"creates_join_request":a.bool("join_request",false)})).await?;
                return Ok(json!({"chat_id":chat,"invite_link":field(&link,"invite_link",json!(""))}));
            }
            "join_chat" => {
                let link = a.str("invite_link");
                let link = link.trim();
                if !["https://t.me/","http://t.me/","t.me/","https://telegram.me/"].iter().any(|p| link.starts_with(p)) {
                    return Err(GwError::bad_request("invite_link phải là liên kết t.me"));
                }
                let r = self.request(json!({"@type":"joinChatByInviteLink","invite_link":link})).await?;
                return Ok(json!({"joined":true,"chat_id":r["chat_id"]}));
            }
            "get_me" => return self.request(json!({"@type":"getMe"})).await.map(|u| user_brief(&u)),
            "set_name" => {
                let (first, last) = (a.str("first_name").trim().to_owned(),a.str("last_name").trim().to_owned());
                if !(1..=64).contains(&first.chars().count()) || last.chars().count()>64 {
                    return Err(GwError::bad_request("first_name 1-64 ký tự, last_name tối đa 64"));
                }
                (json!({"@type":"setName","first_name":first,"last_name":last}),json!({"first_name":first,"last_name":last}))
            }
            "set_bio" => {
                let bio = a.str("bio").trim().to_owned();
                if bio.chars().count()>70 { return Err(GwError::bad_request("bio tối đa 70 ký tự")); }
                (json!({"@type":"setBio","bio":bio}),json!({"bio":bio}))
            }
            "set_username" => {
                let user = a.str("username");
                let user = user.trim().trim_start_matches('@');
                (json!({"@type":"setUsername","username":user}),json!({"username":user}))
            }
            _ => return Err(GwError::not_found(format!("Không có dịch vụ «{name}»."))),
        };
        self.request(req).await?;
        Ok(out)
    }

    async fn find_user(&self, a: &Args<'_>) -> Reply {
        let (phone, username) = (a.str("phone"),a.str("username"));
        let (phone, username) = (phone.trim(),username.trim().trim_start_matches('@'));
        if phone.is_empty() == username.is_empty() { return Err(GwError::bad_request("nêu đúng MỘT trong phone hoặc username")); }
        if !phone.is_empty() {
            let u = self.request(json!({"@type":"searchUserByPhoneNumber","phone_number":phone.trim_start_matches('+').replace(' ',""),"only_local":false})).await?;
            return Ok(user_brief(&u));
        }
        let ch = self.request(json!({"@type":"searchPublicChat","username":username})).await?;
        if ch["type"]["@type"] != "chatTypePrivate" {
            return Ok(json!({"user_id":null,"name":field(&ch,"title",json!("")),"username":username,"kind":chat_type(&ch),"chat_id":ch["id"]}));
        }
        let u = self.request(json!({"@type":"getUser","user_id":ch["type"]["user_id"]})).await?;
        Ok(user_brief(&u))
    }

    async fn send_poll(&self, a: &Args<'_>) -> Reply {
        let (chat, question) = (a.chat()?,a.str("question"));
        let question = question.trim();
        if !(1..=300).contains(&question.chars().count()) { return Err(GwError::bad_request("question phải 1-300 ký tự")); }
        let raw = &a.0["options"];
        let supplied = string(raw);
        let opts: Vec<_> = if let Some(options) = raw.as_array() {
            options.iter().map(|o| o.as_str().map(str::trim).ok_or_else(|| GwError::bad_request("options phải là văn bản"))).collect::<Result<_,_>>()?
        } else { supplied.split('\n').map(str::trim).collect() };
        let options: Vec<_> = opts.into_iter().filter(|s| !s.is_empty()).map(|s| json!({"@type":"inputPollOption","text":formatted_text(s)})).collect();
        if !(2..=10).contains(&options.len()) { return Err(GwError::bad_request("options cần 2-10 lựa chọn")); }
        let quiz = &a.0["quiz_answer"];
        let (typ, multiple) = if quiz.is_null() || quiz == "" || quiz == false || quiz.as_f64() == Some(0.0) {
            (json!({"@type":"inputPollTypeRegular","allow_adding_options":false}),a.bool("multiple",false))
        } else {
            let n = a.range("quiz_answer",None,1,Some(options.len() as i64))?;
            (json!({"@type":"inputPollTypeQuiz","correct_option_ids":[n-1],"explanation":formatted_text(&a.str("explanation"))}),false)
        };
        let m = self.request(json!({"@type":"sendMessage","chat_id":chat,"input_message_content":{"@type":"inputMessagePoll",
            "question":formatted_text(question),"options":options,"is_anonymous":a.bool("anonymous",true),"allows_multiple_answers":multiple,"type":typ}})).await?;
        Ok(json!({"chat_id":chat,"message_id":m["id"],"options":options.len()}))
    }
}
