//! TDLib giả cục bộ để kiểm hợp đồng HTTP.
use super::*;
use std::{collections::VecDeque,sync::{Condvar,atomic::AtomicI32}};

#[derive(Default)]
pub(crate) struct Fake {
    next:AtomicI32,
    inbox:Arc<Mutex<VecDeque<Value>>>,
    cv:Arc<Condvar>,
    states:Mutex<HashMap<i32,Value>>,
    pub sent:Mutex<Vec<(i32,Value)>>,
    pub restored:AtomicBool,
    pub downloads:Mutex<HashMap<i32,String>>,
    pub profiles:Mutex<HashMap<i32,Value>>,
    pub user_full_info:Mutex<HashMap<i64,Value>>,
    pub storage_stats:Mutex<Value>,
    pub storage_chats:Mutex<HashMap<i64, Value>>,
    pub chats:Mutex<HashMap<i64, Value>>,
    pub strict_chats:AtomicBool,
    pub users:Mutex<HashMap<i64, Value>>,
    pub basic_groups:Mutex<HashMap<i64, Value>>,
    pub basic_group_full_info:Mutex<HashMap<i64, Value>>,
    pub supergroups:Mutex<HashMap<i64, Value>>,
    pub supergroup_full_info:Mutex<HashMap<i64, Value>>,
    pub supergroup_members:Mutex<HashMap<i64, Value>>,
    pub optimize_reply:Mutex<Option<Box<dyn Fn(&Value)->Value+std::marker::Send+std::marker::Sync>>>,
    pub optimize_delay:Mutex<Option<Duration>>,
}
impl Fake {
    pub fn push(&self,mut v:Value,id:i32) {
        v["@client_id"]=json!(id);
        self.inbox.lock().unwrap().push_back(v); self.cv.notify_one();
    }
    pub fn auth(&self,id:i32,state:Value) {
        self.states.lock().unwrap().insert(id,state.clone());
        self.push(json!({"@type":"updateAuthorizationState","authorization_state":state}),id);
    }
    pub fn factory()->(Arc<Self>,crate::account_router::ClientFactory) {
        let api=Arc::new(Self::default());
        let receiver=Receiver::start(api.clone());
        (api,Arc::new(move |p|TdClient::start(receiver.clone(),p)))
    }
}
impl TdApi for Fake {
    fn create_client_id(&self)->i32 {
        let id=self.next.fetch_add(1,Ordering::Relaxed)+1;
        self.states.lock().unwrap().insert(id,json!({"@type":"authorizationStateWaitTdlibParameters"}));
        id
    }
    fn execute(&self,_:&Value)->Option<Value>{None}
    fn receive(&self,timeout:f64)->Option<Value> {
        let q=self.inbox.lock().unwrap();
        let (mut q,_)=self.cv.wait_timeout_while(q,Duration::from_secs_f64(timeout),|q|q.is_empty()).unwrap();
        q.pop_front()
    }
    fn send(&self,id:i32,r:&Value) {
        self.sent.lock().unwrap().push((id,r.clone()));
        let mut next=None;
        let answer=match r["@type"].as_str().unwrap_or_default() {
            "getAuthorizationState"=>{
                let state=self.states.lock().unwrap()[&id].clone();
                if r["@extra"]=="khoi-dong" {self.auth(id,state.clone());}
                state
            }
            "setTdlibParameters"=>{next=Some(if self.restored.load(Ordering::Relaxed) {"authorizationStateReady"}else{"authorizationStateWaitPhoneNumber"});json!({"@type":"ok"})}
            "setAuthenticationPhoneNumber" if r["phone_number"].as_str().unwrap_or_default().ends_with("300")=>
                json!({"@type":"error","code":429,"message":"FLOOD_WAIT_321"}),
            "setAuthenticationPhoneNumber"=>{next=Some("authorizationStateWaitCode");json!({"@type":"ok"})}
            "requestQrCodeAuthentication"=>{
                self.auth(id,json!({"@type":"authorizationStateWaitOtherDeviceConfirmation","link":"tg://login?token=qr-test-secret"}));
                json!({"@type":"ok"})
            }
            "checkAuthenticationCode" if r["code"]=="54321"=>{next=Some("authorizationStateWaitPassword");json!({"@type":"ok"})}
            "checkAuthenticationCode" if r["code"]=="22222"=>{next=Some("authorizationStateReady");json!({"@type":"ok"})}
            "checkAuthenticationCode" if r["code"]=="33333"=>json!({"@type":"error","code":429,"message":"FLOOD_WAIT_123"}),
            "checkAuthenticationPassword" if r["password"]==" secret2fa "=>{next=Some("authorizationStateReady");json!({"@type":"ok"})}
            "checkAuthenticationCode"|"checkAuthenticationPassword"=>json!({"@type":"error","code":400,"message":format!("invalid {:?}",r)}),
            "getMe"=>self.profiles.lock().unwrap().get(&id).cloned()
                .unwrap_or_else(||json!({"@type":"user","id":id,"first_name":format!("Account {id}"),"phone_number":""})),
            "getUserFullInfo"=>self.user_full_info.lock().unwrap().get(&r["user_id"].as_i64().unwrap_or_default()).cloned()
                .unwrap_or_else(||json!({"@type":"userFullInfo","bio":{"@type":"formattedText","text":"","entities":[]}})),
            "close"=>{next=Some("authorizationStateClosed");json!({"@type":"ok"})}
            "createCall"=>json!({"@type":"callId","id":71}),
            "getStorageStatistics"=>self.storage_stats.lock().unwrap().as_object().cloned().map(Value::Object)
                .unwrap_or_else(||json!({"@type":"storageStatistics","size":0,"count":0,"by_chat":[]})),
            "getChats" => {
                let mut ids: Vec<i64> = self.chats.lock().unwrap().keys().copied().collect();
                ids.sort();
                json!({"@type":"chats","total_count":ids.len(),"chat_ids":ids})
            }
            "getChat" if r["chat_id"].as_i64().is_some_and(|cid| self.chats.lock().unwrap().contains_key(&cid)) => {
                self.chats.lock().unwrap().get(&r["chat_id"].as_i64().unwrap()).cloned().unwrap()
            }
            "getChat" if r["chat_id"].as_i64().is_some_and(|id| self.storage_chats.lock().unwrap().contains_key(&id)) => {
                let kind = self.storage_chats.lock().unwrap().get(&r["chat_id"].as_i64().unwrap()).cloned();
                json!({"@type":"chat","type":kind})
            }
            "getChat" if self.strict_chats.load(Ordering::Relaxed) => {
                json!({"@type":"error","code":404,"message":"Chat not found"})
            }
            "getBasicGroup" => self.basic_groups.lock().unwrap().get(&r["basic_group_id"].as_i64().unwrap_or_default()).cloned()
                .unwrap_or_else(|| json!({"@type":"basicGroup","id":r["basic_group_id"],"member_count":0,"status":{"@type":"chatMemberStatusMember"}})),
            "getBasicGroupFullInfo" => self.basic_group_full_info.lock().unwrap().get(&r["basic_group_id"].as_i64().unwrap_or_default()).cloned()
                .unwrap_or_else(|| json!({"@type":"basicGroupFullInfo","description":"","members":[]})),
            "getSupergroup" => self.supergroups.lock().unwrap().get(&r["supergroup_id"].as_i64().unwrap_or_default()).cloned()
                .unwrap_or_else(|| json!({"@type":"supergroup","id":r["supergroup_id"],"member_count":0,"status":{"@type":"chatMemberStatusMember"}})),
            "getSupergroupFullInfo" => self.supergroup_full_info.lock().unwrap().get(&r["supergroup_id"].as_i64().unwrap_or_default()).cloned()
                .unwrap_or_else(|| json!({"@type":"supergroupFullInfo","description":"","member_count":0,"can_get_members":true})),
            "getSupergroupMembers" => self.supergroup_members.lock().unwrap().get(&r["supergroup_id"].as_i64().unwrap_or_default()).cloned()
                .unwrap_or_else(|| json!({"@type":"chatMembers","total_count":0,"members":[]})),
            "deleteChatHistory" | "leaveChat" if self.strict_chats.load(Ordering::Relaxed)
                && !r["chat_id"].as_i64().is_some_and(|cid| self.chats.lock().unwrap().contains_key(&cid)) => {
                json!({"@type":"error","code":404,"message":"Chat not found"})
            }
            "optimizeStorage"=>{
                let reply = self.optimize_reply.lock().unwrap().as_ref().map(|f| f(r))
                    .unwrap_or_else(|| json!({"@type":"storageStatistics","size":0,"count":0,"by_chat":[]}));
                if let Some(delay) = *self.optimize_delay.lock().unwrap() {
                    let mut answer = reply;
                    answer["@extra"] = r["@extra"].clone();
                    answer["@client_id"] = json!(id);
                    let inbox = self.inbox.clone();
                    let cv = self.cv.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(delay);
                        inbox.lock().unwrap().push_back(answer);
                        cv.notify_one();
                    });
                    return;
                }
                reply
            }
            "downloadFile"=>json!({"@type":"file","local":{"is_downloading_completed":true,
                "path":self.downloads.lock().unwrap().get(&id).cloned().unwrap_or_default()}}),
            "getUser"=>self.users.lock().unwrap().get(&r["user_id"].as_i64().unwrap_or_default()).cloned()
                .unwrap_or_else(||json!({"@type":"user","id":r["user_id"],"first_name":"Peer"})),
            _=>json!({"@type":"ok"}),
        };
        if let Some(next)=next {self.auth(id,json!({"@type":next}));}
        let mut answer=answer;answer["@extra"]=r["@extra"].clone(); self.push(answer,id);
    }
}
