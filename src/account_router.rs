//! Tách dịch vụ, phiên và cập nhật theo tài khoản; không đổi các hợp đồng backend.
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, Mutex, RwLock, atomic::{AtomicBool, AtomicU64, Ordering}}, time::Duration};
use serde_json::{json, Value};
use crate::{accounts::AccountStore, calls::CallManager, chat::ChatService,
    contract::{AudioOut, Binary, BoxFut, CallControl, CastAudio, ChatControl, GwError, TdLink},
    events::{EventBridge, EventStore, Webhook}, td::{self, login::LoginConfig, TdClient}};

pub(crate) struct Entry {
    pub client: Arc<TdClient>,
    session: PathBuf,
    uploads: PathBuf,
    storage_lock: tokio::sync::RwLock<()>,
    sweep_lock: tokio::sync::Mutex<()>,
    settings_lock: tokio::sync::Mutex<()>,
    appearance_lock: tokio::sync::Mutex<()>,
    storage_generation: Arc<AtomicU64>,
    storage_report: Mutex<crate::storage::Report>,
    chat: Arc<ChatService>,
    calls: CallManager,
    ready: AtomicBool,
    events: tokio::sync::Mutex<Option<EventBridge>>,
    call_events: tokio::task::JoinHandle<()>,
    storage_task: std::sync::OnceLock<tokio::task::JoinHandle<()>>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.call_events.abort();
        if let Some(task) = self.storage_task.get() { task.abort(); }
    }
}

impl Entry {
    async fn storage_sweep(&self, scope: Option<crate::storage::Clear>) -> Result<Value, GwError> {
        if !self.ready.load(Ordering::Acquire) || self.client.auth_state() != "authorizationStateReady" {
            return Err(offline());
        }
        if scope.is_none() {
            match crate::storage::settings(&self.session).await {
                Ok(policy) => {
                    if crate::storage::policy_is_noop(&policy)? {
                        let mut report = crate::storage::Report::new();
                        report.finish();
                        let value = report.json();
                        *self.storage_report.lock().unwrap() = report;
                        return Ok(value);
                    }
                }
                Err(err) => {
                    let mut report = crate::storage::Report::new();
                    report.failed(&err.code);
                    report.finish();
                    *self.storage_report.lock().unwrap() = report;
                    return Err(err);
                }
            }
        }
        let _sweep = self.sweep_lock.lock().await;
        if !self.ready.load(Ordering::Acquire) || self.client.auth_state() != "authorizationStateReady" {
            return Err(offline());
        }
        let mut report = crate::storage::Report::new();
        if self.calls.has_active_call().await? {
            if scope.is_none() {
                report.failed("STORAGE_BUSY");
                report.finish();
                *self.storage_report.lock().unwrap() = report;
            }
            return Err(GwError::new(409, "STORAGE_BUSY", "Bộ nhớ đệm đang được bảo vệ trong lúc gọi."));
        }
        let result = async {
            crate::storage::check_cache_paths(&self.session).await?;
            if self.calls.has_active_call().await? {
                return Err(GwError::new(409, "STORAGE_BUSY", "Bộ nhớ đệm đang được bảo vệ trong lúc gọi."));
            }
            if let Some(scope) = scope {
                match scope {
                    crate::storage::Clear::Chat(_) => {
                        let cancelled = || !self.ready.load(Ordering::Acquire);
                        crate::storage::clear_with_policy(self.client.as_ref(), scope, &[], &mut report, &cancelled).await
                    }
                    scope @ (crate::storage::Clear::All | crate::storage::Clear::Types(_)) => {
                        let start_gen = self.storage_generation.load(Ordering::Acquire);
                        let policy = crate::storage::settings(&self.session).await?;
                        let forever = crate::storage::forever_exceptions(&policy)?;
                        let cancelled = || {
                            self.storage_generation.load(Ordering::Acquire) != start_gen
                                || !self.ready.load(Ordering::Acquire)
                        };
                        crate::storage::clear_with_policy(self.client.as_ref(), scope, &forever, &mut report, &cancelled).await?;
                        if report.policy_changed() {
                            return Err(GwError::new(
                                409,
                                "STORAGE_POLICY_CHANGED",
                                "Chính sách lưu trữ vừa thay đổi; hãy kiểm tra lại trước khi xoá.",
                            ));
                        }
                        Ok(())
                    }
                }
            } else {
                let start_gen = self.storage_generation.load(Ordering::Acquire);
                let policy = crate::storage::settings(&self.session).await?;
                if crate::storage::policy_is_noop(&policy)? {
                    return Ok(());
                }
                let cancelled = || {
                    self.storage_generation.load(Ordering::Acquire) != start_gen
                        || !self.ready.load(Ordering::Acquire)
                };
                crate::storage::run_cancellable(self.client.as_ref(), &policy, &mut report, &cancelled).await
            }
        }.await;
        if let Err(ref err) = result { report.failed(&err.code); }
        report.finish();
        let value = report.json();
        *self.storage_report.lock().unwrap() = report;
        result.map(|()| value)
    }

    fn stop_storage(&self) {
        self.ready.store(false, Ordering::Release);
        if let Some(task) = self.storage_task.get() { task.abort(); }
    }
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
            let entry = match hub.attach(&phone, client.clone(), session.clone(), service_dir, false).await {
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
                    entry.stop_storage();
                    let _ = entry.calls.shutdown(Duration::from_secs(10)).await;
                    client.close(Duration::from_secs(10)).await;
                    let events = entry.events.lock().await.take();
                    let event_store = events.as_ref().map(|ev| ev.store.clone());
                    if let Some(events) = events { events.shutdown().await; }
                    if let Some(source) = event_store {
                        let since = accounts.event_log.lock().unwrap().source_cursor(&phone);
                        if let Ok(polled) = source.poll(since).await {
                            let _ = accounts.event_log.lock().unwrap().ingest(&phone, &polled);
                        }
                        accounts.journals.write().unwrap().retain(|(p, s)| p != &phone || !s.same_source(&source));
                    }
                    {
                        let mut entries = accounts.entries.write().unwrap();
                        if entries.get(&phone).is_some_and(|active| Arc::ptr_eq(active, &entry)) {
                            entries.remove(&phone);
                            accounts.order.write().unwrap().retain(|p| p != &phone);
                        }
                    }
                    eprintln!("Đăng nhập nền thất bại; HTTP tiếp tục hoạt động.");
                }
            });
            hub.tasks.lock().unwrap().push((task_phone,task));
        }
        Ok(hub)
    }

    async fn attach(&self, phone: &str, client: Arc<TdClient>, session: PathBuf, service_dir: PathBuf, ready: bool) -> Result<Arc<Entry>, GwError> {
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
        let uploads = service_dir.join("uploads");
        let entry = Arc::new(Entry { client, session, uploads, chat: Arc::new(ChatService::with_events(td, phone, service_dir, Some(events.sink.clone())).follow_calls(calls.events())),
            calls, ready: AtomicBool::new(ready), events: tokio::sync::Mutex::new(Some(events)), call_events,
            storage_lock: tokio::sync::RwLock::new(()), sweep_lock: tokio::sync::Mutex::new(()),
            settings_lock: tokio::sync::Mutex::new(()), appearance_lock: tokio::sync::Mutex::new(()),
            storage_generation: Arc::new(AtomicU64::new(0)),
            storage_report: Mutex::new(crate::storage::Report::new()), storage_task: std::sync::OnceLock::new() });
        let task_entry = Arc::downgrade(&entry);
        let mut updates = entry.client.updates();
        let _ = entry.storage_task.set(tokio::spawn(async move {
            let period = Duration::from_secs(600);
            let mut clock = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let update = tokio::select! {
                    _ = clock.tick() => None,
                    update = updates.recv() => match update {
                        Ok(update) if update["@type"] == "updateNewMessage" => Some(update),
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        _ => continue,
                    }
                };
                let Some(entry) = task_entry.upgrade() else { break; };
                if let Some(update) = update {
                    let _access = entry.storage_lock.read().await;
                    if entry.ready.load(Ordering::Acquire) {
                        crate::storage::auto_download(entry.client.as_ref(), &entry.session, &update["message"]).await;
                    }
                } else {
                    let _ = entry.storage_sweep(None).await;
                }
            }
        }));
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
            let mut store = self.store.lock().unwrap();
            if phone != self.config.phone {
                store.keep_legacy(&self.config.phone).map_err(|_| storage_error())?;
            }
            store.put_online(phone, name).map_err(|_| storage_error())?;
        }
        let service_dir = if dir == self.config.session_dir() {
            self.config.data_dir.clone()
        } else {
            dir.clone()
        };
        self.attach(phone, client, dir, service_dir, true).await?;
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
            e.stop_storage();
            let _sweep = e.sweep_lock.lock().await;
            let _storage = e.storage_lock.write().await;
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
        {
            let mut journals = self.journals.write().unwrap();
            journals.retain(|(p, _)| p != phone);
            self.event_log.lock().unwrap().forget_source(phone).map_err(|_| storage_error())?;
        }
        {
            let mut entries = self.entries.write().unwrap();
            entries.remove(phone);
            self.order.write().unwrap().retain(|p| p != phone);
            self.feed_log.lock().unwrap().forget_source(phone);
        }
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
        let (p, e) = picked.ok_or_else(|| {
            let known = phone.is_empty() || {
                let store = self.store.lock().unwrap();
                (store.legacy && phone == self.config.phone) || store.rows().iter().any(|a| a.phone == phone)
            };
            if known { offline() } else { GwError::not_found("Không rõ tài khoản.") }
        })?;
        if !e.ready.load(Ordering::Acquire) || e.client.auth_state() != "authorizationStateReady" { return Err(offline()); }
        Ok((p,e))
    }

    fn select_storage(&self, phone: &str) -> Result<(String, Arc<Entry>), GwError> {
        if phone.is_empty() && (self.all().len() > 1 || self.store.lock().unwrap().rows().len() > 1) {
            return Err(GwError::bad_request("Có nhiều tài khoản; cần chỉ định tham số account."));
        }
        self.select(phone)
    }

    fn select_account_scoped(&self, phone: &str) -> Result<(String, Arc<Entry>), GwError> {
        if phone.is_empty() && (self.all().len() > 1 || self.store.lock().unwrap().rows().len() > 1) {
            return Err(GwError::bad_request("Có nhiều tài khoản; cần chỉ định tham số account."));
        }
        self.select(phone)
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
            e.stop_storage();
            let _sweep = e.sweep_lock.lock().await;
            let _storage = e.storage_lock.write().await;
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
                let since = {
                    let journals = self.journals.read().unwrap();
                    if !journals.iter().any(|(p, s)| p == &phone && s.same_source(&source)) {
                        continue;
                    }
                    self.event_log.lock().unwrap().source_cursor(&phone)
                };
                let result = source.poll(since).await?;
                let journals = self.journals.read().unwrap();
                if journals.iter().any(|(p, s)| p == &phone && s.same_source(&source)) {
                    self.event_log.lock().unwrap().ingest(&phone, &result).map_err(|_| storage_error())?;
                }
            }
            return Ok(self.event_log.lock().unwrap().poll(number(query,"since")?));
        }
        if path == "/api/feed" {
            let since = number(query,"since")?;
            let wait: f64 = param(query,"wait").filter(|s|!s.is_empty()).unwrap_or_else(||"0".into()).trim().parse()
                .map_err(|_|GwError::bad_request("Tham số wait phải là số."))?;
            let deadline = tokio::time::Instant::now()+Duration::from_secs_f64(if wait.is_nan(){25.0}else{wait.clamp(0.0,25.0)});
            loop {
                for (phone, e) in self.all() {
                    let cursor = {
                        let entries = self.entries.read().unwrap();
                        if !entries.get(&phone).is_some_and(|active| Arc::ptr_eq(active, &e)) {
                            continue;
                        }
                        self.feed_log.lock().unwrap().source_cursor(&phone)
                    };
                    let result = e.chat.handle("GET","/api/feed",&format!("since={cursor}"),Value::Null).await?;
                    let entries = self.entries.read().unwrap();
                    if entries.get(&phone).is_some_and(|active| Arc::ptr_eq(active, &e)) {
                        self.feed_log.lock().unwrap().ingest(&phone,&result);
                    }
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
        if path == "/api/storage/clear" {
            let scope = crate::storage::Clear::parse(&body)?;
            let entry = self.select_storage(&selected)?.1;
            return tokio::spawn(async move { entry.storage_sweep(Some(scope)).await })
                .await.map_err(|_| offline())?;
        }
        if matches!(path, "/api/storage/janitor" | "/api/storage/usage" | "/api/settings/storage") {
            let (_, e) = self.select_storage(&selected)?;
            if path == "/api/storage/janitor" {
                return Ok(e.storage_report.lock().unwrap().json());
            }
            if path == "/api/storage/usage" {
                let _access = e.storage_lock.read().await;
                return crate::storage::usage(e.client.as_ref()).await;
            }
            if method == "POST" {
                let (clean, confirmed) = crate::storage_settings::parse_update(body)?;
                let entry = e.clone();
                return tokio::spawn(async move {
                    let _access = entry.storage_lock.read().await;
                    let _write = entry.settings_lock.lock().await;
                    if !entry.ready.load(Ordering::Acquire) { return Err(offline()); }
                    let dir = entry.session.clone();
                    let generation = entry.storage_generation.clone();
                    tokio::task::spawn_blocking(move || {
                        let current = crate::storage_settings::load(&dir)
                            .map_err(|_| crate::storage::settings_error())?;
                        if crate::storage_settings::is_tightening(&current, &clean) && !confirmed {
                            let message = if current["mode"] == "local" && clean["mode"] == "stream" {
                                "Chuyển sang chế độ xem trực tuyến cần confirm=true vì tệp cục bộ sẽ bị dọn sau thời hạn."
                            } else {
                                "Thay đổi làm chặt chính sách lưu trữ cần confirm=true vì dữ liệu vượt giới hạn mới sẽ bị dọn trong lượt tới."
                            };
                            return Err(GwError::bad_request(message));
                        }
                        crate::storage_settings::save(&dir, &clean)
                            .map_err(|_| crate::storage::settings_error())?;
                        generation.fetch_add(1, Ordering::Release);
                        Ok(clean)
                    }).await.map_err(|_| crate::storage::settings_error())?
                }).await.map_err(|_| crate::storage::settings_error())?;
            }
            return Ok(crate::storage::settings_read_only(&e.session).await);
        }
        if path == "/api/settings/appearance" {
            let (_, e) = self.select_account_scoped(&selected)?;
            if method == "POST" {
                crate::appearance_settings::validate(&body)?;
                let entry = e.clone();
                return tokio::spawn(async move {
                    let _access = entry.storage_lock.read().await;
                    let _write = entry.appearance_lock.lock().await;
                    if !entry.ready.load(Ordering::Acquire) { return Err(offline()); }
                    let dir = entry.session.clone();
                    let clean = body;
                    tokio::task::spawn_blocking(move || {
                        crate::appearance_settings::save(&dir, &clean)
                            .map_err(|_| crate::appearance_settings::io_error())?;
                        Ok(clean)
                    }).await.map_err(|_| crate::appearance_settings::io_error())?
                }).await.map_err(|_| crate::appearance_settings::io_error())?;
            }
            let dir = e.session.clone();
            return tokio::task::spawn_blocking(move || crate::appearance_settings::load_or_default(&dir))
                .await.map_err(|_| crate::appearance_settings::io_error());
        }
        if path == "/api/profile" {
            let (phone, e) = self.select_account_scoped(&selected)?;
            let _access = e.storage_lock.read().await;
            let result = e.chat.handle(method, path, query, body).await?;
            if method == "POST" {
                let full_name = format!(
                    "{} {}",
                    result["first_name"].as_str().unwrap_or_default(),
                    result["last_name"].as_str().unwrap_or_default(),
                ).trim().to_owned();
                if !full_name.is_empty() && self.store.lock().unwrap().refresh_name(&phone, &full_name).is_err() {
                    log::warn!("Không cập nhật được tên hồ sơ trong sổ tài khoản.");
                }
            }
            return Ok(result);
        }
        if is_chat_info_route(path) {
            let mut clean_body = body;
            let q_acc = param(query, "account").filter(|s| !s.is_empty());
            if let Some(map) = clean_body.as_object_mut() {
                if let Some(val) = map.remove("account") {
                    if !val.is_null() {
                        let Some(b_acc) = val.as_str() else {
                            return Err(GwError::bad_request("Tham số account phải là chuỗi."));
                        };
                        if let (Some(q), b) = (q_acc.as_deref(), b_acc) {
                            if !b.is_empty() && q != b {
                                return Err(GwError::bad_request("Tham số account trong query và thân JSON không khớp."));
                            }
                        }
                    }
                }
            }
            let (_, e) = self.select_account_scoped(&selected)?;
            let _access = e.storage_lock.read().await;
            return e.chat.handle(method, path, query, clean_body).await;
        }
        let (phone,e) = self.select(&selected)?;
        if path == "/api/live/settings" { return e.calls.settings(Value::Null).await; }
        if path == "/api/live/devices" { return e.calls.devices(Value::Null).await; }
        let _=phone;
        let _access = e.storage_lock.read().await;
        e.chat.handle(method,path,query,body).await
    }
}

fn is_chat_info_route(path: &str) -> bool {
    let mut parts = path.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(""), Some("api"), Some("chats"), Some(_), Some("info" | "settings" | "clear_history" | "delete" | "leave" | "members"), None)
    )
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
            let (_,e)=self.select(&param(&q,"account").unwrap_or_default())?;
            if let Some(id)=p.strip_prefix("/api/file/") {
                let id=id.parse().map_err(|_|GwError::bad_request("file_id phải là số."))?;
                let entry = e.clone();
                let uploads = entry.uploads.clone();
                return tokio::spawn(async move {
                    let _access = entry.storage_lock.read().await;
                    if !entry.ready.load(Ordering::Acquire) { return Err(offline()); }
                    let stream = crate::storage::settings_read_only(&entry.session).await["mode"] == "stream";
                    crate::account_files::download(entry.client.clone(), entry.session.clone(), uploads, id, stream).await
                }).await.map_err(|_| offline())?;
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
                let _access = e.storage_lock.read().await;
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
            let _access = e.storage_lock.read().await;
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
    fn video_in(&self,id:&str,jpeg:Vec<u8>)->BoxFut<'_,Result<Value,GwError>> {
        let id=id.to_owned(); Box::pin(async move {
            for (_,e) in self.all() {
                if e.calls.owns_call(&id).await { return e.calls.video_in(&id,jpeg).await; }
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
