//! Tách dịch vụ, phiên và cập nhật theo tài khoản; không đổi các hợp đồng backend.
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, Mutex, RwLock, atomic::{AtomicBool, Ordering}}, time::Duration};
use serde_json::{json, Value};
use crate::{accounts::AccountStore, calls::CallManager, chat::ChatService,
    contract::{AudioOut, Binary, BoxFut, CallControl, CastAudio, ChatControl, GwError, TdLink},
    events::{EventBridge, EventStore, Webhook}, td::{self, login::LoginConfig, TdClient}};

pub(crate) struct Entry {
    pub client: Arc<TdClient>,
    chat: Arc<ChatService>,
    calls: CallManager,
    ready: AtomicBool,
    events: tokio::sync::Mutex<Option<EventBridge>>,
    call_events: tokio::task::JoinHandle<()>,
}

impl Drop for Entry {
    fn drop(&mut self) { self.call_events.abort(); }
}

pub(crate) struct Accounts {
    pub store: Mutex<AccountStore>,
    pub config: LoginConfig,
    port: u16,
    entries: RwLock<BTreeMap<String, Arc<Entry>>>,
    order: RwLock<Vec<String>>,
    tasks: Mutex<Vec<(String,tokio::task::JoinHandle<()>)>>,
    pub logins: std::sync::OnceLock<crate::login_http::LoginHttp>,
    journals: RwLock<Vec<(String, EventStore)>>,
    event_log: Mutex<crate::multi_feed::Journal>,
    feed_log: Arc<Mutex<crate::multi_feed::Feed>>,
    factory: ClientFactory,
    call_creation: tokio::sync::Mutex<()>,
}

pub(crate) type ClientFactory=Arc<dyn Fn(&std::path::Path)->Result<Arc<TdClient>,String>+Send+Sync>;

#[cfg(test)]
pub(crate) mod tests;

fn offline() -> GwError { GwError::new(503,"ACCOUNT_OFFLINE","Tài khoản đang mất kết nối tới Telegram.") }
fn storage_error() -> GwError { GwError::new(500,"ACCOUNT_STORAGE","Không ghi được sổ tài khoản.") }

impl Accounts {
    pub async fn new(config: LoginConfig, library: String, port: u16) -> Result<Arc<Self>, String> {
        let lib=library.clone();
        Self::with_factory(config,library,port,Arc::new(move |p|TdClient::open(&lib,p))).await
    }

    pub(crate) async fn with_factory(config:LoginConfig,_library:String,port:u16,factory:ClientFactory)->Result<Arc<Self>,String> {
        let store = AccountStore::open(&config.data_dir, &config.phone).map_err(|_| "Không đọc được sổ tài khoản.")?;
        let event_log = crate::multi_feed::Journal::open(&config.data_dir).map_err(|_| "Không mở được sổ sự kiện tài khoản.")?;
        let hub = Arc::new(Self { store: Mutex::new(store), config, port,
            entries: RwLock::new(BTreeMap::new()), order: RwLock::new(Vec::new()), tasks: Mutex::new(Vec::new()),
            logins: std::sync::OnceLock::new(), journals: RwLock::new(Vec::new()), event_log: Mutex::new(event_log),
            feed_log: Arc::new(Mutex::new(crate::multi_feed::Feed::default())),factory,call_creation:tokio::sync::Mutex::new(()) });
        let _ = hub.logins.set(crate::login_http::LoginHttp::new(Arc::downgrade(&hub)));
        let sessions: Vec<_> = {
            let store = hub.store.lock().unwrap();
            if store.legacy {
                vec![(hub.config.phone.clone(), hub.config.session_dir(), hub.config.data_dir.clone(), false)]
            } else {
                store.rows().iter().filter(|a| a.status == "online").map(|a| {
                    let session = store.session_dir(&a.phone);
                    let services=if session==hub.config.session_dir(){hub.config.data_dir.clone()}else{session.clone()};
                    (a.phone.clone(), session, services, true)
                }).collect()
            }
        };
        for (phone, session, service_dir, restore) in sessions {
            let client = match hub.open_client(&session) {
                Ok(c) => c,
                Err(_) => { eprintln!("Không mở được một phiên tài khoản; HTTP tiếp tục hoạt động."); continue; }
            };
            let entry = match hub.attach(&phone, client.clone(), service_dir, false).await {
                Ok(e) => e,
                Err(_) => { client.close(Duration::from_secs(10)).await; continue; }
            };
            let accounts = hub.clone();
            let mut cfg = accounts.config.clone();
            cfg.phone = phone.clone();
            let task_phone=phone.clone();
            let task = tokio::spawn(async move {
                let result = td::login::run_in(&client, &cfg, &session, restore).await;
                if let Some(events)=entry.events.lock().await.as_ref() {
                    events.sink.account_login(result.is_ok());
                }
                if result.is_ok() { entry.ready.store(true, Ordering::Release); }
                else {
                    client.close(Duration::from_secs(10)).await;
                    let _=entry.calls.shutdown(Duration::from_secs(10)).await;
                    accounts.entries.write().unwrap().remove(&phone);
                    accounts.order.write().unwrap().retain(|p|p!=&phone);
                    if let Some(events)=entry.events.lock().await.take() {events.shutdown().await;}
                    accounts.journals.write().unwrap().retain(|(p,_)|p!=&phone);
                    eprintln!("Đăng nhập nền thất bại; HTTP tiếp tục hoạt động.");
                }
            });
            hub.tasks.lock().unwrap().push((task_phone,task));
        }
        Ok(hub)
    }

    async fn attach(&self, phone: &str, client: Arc<TdClient>, service_dir: PathBuf, ready: bool) -> Result<Arc<Entry>, GwError> {
        let events = EventBridge::start(&service_dir, phone, Webhook::from_env()).await.map_err(|_| storage_error())?;
        let td: Arc<dyn TdLink> = client.clone();
        let calls = CallManager::with_cast(td.clone(), &service_dir, Some(self.port))?;
        let sink=events.sink.clone();
        let source=calls.events();
        let mut feed_source=source.resubscribe();
        let feed=self.feed_log.clone();
        let account=phone.to_owned();
        let call_events = tokio::spawn(async move {
            tokio::join!(sink.follow_calls(source),async move {
                loop {
                    match feed_source.recv().await {
                        Ok(event)=>feed.lock().unwrap().push(&account,json!({"type":"call","event":event["kind"],"call":event["call"]})),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>continue,
                        Err(_)=>break,
                    }
                }
            });
        });
        let entry = Arc::new(Entry { client, chat: Arc::new(ChatService::with_events(td, phone, service_dir, Some(events.sink.clone())).follow_calls(calls.events())),
            calls, ready: AtomicBool::new(ready), events: tokio::sync::Mutex::new(Some(events)), call_events });
        let event_store = entry.events.lock().await.as_ref().unwrap().store.clone();
        let mut journals=self.journals.write().unwrap();
        journals.retain(|(p,_)|p!=phone);
        journals.push((phone.into(), event_store));
        self.entries.write().unwrap().insert(phone.into(), entry.clone());
        self.order.write().unwrap().push(phone.into());
        Ok(entry)
    }

    pub fn contains(&self, phone: &str) -> bool { self.entries.read().unwrap().contains_key(phone) }

    pub async fn finish(&self, phone: &str, client: Arc<TdClient>) -> Result<(), GwError> {
        let me = client.request(json!({"@type":"getMe"})).await.map_err(|_| GwError::bad_request("Không xác minh được tài khoản Telegram."))?;
        let name = [me["first_name"].as_str().unwrap_or_default(),me["last_name"].as_str().unwrap_or_default()]
            .into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        let dir = self.store.lock().unwrap().session_dir(phone);
        {
            let mut store=self.store.lock().unwrap();
            store.keep_legacy(&self.config.phone).map_err(|_|storage_error())?;
            store.put_online(phone, name).map_err(|_| storage_error())?;
        }
        self.attach(phone, client, dir, true).await?;
        Ok(())
    }

    pub fn open_client(&self,session:&std::path::Path)->Result<Arc<TdClient>,String> { (self.factory)(session) }

    pub async fn remove(&self,phone:&str)->Result<(),GwError> {
        if phone!=self.config.phone || !self.store.lock().unwrap().legacy {
            if !self.store.lock().unwrap().rows().iter().any(|a|a.phone==phone) {
                return Err(GwError::not_found("Không rõ tài khoản."));
            }
        }
        self.tasks.lock().unwrap().retain(|(p,task)| {if p==phone {task.abort();false}else{true}});
        if self.store.lock().unwrap().legacy {
            self.store.lock().unwrap().keep_legacy(&self.config.phone).map_err(|_|storage_error())?;
        }
        let entry=self.entries.read().unwrap().get(phone).cloned();
        if let Some(e)=entry {
            e.ready.store(false,Ordering::Release);
            let _=e.calls.shutdown(Duration::from_secs(10)).await;
            e.client.close(Duration::from_secs(10)).await;
            if e.client.auth_state()!="authorizationStateClosed" {
                return Err(GwError::new(503,"ACCOUNT_CLOSING","TDLib chưa xác nhận đóng phiên; chưa di chuyển dữ liệu."));
            }
            if let Some(events)=e.events.lock().await.take() {events.shutdown().await;}
        }
        let journals=self.journals.read().unwrap().clone();
        for (p,source) in journals.into_iter().filter(|(p,_)|p==phone) {
            let since=self.event_log.lock().unwrap().source_cursor(&p);
            let result=source.poll(since).await?;
            self.event_log.lock().unwrap().ingest(&p,&result).map_err(|_|storage_error())?;
        }
        self.store.lock().unwrap().archive(phone).map_err(|e|if e.kind()==std::io::ErrorKind::NotFound {
            GwError::not_found("Không rõ tài khoản.") }else{storage_error()})?;
        self.journals.write().unwrap().retain(|(p,_)|p!=phone);
        self.event_log.lock().unwrap().forget_source(phone).map_err(|_|storage_error())?;
        self.feed_log.lock().unwrap().forget_source(phone);
        self.entries.write().unwrap().remove(phone);
        self.order.write().unwrap().retain(|p|p!=phone);
        Ok(())
    }

    fn all(&self) -> Vec<(String, Arc<Entry>)> {
        let entries = self.entries.read().unwrap();
        self.order.read().unwrap().iter().filter_map(|p| entries.get(p).map(|e| (p.clone(),e.clone()))).collect()
    }

    fn select(&self, phone: &str) -> Result<(String, Arc<Entry>), GwError> {
        let all = self.all();
        let picked = if phone.is_empty() {
            all.iter().find(|(_,e)| e.ready.load(Ordering::Acquire) && e.client.auth_state() == "authorizationStateReady")
                .or_else(|| all.first()).cloned()
        } else { all.into_iter().find(|(p,_)| p == phone) };
        let (p,e) = picked.ok_or_else(|| if phone.is_empty() || self.store.lock().unwrap().rows().iter().any(|a|a.phone==phone) {
            offline()
        } else { GwError::not_found("Không rõ tài khoản.") })?;
        if !e.ready.load(Ordering::Acquire) || e.client.auth_state() != "authorizationStateReady" { return Err(offline()); }
        Ok((p,e))
    }

    async fn select_call(&self,body:&Value)->Result<(String,Arc<Entry>),GwError> {
        let selected=body["account"].as_str().unwrap_or_default();
        if selected.is_empty() {
            if let Some(id)=body["id"].as_str() {
                for (phone,e) in self.all() {
                    if e.calls.owns_call(id).await {
                        return self.select(&phone);
                    }
                }
                return Err(GwError::not_found("Không có cuộc gọi này."));
            }
        }
        self.select(selected)
    }

    pub async fn shutdown(&self) {
        for (_,t) in self.tasks.lock().unwrap().drain(..) { t.abort(); }
        self.logins.get().unwrap().shutdown().await;
        for (_,e) in self.all() {
            let _ = e.calls.shutdown(Duration::from_secs(10)).await;
            e.client.close(Duration::from_secs(10)).await;
            if let Some(events) = e.events.lock().await.take() { events.shutdown().await; }
        }
    }

    async fn chat(&self, method: &str, path: &str, query: &str, body: Value) -> Result<Value,GwError> {
        if path.starts_with("/ui/api/") { return self.logins.get().unwrap().handle(method,path,body).await; }
        if path == "/api/accounts" {
            let (mut rows, online) = {
                let store = self.store.lock().unwrap();
                let entries = self.entries.read().unwrap();
                let mut rows: Vec<_> = store.rows().iter().map(|a| {
                    let online = entries.get(&a.phone).is_some_and(|e| e.ready.load(Ordering::Acquire) && e.client.auth_state()=="authorizationStateReady");
                    json!({"phone":a.phone,"display_name":a.display_name,"status":if online {"online"} else {"offline"}})
                }).collect();
                if store.legacy && rows.is_empty() {
                    rows.push(json!({"phone":self.config.phone,"display_name":"",
                        "status":if entries.get(&self.config.phone).is_some_and(|e| e.ready.load(Ordering::Acquire) && e.client.auth_state()=="authorizationStateReady") {"online"} else {"offline"}}));
                }
                let online: Vec<_> = rows.iter().enumerate().filter(|(_,a)|a["status"]=="online")
                    .map(|(i,a)|(i,entries[a["phone"].as_str().unwrap()].clone())).collect();
                (rows, online)
            };
            for (index, entry) in online {
                if let Ok(name) = entry.chat.account_name().await {
                    if !name.is_empty() {
                        let phone = rows[index]["phone"].as_str().unwrap();
                        if self.store.lock().unwrap().refresh_name(phone,&name).is_err() {
                            log::warn!("Không cập nhật được tên trong sổ tài khoản.");
                        }
                        rows[index]["display_name"] = json!(name);
                    }
                }
            }
            return Ok(json!({"tai_khoan":rows}));
        }
        if path == "/api/events" {
            let sources = self.journals.read().unwrap().clone();
            for (phone, source) in sources {
                let since = self.event_log.lock().unwrap().source_cursor(&phone);
                let result = source.poll(since).await?;
                self.event_log.lock().unwrap().ingest(&phone, &result).map_err(|_| storage_error())?;
            }
            return Ok(self.event_log.lock().unwrap().poll(number(query,"since")?));
        }
        if path == "/api/feed" {
            let since = number(query,"since")?;
            let wait: f64 = param(query,"wait").filter(|s|!s.is_empty()).unwrap_or_else(||"0".into()).trim().parse()
                .map_err(|_|GwError::bad_request("Tham số wait phải là số."))?;
            let deadline = tokio::time::Instant::now()+Duration::from_secs_f64(if wait.is_nan(){25.0}else{wait.clamp(0.0,25.0)});
            loop {
                for (phone,e) in self.all() {
                    let cursor = self.feed_log.lock().unwrap().source_cursor(&phone);
                    let result = e.chat.handle("GET","/api/feed",&format!("since={cursor}"),Value::Null).await?;
                    self.feed_log.lock().unwrap().ingest(&phone,&result);
                }
                let result = self.feed_log.lock().unwrap().poll(since);
                if !result["items"].as_array().unwrap().is_empty() || tokio::time::Instant::now() >= deadline { return Ok(result); }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        if path == "/api/live/calls" {
            if let Some(phone) = param(query,"account").filter(|s| !s.is_empty()) {
                return self.select(&phone)?.1.calls.calls().await;
            }
            return self.calls().await;
        }
        let selected = param(query,"account").filter(|s| !s.is_empty()).unwrap_or_else(||body["account"].as_str().unwrap_or_default().into());
        let (phone,e) = self.select(&selected)?;
        if path == "/api/live/settings" { return e.calls.settings(Value::Null).await; }
        if path == "/api/live/devices" { return e.calls.devices(Value::Null).await; }
        let _=phone;
        e.chat.handle(method,path,query,body).await
    }
}

pub(crate) fn param(query: &str, name: &str) -> Option<String> {
    form_urlencoded::parse(query.as_bytes()).filter(|(k,_)| k==name).last().map(|(_,v)|v.into_owned())
}
fn number(query: &str, name: &str) -> Result<i64,GwError> {
    param(query,name).filter(|s|!s.is_empty()).map_or(Ok(0),|s|s.trim().parse().map_err(|_|GwError::bad_request(format!("Tham số {name} phải là số."))))
}

impl ChatControl for Accounts {
    fn handle(&self, method:&str,path:&str,query:&str,body:Value)->BoxFut<'_,Result<Value,GwError>> {
        let (m,p,q)=(method.to_owned(),path.to_owned(),query.to_owned());
        Box::pin(async move { self.chat(&m,&p,&q,body).await })
    }
    fn bytes(&self,path:&str,query:&str)->BoxFut<'_,Result<Binary,GwError>> {
        let (p,q)=(path.to_owned(),query.to_owned());
        Box::pin(async move {
            if p.starts_with("/api/live/snapshot/") {
                let id=p.rsplit('/').next().unwrap();
                return match param(&q,"account").filter(|s|!s.is_empty()) {
                    Some(phone)=>self.select(&phone)?.1.calls.snapshot(id).await,
                    None=>self.snapshot(id).await,
                };
            }
            let (phone,e)=self.select(&param(&q,"account").unwrap_or_default())?;
            if let Some(id)=p.strip_prefix("/api/file/") {
                let session={
                    let store=self.store.lock().unwrap();
                    if store.legacy && phone==self.config.phone {self.config.session_dir()}else{store.session_dir(&phone)}
                };
                if session==self.config.session_dir() {return e.chat.bytes(&p,&q).await;}
                let id=id.parse().map_err(|_|GwError::bad_request("file_id phải là số."))?;
                return crate::account_files::download(e.client.clone(),session,id).await;
            }
            e.chat.bytes(&p,&q).await
        })
    }
}

macro_rules! route_call {
    ($name:ident) => {
        fn $name(&self,body:Value)->BoxFut<'_,Result<Value,GwError>> {
            Box::pin(async move {
                let (phone,e)=self.select_call(&body).await?;
                let _=phone;
                e.calls.$name(body).await
            })
        }
    }
}
impl CallControl for Accounts {
    fn calls(&self)->BoxFut<'_,Result<Value,GwError>> {
        Box::pin(async move {
            let mut calls=Vec::new();
            let all=self.all();
            if !all.iter().any(|(_,e)|e.ready.load(Ordering::Acquire) && e.client.auth_state()=="authorizationStateReady") { return Err(offline()); }
            for (_,e) in all {
                let result=e.calls.calls().await?;
                calls.extend(result["calls"].as_array().cloned().unwrap_or_default());
            }
            Ok(json!({"calls":calls}))
        })
    }
    fn call_out(&self,mut body:Value)->BoxFut<'_,Result<Value,GwError>> {
        Box::pin(async move {
            let _guard=self.call_creation.lock().await;
            let (phone,e)=self.select(body["account"].as_str().unwrap_or_default())?;
            let user=e.calls.resolve(body.clone()).await?;
            for (_,other) in self.all() {
                let calls=other.calls.calls().await?;
                if calls["calls"].as_array().is_some_and(|calls|calls.iter().any(|c|c["user_id"]==user && c["state"]!="ended")) {
                    return Err(GwError::new(409,"CALL_BUSY","đang có cuộc gọi với người này"));
                }
            }
            body.as_object_mut().ok_or_else(||GwError::bad_request("Cần JSON object"))?.remove("peer");
            body["user_id"]=user;
            let _=phone;
            e.calls.call_out(body).await
        })
    }
    route_call!(accept); route_call!(hangup); route_call!(settings);
    route_call!(media); route_call!(playback); route_call!(say);
    fn devices(&self, mut body:Value)->BoxFut<'_,Result<Value,GwError>> {
        Box::pin(async move {
            let (_,e)=self.select(body["account"].as_str().unwrap_or_default())?;
            if let Some(map)=body.as_object_mut() { map.remove("account"); }
            e.calls.devices(body).await
        })
    }
    fn audio_in(&self,id:&str,pcm:Vec<u8>)->BoxFut<'_,Result<Value,GwError>> {
        let id=id.to_owned(); Box::pin(async move {
            for (_,e) in self.all() {
                if e.calls.owns_call(&id).await { return e.calls.audio_in(&id,pcm).await; }
            }
            Err(GwError::not_found("Không có cuộc gọi này."))
        })
    }
    fn audio_out(&self,id:&str,token:&str)->BoxFut<'_,Result<AudioOut,GwError>> {
        let (id,token)=(id.to_owned(),token.to_owned()); Box::pin(async move {
            for (_,e) in self.all() {
                if e.calls.owns_call(&id).await { return e.calls.audio_out(&id,&token).await; }
            }
            Err(GwError::not_found("Không có cuộc gọi này."))
        })
    }
    fn snapshot(&self,id:&str)->BoxFut<'_,Result<Binary,GwError>> {
        let id=id.to_owned(); Box::pin(async move {
            for (_,e) in self.all() {
                if e.calls.owns_call(&id).await {
                    return e.calls.snapshot(&id).await;
                }
            }
            Err(GwError::not_found("Không có cuộc gọi này."))
        })
    }
    fn video_frames(&self,id:&str)->BoxFut<'_,Result<crate::contract::VideoFrames,GwError>> {
        let id=id.to_owned(); Box::pin(async move {
            for (_,e) in self.all() {
                if e.calls.owns_call(&id).await {
                    return e.calls.video_frames(&id).await;
                }
            }
            Err(GwError::not_found("Không có cuộc gọi này."))
        })
    }
    fn cast_audio(&self,id:&str,token:&str)->BoxFut<'_,Option<CastAudio>> {
        let (id,token)=(id.to_owned(),token.to_owned()); Box::pin(async move {
            for (_,e) in self.all() { if let Some(stream)=e.calls.cast_audio(&id,&token).await { return Some(stream); } } None
        })
    }
}
