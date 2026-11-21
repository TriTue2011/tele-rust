//! TDLib giả cục bộ để kiểm hợp đồng HTTP.
use super::*;
use std::{collections::VecDeque,sync::{Condvar,atomic::AtomicI32}};

#[derive(Default)]
pub(crate) struct Fake {
    next:AtomicI32,
    inbox:Mutex<VecDeque<Value>>,
    cv:Condvar,
    states:Mutex<HashMap<i32,Value>>,
    pub sent:Mutex<Vec<(i32,Value)>>,
    pub restored:AtomicBool,
    pub downloads:Mutex<HashMap<i32,String>>,
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
            "getMe"=>json!({"@type":"user","id":id,"first_name":format!("Account {id}"),"phone_number":""}),
            "close"=>{next=Some("authorizationStateClosed");json!({"@type":"ok"})}
            "createCall"=>json!({"@type":"callId","id":71}),
            "downloadFile"=>json!({"@type":"file","local":{"is_downloading_completed":true,
                "path":self.downloads.lock().unwrap().get(&id).cloned().unwrap_or_default()}}),
            "getUser"=>json!({"@type":"user","id":r["user_id"],"first_name":"Peer"}),
            _=>json!({"@type":"ok"}),
        };
        if let Some(next)=next {self.auth(id,json!({"@type":next}));}
        let mut answer=answer;answer["@extra"]=r["@extra"].clone(); self.push(answer,id);
    }
}
