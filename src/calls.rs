//! Cuộc gọi P2P: một CallManager cho một TdLink/tài khoản.
//!
//! `new` tự đăng ký updates(); tầng REST giữ một bản clone làm `dyn CallControl`.
//! `events` phát incoming/active/ended/peer_video. NTgCalls nằm trong một thread
//! có Tokio runtime riêng, vì binding 3.0.7 không Sync. Không giữ khóa qua await.
mod media;
mod sources;
mod snapshot;
pub mod versions;

pub use sources::{camera_command, is_cast_spec, mic_command, speaker_command, valid_speaker_url};

use std::{collections::{HashSet, VecDeque}, fs::{self, OpenOptions}, io::Write,
    os::unix::fs::OpenOptionsExt, path::{Path, PathBuf}, sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

use base64::{engine::general_purpose::STANDARD, Engine};
use ntgcalls::{ConnectionState, NTgCalls, Protocol, StreamDevice, StreamMode};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::contract::{Binary, BoxFut, CallControl, GwError, TdLink};
use media::{Media, MediaEvent, MediaOp, NativeMedia, Ready};
use sources::{Devices, playback_description, video_profile};

type Reply = Result<Value, GwError>;

#[derive(Clone, Copy)]
enum Action { Calls, CallOut, Accept, Hangup, Settings, Media, Playback, Say }

enum Command {
    Api(Action, Value, oneshot::Sender<Reply>),
    Update(Value, oneshot::Sender<Reply>),
    Media(MediaEvent),
    Snapshot(String, oneshot::Sender<Result<Binary, GwError>>),
    VideoJpeg(String, Option<Vec<u8>>),
    RestoreMic(String, u64, String, String, PathBuf),
    RefreshCapture(String),
    Shutdown(Option<std::sync::mpsc::Sender<()>>, Instant),
}

struct Handle {
    sender: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<Value>,
    shutting_down: AtomicBool,
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.shutting_down.swap(true, Ordering::AcqRel) {
            let _ = self.sender.send(Command::Shutdown(None, Instant::now() + Duration::from_secs(5)));
        }
    }
}

#[derive(Clone)]
pub struct CallManager { handle: Arc<Handle> }

impl CallManager {
    /// Dùng cùng TdLink mà tầng TDLib đã đăng nhập; không tự mở client TDLib.
    pub fn new(td: Arc<dyn TdLink>, data_dir: impl AsRef<Path>) -> Result<Self, GwError> {
        Self::spawn(td, data_dir.as_ref().to_path_buf(), Box::new(NativeMedia(NTgCalls::new())), p2p_enabled(std::env::var("TELEGRAM_P2P").ok().as_deref()))
    }

    fn spawn(td: Arc<dyn TdLink>, data_dir: PathBuf, mut media: Box<dyn Media>, p2p: bool) -> Result<Self, GwError> {
        let updates = td.updates(); // Đăng ký trước khi bất kỳ createCall nào có thể gửi đi.
        let protocol = media.protocol().map_err(|_| GwError::new(502, "MEDIA_FAILED", "Không đọc được protocol NTgCalls"))?;
        let (sender, receiver) = mpsc::unbounded_channel();
        let (frame_sender, frame_receiver) = mpsc::channel(3);
        let (events, _) = broadcast::channel(256);
        let handle = Arc::new(Handle { sender: sender.clone(), events: events.clone(), shutting_down: AtomicBool::new(false) });
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("tggw-calls".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r) => r,
                Err(_) => { let _ = startup_tx.send(Err(GwError::new(500, "CALL_RUNTIME", "Không tạo được runtime cuộc gọi"))); return; }
            };
            runtime.block_on(async move {
                let mut defaults = Devices::default();
                match fs::read(data_dir.join("live_settings.json")) {
                    Ok(raw) => {
                        if let Ok(saved) = serde_json::from_slice::<Value>(&raw) { let _ = defaults.update(&saved); }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(_) => { let _ = startup_tx.send(Err(io_error())); return; }
                }
                media.callbacks(sender.clone(), frame_sender);
                let mut actor = Actor { td, media, protocol, p2p, data_dir, defaults, calls: Vec::new(),
                    handled: HashSet::new(), events, sender, profile: video_profile(&std::env::var("VIDEO_PROFILE").unwrap_or_default()) };
                let _ = startup_tx.send(Ok(()));
                actor.run(receiver, updates, frame_receiver).await;
            });
        }).map_err(|_| GwError::new(500, "CALL_RUNTIME", "Không tạo được luồng cuộc gọi"))?;
        startup_rx.recv().map_err(|_| runtime_error())??;
        Ok(Self { handle })
    }

    pub fn events(&self) -> broadcast::Receiver<Value> { self.handle.events.subscribe() }

    /// Ngừng nhận lệnh, kết thúc các cuộc gọi hiện tại rồi chờ actor dọn media.
    pub async fn shutdown(&self, wait: Duration) -> Result<(), String> {
        if self.handle.shutting_down.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Ok(());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.handle.sender.send(Command::Shutdown(Some(tx), Instant::now() + wait)).map_err(|_| "runtime cuộc gọi đã dừng".to_owned())?;
        tokio::task::spawn_blocking(move || rx.recv_timeout(wait))
            .await
            .map_err(|_| "không chờ được runtime cuộc gọi dừng".to_owned())?
            .map_err(|_| "quá thời gian chờ runtime cuộc gọi dừng".to_owned())
    }

    /// Lối vào có chờ xử lý, tiện cho adapter TDLib và kiểm thử; new() đã tự nghe updates().
    pub async fn handle_update(&self, update: Value) -> Reply {
        if self.handle.shutting_down.load(Ordering::Acquire) { return Err(runtime_error()); }
        let (tx, rx) = oneshot::channel();
        self.handle.sender.send(Command::Update(update, tx)).map_err(|_| runtime_error())?;
        rx.await.map_err(|_| runtime_error())?
    }

    async fn request(&self, action: Action, body: Value) -> Reply {
        if self.handle.shutting_down.load(Ordering::Acquire) { return Err(runtime_error()); }
        let (tx, rx) = oneshot::channel();
        self.handle.sender.send(Command::Api(action, body, tx)).map_err(|_| runtime_error())?;
        rx.await.map_err(|_| runtime_error())?
    }
}

impl CallControl for CallManager {
    fn calls(&self) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Calls, Value::Null)) }
    fn call_out(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::CallOut, body)) }
    fn accept(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Accept, body)) }
    fn hangup(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Hangup, body)) }
    /// Null = GET; object = POST, cùng hình dạng JSON của Python.
    fn settings(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Settings, body)) }
    fn media(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Media, body)) }
    fn playback(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Playback, body)) }
    fn say(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Say, body)) }
    fn snapshot(&self, id: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        let id = id.to_owned();
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            self.handle.sender.send(Command::Snapshot(id, tx)).map_err(|_| runtime_error())?;
            rx.await.map_err(|_| runtime_error())?
        })
    }
}

fn runtime_error() -> GwError { GwError::new(503, "CALL_RUNTIME", "Runtime cuộc gọi đã dừng") }
fn io_error() -> GwError { GwError::new(500, "CALL_STORAGE", "Không đọc/ghi được dữ liệu cuộc gọi") }
fn missing() -> GwError { GwError::not_found("Không có cuộc gọi này hoặc cuộc gọi đã được xử lý.") }
fn media_error() -> GwError { GwError::new(502, "MEDIA_FAILED", "Thao tác NTgCalls thất bại") }

pub fn p2p_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

struct LiveCall {
    id: String,
    td_id: Option<i64>,
    user_id: i64,
    outgoing: bool,
    video: bool,
    state: &'static str,
    accepted: bool,
    ready_done: bool,
    media_created: bool,
    pending_signaling: Vec<Vec<u8>>,
    devices: Devices,
    playback_kind: String,
    playback_cmd: String,
    playback_path: PathBuf,
    playback_final: Option<u64>,
    cam_off: bool,
    view_on: bool,
    peer_video: bool,
    view: Value,
    snapshot: snapshot::Snapshot,
    sig: [u64; 4],
    frames_in: u64,
    frame_bytes: u64,
    peak: i32,
    error: String,
    emojis: Value,
    started: Instant,
    active: Option<Instant>,
    ended: Option<Instant>,
    events: VecDeque<Value>,
    capture_revision: u64,
    speech_revision: Option<u64>,
}

impl LiveCall {
    fn new(user_id: i64, outgoing: bool, video: bool, devices: Devices, data_dir: &Path) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let id = format!("{:x}{:x}{:x}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(), SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let playback_kind = if devices.speaker.is_empty() { devices.playback.clone() } else { "push".into() };
        let playback_cmd = if playback_kind == "push" { devices.speaker.clone() } else { String::new() };
        let playback_path = data_dir.join("live").join(format!("{id}_in.pcm"));
        let mut c = Self { id, td_id: None, user_id, outgoing, video, state: if outgoing { "dialing" } else { "ringing" },
            accepted: false, ready_done: false, media_created: false, pending_signaling: Vec::new(), devices, playback_kind, playback_cmd,
            playback_path, playback_final: None, cam_off: false, view_on: video, peer_video: false,
            view: json!({"frames":0,"jpeg":0,"size":"","dropped":0,"bad":0}), snapshot: Default::default(), sig: [0;4], frames_in: 0, frame_bytes: 0,
            peak: 0, error: String::new(), emojis: json!([]), started: Instant::now(), active: None, ended: None, events: VecDeque::new(), capture_revision: 0, speech_revision: None };
        c.log(if outgoing { "tạo cuộc gọi đi" } else { "tạo cuộc gọi đến" });
        c
    }

    fn log(&mut self, text: &str) {
        // Không ghi nguồn, URL hoặc command ra nhật ký cuộc gọi.
        self.events.push_back(json!({"t":(self.started.elapsed().as_secs_f64()*100.0).round()/100.0,"e":sources::mask(text)}));
        while self.events.len() > 200 { self.events.pop_front(); }
    }

    fn playback_size(&self) -> u64 {
        self.playback_final.unwrap_or_else(|| fs::metadata(&self.playback_path).map(|m| m.len()).unwrap_or(0))
    }

    fn info(&self) -> Value {
        let duration = self.ended.unwrap_or_else(Instant::now).duration_since(self.active.unwrap_or(self.started)).as_secs_f64();
        json!({"id":self.id,"user_id":self.user_id,"outgoing":self.outgoing,"video":self.video,
            "state":self.state,"emojis":self.emojis,"error":self.error,"seconds":(duration*10.0).round()/10.0,
            "peer_video":self.peer_video,"view":self.view,"diag":{},"sig":{"tx_n":self.sig[0],"tx_b":self.sig[1],"rx_n":self.sig[2],"rx_b":self.sig[3]},
            "cpu_max":0.0,"mic_muted":self.devices.mic=="silence","cam_off":self.cam_off,"frames_in":self.frames_in,"frame_bytes":self.frame_bytes,"peak":self.peak,
            "playback":{"kind":self.playback_kind,"bytes":self.playback_size()},"events":self.events.iter().skip(self.events.len().saturating_sub(40)).collect::<Vec<_>>()})
    }

    fn playback(&self) -> Result<ntgcalls::MediaDescription, GwError> {
        playback_description(&self.playback_kind, &self.playback_cmd, &self.playback_path.to_string_lossy(), self.view_on)
    }
}

struct Actor {
    td: Arc<dyn TdLink>,
    media: Box<dyn Media>,
    protocol: Protocol,
    p2p: bool,
    data_dir: PathBuf,
    defaults: Devices,
    calls: Vec<LiveCall>,
    // Tombstone tách khỏi lịch sử hiển thị: prune không làm cuộc gọi cũ sống lại.
    handled: HashSet<i64>,
    events: broadcast::Sender<Value>,
    sender: mpsc::UnboundedSender<Command>,
    profile: (i16, i16, u8),
}

impl Actor {
    async fn run(&mut self, mut commands: mpsc::UnboundedReceiver<Command>, mut updates: broadcast::Receiver<Value>, mut frames: mpsc::Receiver<MediaEvent>) {
        let mut updates_open = true;
        let mut shutdown_ack = None;
        let mut shutdown_deadline = None;
        loop {
            tokio::select! {
                cmd = commands.recv() => match cmd {
                    Some(Command::Api(action, body, reply)) => {
                        // Áp trạng thái đã đến trước yêu cầu REST (đặc biệt accept sau discarded).
                        loop {
                            match updates.try_recv() {
                                Ok(update) => { let _ = self.update(update).await; }
                                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                                    for i in 0..self.calls.len() {
                                        if self.calls[i].state != "ended" { self.fail(i, "Mất update TDLib; đã kết thúc cuộc gọi").await; }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        let result = self.api(action, body).await; let _ = reply.send(result);
                    }
                    Some(Command::Update(update, reply)) => { let result = self.update(update).await.map(|_| json!({"ok":true})); let _ = reply.send(result); }
                    Some(Command::Media(event)) => self.media_event(event).await,
                    Some(Command::Snapshot(id, reply)) => {
                        let result = self.calls.iter().find(|c| c.id == id && c.state != "ended")
                            .ok_or_else(|| GwError::not_found("Không có cuộc gọi đang diễn ra này."))
                            .and_then(|c| c.snapshot.response());
                        let _ = reply.send(result);
                    }
                    Some(Command::VideoJpeg(id, jpeg)) => {
                        if let Some(c) = self.calls.iter_mut().find(|c| c.id == id && c.state != "ended") {
                            c.snapshot.finish(jpeg);
                            c.view["jpeg"] = json!(c.snapshot.sequence);
                        }
                    }
                    Some(Command::RestoreMic(id, revision, mic, fmt, path)) => {
                        self.restore_mic(&id, revision, mic, fmt, path).await;
                    }
                    Some(Command::RefreshCapture(id)) => {
                        if let Some(i) = self.calls.iter().position(|c| c.id == id && c.state == "active") {
                            if self.capture(i).await.is_err() { self.calls[i].log("bật lại nguồn gửi đi lỗi"); }
                        }
                    }
                    Some(Command::Shutdown(ack, deadline)) => { shutdown_ack = ack; shutdown_deadline = Some(deadline); break; }
                    None => break,
                },
                update = updates.recv(), if updates_open => match update {
                    Ok(update) => { let _ = self.update(update).await; }
                    Err(broadcast::error::RecvError::Closed) => updates_open = false,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Đã mất trạng thái TDLib: không giữ cuộc gọi có thể đã kết thúc.
                        for i in 0..self.calls.len() {
                            if self.calls[i].state != "ended" { self.fail(i, "Mất update TDLib; đã kết thúc cuộc gọi").await; }
                        }
                    }
                },
                Some(frame) = frames.recv() => self.media_event(frame).await,
            }
        }
        for i in 0..self.calls.len() {
            if self.calls[i].state != "ended" {
                let deadline = shutdown_deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(5));
                if let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    let _ = tokio::time::timeout(left, self.discard(i)).await;
                    let left = deadline.checked_duration_since(Instant::now()).unwrap_or_default();
                    let _ = tokio::time::timeout(left, self.end(i, "runtime dừng")).await;
                } else {
                    self.calls[i].state = "ended";
                    self.calls[i].ended = Some(Instant::now());
                }
            }
        }
        if let Some(ack) = shutdown_ack { let _ = ack.send(()); }
    }

    fn protocol_json(&self) -> Value {
        json!({"@type":"callProtocol","udp_p2p":self.protocol.udp_p2p && self.p2p,"udp_reflector":self.protocol.udp_reflector,
            "min_layer":self.protocol.min_layer,"max_layer":self.protocol.max_layer,"library_versions":self.protocol.library_versions})
    }

    fn event(&self, kind: &str, index: usize) { let _ = self.events.send(json!({"kind":kind,"call":self.calls[index].info()})); }

    fn find(&self, body: &Value) -> Result<usize, GwError> {
        let id = body["id"].as_str().ok_or_else(missing)?;
        self.calls.iter().position(|c| c.id == id).ok_or_else(missing)
    }

    async fn td_request(&self, req: Value) -> Result<Value, GwError> {
        tokio::time::timeout(Duration::from_secs(15), self.td.request(req)).await
            .map_err(|_| GwError::new(504, "TDLIB_TIMEOUT", "TDLib không phản hồi"))?
            .map_err(|e| if e.code.is_some() { GwError::from_tdlib(e) } else {
                GwError::new(502, "TDLIB_FAILED", "Yêu cầu TDLib thất bại")
            })
    }

    async fn api(&mut self, action: Action, body: Value) -> Reply {
        if !matches!(action, Action::Calls | Action::Settings) && !body.is_object() { return Err(GwError::bad_request("Cần JSON object")); }
        match action {
            Action::Calls => Ok(json!({"calls":self.calls.iter().skip(self.calls.len().saturating_sub(10)).map(LiveCall::info).collect::<Vec<_>>()})),
            Action::Settings => self.settings(body),
            Action::CallOut => self.call_out(body).await,
            Action::Accept => self.accept(body).await,
            Action::Hangup => {
                let i = self.find(&body)?;
                if self.calls[i].state != "ended" {
                    let result = self.discard(i).await;
                    self.end(i, "người dùng cúp").await;
                    result?;
                }
                Ok(self.calls[i].info())
            }
            Action::Media => self.set_media(body).await,
            Action::Playback => self.set_playback(body).await,
            Action::Say => self.say(body).await,
        }
    }

    fn settings(&mut self, body: Value) -> Reply {
        if body.is_null() { return Ok(self.defaults.json(true)); }
        let mut devices = self.defaults.clone();
        devices.update(&body)?;
        fs::create_dir_all(&self.data_dir).map_err(|_| io_error())?;
        let tmp = self.data_dir.join("live_settings.json.tmp");
        let mut file = OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&tmp).map_err(|_| io_error())?;
        file.write_all(devices.json(false).to_string().as_bytes()).map_err(|_| io_error())?;
        fs::rename(tmp, self.data_dir.join("live_settings.json")).map_err(|_| io_error())?;
        self.defaults = devices;
        Ok(json!({"ok":true}))
    }

    async fn resolve_user(&self, body: &Value) -> Result<i64, GwError> {
        let bad = || GwError::bad_request("Thiếu người nhận: user_id (số) hoặc peer (@username, +số điện thoại).");
        if let Some(id) = body["peer"].as_i64().filter(|id| *id > 0) { return Ok(id); }
        if let Some(peer) = body["peer"].as_str().filter(|s| !s.trim().is_empty()) {
            let peer = peer.trim();
            if let Ok(id) = peer.parse::<i64>() { if !peer.starts_with('+') && id > 0 { return Ok(id); } }
            if peer.starts_with('+') {
                let phone = peer[1..].chars().filter(|c| !c.is_whitespace()).collect::<String>();
                if phone.is_empty() || !phone.bytes().all(|b| b.is_ascii_digit()) { return Err(bad()); }
                return self.td_request(json!({"@type":"searchUserByPhoneNumber","phone_number":phone,"only_local":false})).await?["id"].as_i64().filter(|id| *id > 0).ok_or_else(missing);
            }
            let chat = self.td_request(json!({"@type":"searchPublicChat","username":peer.trim_start_matches('@')})).await?;
            if chat["type"]["@type"] != "chatTypePrivate" { return Err(missing()); }
            return chat["type"]["user_id"].as_i64().filter(|id| *id > 0).ok_or_else(missing);
        }
        body["user_id"].as_i64().or_else(|| body["user_id"].as_str().and_then(|v| v.parse().ok())).filter(|id| *id > 0).ok_or_else(bad)
    }

    async fn call_out(&mut self, body: Value) -> Reply {
        let user = self.resolve_user(&body).await?;
        if self.calls.iter().any(|c| c.user_id == user && c.state != "ended") { return Err(GwError::new(409, "CALL_BUSY", "đang có cuộc gọi với người này")); }
        let mut devices = self.defaults.clone();
        let mut overrides = body.as_object().unwrap().clone();
        for key in ["mic", "camera"] {
            if overrides.get(key).is_some_and(|v| v.is_null() || v.as_str() == Some("")) {
                overrides.remove(key);
            }
        }
        if let Some(speaker) = overrides.remove("speaker") {
            let speaker = speaker.as_str().ok_or_else(|| GwError::bad_request("speaker phải là chuỗi"))?.trim();
            if !speaker.is_empty() && !valid_speaker_url(speaker) {
                return Err(GwError::bad_request("speaker phải là cửa bộ đàm hoặc cast:// nội bộ"));
            }
            devices.speaker = speaker.to_owned();
        }
        devices.update(&Value::Object(overrides))?;
        if body["playback"] == "shell" { devices.playback = "shell".into(); }
        let video = body["video"].as_bool().unwrap_or(false);
        let call = LiveCall::new(user, true, video, devices, &self.data_dir);
        call.playback()?; // Cast trả 501 ngay, trước khi createCall gây hiệu ứng bên ngoài.
        self.prune();
        self.calls.push(call);
        let i = self.calls.len()-1;
        match self.td_request(json!({"@type":"createCall","user_id":user,"protocol":self.protocol_json(),"is_video":video})).await {
            Ok(reply) => {
                let id = reply["call_id"].as_i64().or_else(|| reply["id"].as_i64()).or_else(|| reply.as_i64())
                    .filter(|id| *id > 0).ok_or_else(|| GwError::new(502,"TDLIB_FAILED","createCall không trả mã cuộc gọi"));
                match id {
                    Ok(id) => {
                        self.calls[i].td_id = Some(id);
                        self.calls[i].log("TDLib: createCall");
                        // Phản hồi createCall cũng là Call; ready/discard có thể đã tới ở đây.
                        if reply["state"].is_object() { self.update(json!({"@type":"updateCall","call":reply})).await?; }
                        Ok(self.calls[i].info())
                    }
                    Err(e) => { self.end(i, "createCall không trả mã cuộc gọi").await; Err(e) }
                }
            }
            Err(e) => { self.end(i, "createCall thất bại").await; Err(e) }
        }
    }

    async fn accept(&mut self, body: Value) -> Reply {
        let i = self.find(&body)?;
        let c = &self.calls[i];
        if c.outgoing || c.accepted || c.ready_done || c.state != "ringing" { return Err(missing()); }
        let mut devices = self.defaults.clone();
        // apply_defaults: chỉ external/file được thừa kế nếu không có speaker.
        if !matches!(devices.playback.as_str(), "external" | "file") { devices.playback = "file".into(); }
        let kind = if devices.speaker.is_empty() { devices.playback.clone() } else { "push".into() };
        playback_description(&kind, &devices.speaker, &c.playback_path.to_string_lossy(), c.view_on)?;
        self.td_request(json!({"@type":"acceptCall","call_id":c.td_id,"protocol":self.protocol_json()})).await?;
        let c = &mut self.calls[i];
        c.accepted = true;
        c.devices = devices;
        c.playback_kind = kind;
        c.playback_cmd = c.devices.speaker.clone();
        c.log("đã nghe máy");
        Ok(c.info())
    }

    async fn discard(&self, i: usize) -> Result<(), GwError> {
        let c = &self.calls[i];
        if let Some(id) = c.td_id {
            self.td_request(json!({"@type":"discardCall","call_id":id,"invite_link":"","is_disconnected":false,"duration":c.active.map(|t| t.elapsed().as_secs()).unwrap_or(0),"is_video":c.video,"connection_id":0})).await?;
        }
        Ok(())
    }

    async fn end(&mut self, i: usize, why: &str) {
        if self.calls[i].state == "ended" { return; }
        let c = &mut self.calls[i];
        c.state = "ended";
        c.snapshot = Default::default();
        c.ended = Some(Instant::now());
        if let Some(id) = c.td_id { self.handled.insert(id); }
        c.log(why);
        c.pending_signaling.clear();
        if c.media_created {
            let _ = self.media.execute(MediaOp::Stop(c.user_id)).await;
            c.media_created = false;
        }
        c.playback_final = Some(c.playback_size());
        let _ = fs::remove_file(&c.playback_path);
        let _ = fs::remove_file(format!("{}.wav", c.playback_path.to_string_lossy()));
        if let Some(revision) = c.speech_revision.take() {
            let _ = fs::remove_file(self.data_dir.join("live").join(format!("{}_say_{revision}.wav", c.id)));
        }
        self.event("ended", i);
    }

    async fn fail(&mut self, i: usize, why: &str) {
        self.calls[i].error = sources::mask(why);
        let _ = self.discard(i).await;
        self.end(i, why).await;
    }

    fn prune(&mut self) {
        let mut ended = self.calls.iter().filter(|c| c.state == "ended").map(|c| (c.ended,c.id.clone())).collect::<Vec<_>>();
        ended.sort_by_key(|(time,_)| *time);
        let remove = ended.iter().take(ended.len().saturating_sub(20)).map(|(_,id)|id.clone()).collect::<HashSet<_>>();
        self.calls.retain(|c| !remove.contains(&c.id));
    }

    async fn update(&mut self, update: Value) -> Result<(), GwError> {
        match update["@type"].as_str() {
            Some("updateCall") => {
                let call = &update["call"];
                let id = call["id"].as_i64().ok_or_else(|| GwError::bad_request("updateCall thiếu id"))?;
                let state = &call["state"];
                let kind = state["@type"].as_str().unwrap_or("");
                if self.handled.contains(&id) { return Ok(()); }
                let mut found = self.calls.iter().position(|c| c.td_id == Some(id));
                if found.is_none() && call["is_outgoing"] == true {
                    found = self.calls.iter().position(|c| c.outgoing && c.td_id.is_none() && c.state != "ended" && Some(c.user_id) == call["user_id"].as_i64());
                    if let Some(i) = found { self.calls[i].td_id = Some(id); }
                }
                if found.is_none() {
                    if matches!(kind, "callStateDiscarded" | "callStateError" | "callStateHangingUp" | "callStateReady" | "callStateExchangingKeys") {
                        self.handled.insert(id); // Cuộc gọi cũ / đã được thiết bị khác xử lý.
                        return Ok(());
                    }
                    if kind != "callStatePending" || call["is_outgoing"] != false { return Ok(()); }
                    let user = call["user_id"].as_i64().filter(|id| *id > 0).ok_or_else(|| GwError::bad_request("updateCall thiếu user_id"))?;
                    if self.calls.iter().any(|c| c.user_id == user && c.state != "ended") { return Ok(()); }
                    self.prune();
                    let mut c = LiveCall::new(user, false, call["is_video"].as_bool().unwrap_or(false), Devices::default(), &self.data_dir);
                    c.td_id = Some(id);
                    self.calls.push(c);
                    found = Some(self.calls.len()-1);
                    self.event("incoming", found.unwrap());
                }
                let i = found.unwrap();
                if self.calls[i].state == "ended" { return Ok(()); }
                match kind {
                    "callStatePending" if !self.calls[i].ready_done && !self.calls[i].accepted => {
                        self.calls[i].state = if !self.calls[i].outgoing || state["is_received"] == true { "ringing" } else { "dialing" };
                        self.calls[i].log("TDLib: pending");
                    }
                    "callStateExchangingKeys" if !self.calls[i].ready_done => {
                        if !self.calls[i].outgoing && !self.calls[i].accepted {
                            self.end(i, "cuộc gọi đã được xử lý ở thiết bị khác").await;
                        } else { self.calls[i].state = "connecting"; self.calls[i].log("TDLib: đang trao đổi khoá"); }
                    }
                    "callStateReady" if !self.calls[i].ready_done => {
                        if !self.calls[i].outgoing && !self.calls[i].accepted {
                            self.end(i, "cuộc gọi đã được xử lý ở thiết bị khác").await;
                            return Ok(());
                        }
                        self.calls[i].ready_done = true;
                        self.calls[i].state = "connecting";
                        self.calls[i].emojis = state["emojis"].as_array().map(|e| json!(e)).unwrap_or_else(|| json!([]));
                        let ready = match Ready::parse(state, self.p2p) { Ok(r) => r, Err(_) => { self.fail(i,"TDLib Ready không hợp lệ").await; return Err(media_error()); } };
                        let (common, description) = versions::describe(&ready.versions, &self.protocol.library_versions);
                        self.calls[i].log(&format!("TDLib: Ready; thương lượng={description}"));
                        if !ready.versions.is_empty() && !self.protocol.library_versions.is_empty() && common.is_none() {
                            self.fail(i, &format!("phiên bản tgcalls không tương thích: {description}")).await;
                            return Ok(());
                        }
                        if let Err(e) = self.ready(i, ready).await {
                            self.fail(i, &e.message).await;
                            return Err(e);
                        }
                    }
                    "callStateDiscarded" => {
                        let reason = state["reason"]["@type"].as_str().unwrap_or("?");
                        self.end(i, &format!("TDLib: bị kết thúc ({reason})")).await;
                    }
                    "callStateError" => { self.calls[i].error = sources::mask(state["error"]["message"].as_str().unwrap_or("lỗi TDLib")); self.end(i,"TDLib: lỗi cuộc gọi").await; }
                    "callStateHangingUp" => self.end(i,"TDLib: đang cúp").await,
                    _ => (),
                }
            }
            Some("updateNewCallSignalingData") => {
                let id = update["call_id"].as_i64().ok_or_else(|| GwError::bad_request("Báo hiệu thiếu call_id"))?;
                if let Some(i) = self.calls.iter().position(|c| c.td_id == Some(id) && c.state != "ended") {
                    let bytes = STANDARD.decode(update["data"].as_str().unwrap_or("")).map_err(|_| GwError::bad_request("Báo hiệu không phải base64"))?;
                    self.calls[i].sig[2] += 1;
                    self.calls[i].sig[3] += bytes.len() as u64;
                    if self.calls[i].media_created {
                        if self.media.execute(MediaOp::Signaling(self.calls[i].user_id, bytes)).await.is_err() { self.fail(i,"NTgCalls không nhận được báo hiệu").await; return Err(media_error()); }
                    } else { self.calls[i].pending_signaling.push(bytes); }
                }
            }
            _ => (),
        }
        Ok(())
    }

    async fn ready(&mut self, i: usize, ready: Ready) -> Result<(), GwError> {
        let c = &mut self.calls[i];
        fs::create_dir_all(self.data_dir.join("live")).map_err(|_| io_error())?;
        c.log("NTgCalls: create_p2p_call");
        self.media.execute(MediaOp::Create(c.user_id)).await.map_err(|_| media_error())?;
        c.media_created = true;
        self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Capture, c.devices.capture(c.video, c.cam_off, self.profile))).await.map_err(|_| media_error())?;
        self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Playback, c.playback()?)).await.map_err(|_| media_error())?;
        if !ready.key.is_empty() {
            c.log("NTgCalls: skip_exchange");
            self.media.execute(MediaOp::Skip(c.user_id, ready.key.clone(), c.outgoing)).await.map_err(|_| media_error())?;
        }
        c.log("NTgCalls: connect_p2p");
        self.media.execute(MediaOp::Connect(c.user_id, ready)).await.map_err(|_| media_error())?;
        for data in std::mem::take(&mut c.pending_signaling) { self.media.execute(MediaOp::Signaling(c.user_id, data)).await.map_err(|_| media_error())?; }
        Ok(())
    }

    async fn capture(&mut self, i: usize) -> Result<(), GwError> {
        let c = &self.calls[i];
        if c.media_created { self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Capture, c.devices.capture(c.video,c.cam_off,self.profile))).await.map_err(|_| media_error())?; }
        Ok(())
    }

    async fn set_media(&mut self, body: Value) -> Reply {
        let i = self.find(&body)?;
        if self.calls[i].state == "ended" { return Err(missing()); }
        if !["mic","camera","camera_off","video"].iter().any(|k| body.get(*k).is_some()) && !matches!(body["mic_format"].as_str(),Some("auto"|"s16le16k")) {
            return Err(GwError::bad_request("Cần mic, camera, camera_off hoặc video."));
        }
        let c = &mut self.calls[i];
        let cam_off = body.get("camera_off").map(|v| v.as_bool().ok_or_else(|| GwError::bad_request("camera_off phải là boolean"))).transpose()?;
        let mut devices = c.devices.clone();
        // Chỉ cập nhật capture, không nhận nhầm speaker/playback từ endpoint media.
        let mut capture = json!({});
        for key in ["mic","camera","mic_format"] { if let Some(v) = body.get(key) { capture[key] = v.clone(); } }
        devices.update(&capture)?;
        if devices.mic != c.devices.mic {
            c.capture_revision += 1;
        }
        c.devices = devices;
        if let Some(value) = cam_off { c.cam_off = value; }
        if body["video"] == true { c.video = true; c.cam_off = false; }
        self.capture(i).await?;
        self.calls[i].log("đổi nguồn gửi đi");
        Ok(self.calls[i].info())
    }

    async fn set_playback(&mut self, body: Value) -> Reply {
        let i = self.find(&body)?;
        let c = &self.calls[i];
        if c.state == "ended" { return Err(missing()); }
        let kind = body["kind"].as_str().filter(|s| !s.is_empty()).unwrap_or("file").to_owned();
        // push CHỈ lấy url đã kiểm tra; cmd không thể ghi đè URL như một lệnh shell.
        let cmd = if kind == "push" { body["url"].as_str().filter(|s| !s.is_empty()).unwrap_or(&c.devices.speaker) }
            else { body["cmd"].as_str().unwrap_or("") }.to_owned();
        let desc = playback_description(&kind, &cmd, &c.playback_path.to_string_lossy(), c.view_on)?;
        if c.media_created { self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Playback, desc)).await.map_err(|_| media_error())?; }
        let c = &mut self.calls[i];
        c.playback_kind = kind;
        if c.playback_kind == "push" { c.devices.speaker = cmd.clone(); }
        c.playback_cmd = cmd;
        c.log("đổi cách nhận tiếng");
        Ok(c.info())
    }

    async fn media_event(&mut self, event: MediaEvent) {
        let user = match &event { MediaEvent::Signaling(u,_) | MediaEvent::Connection(u,_) | MediaEvent::Frames(u,_,_,_) | MediaEvent::PeerVideo(u,_) => *u };
        let Some(i) = self.calls.iter().position(|c| c.user_id == user && c.state != "ended" && c.media_created) else { return };
        match event {
            MediaEvent::Signaling(_, bytes) => {
                self.calls[i].sig[0] += 1;
                self.calls[i].sig[1] += bytes.len() as u64;
                if self.td_request(json!({"@type":"sendCallSignalingData","call_id":self.calls[i].td_id,"data":STANDARD.encode(bytes)})).await.is_err() {
                    self.fail(i,"TDLib không gửi được báo hiệu").await;
                }
            }
            MediaEvent::Connection(_, state) => match state {
                ConnectionState::Connected => {
                    if self.calls[i].active.is_none() {
                        let c = &mut self.calls[i];
                        c.state = "active"; c.active = Some(Instant::now()); c.log("NTgCalls kết nối: CONNECTED");
                        if c.outgoing && c.video {
                            let tx = self.sender.clone(); let id = c.id.clone();
                            tokio::spawn(async move {
                                for delay in [1500,5000] { tokio::time::sleep(Duration::from_millis(delay)).await; if tx.send(Command::RefreshCapture(id.clone())).is_err() { break; } }
                            });
                        }
                        self.event("active",i);
                    }
                }
                ConnectionState::Failed | ConnectionState::Timeout | ConnectionState::Closed => self.fail(i, &format!("NTgCalls kết nối: {state:?}")).await,
                ConnectionState::Connecting => self.calls[i].log("NTgCalls kết nối: CONNECTING"),
            },
            MediaEvent::Frames(_, mode, device, frames) => {
                if mode != StreamMode::Playback { return; }
                let c = &mut self.calls[i];
                for frame in frames {
                    if matches!(device, StreamDevice::Camera | StreamDevice::Screen) {
                        let w = frame.frame_data.width as usize; let h = frame.frame_data.height as usize;
                        let key = if snapshot::valid(w, h, frame.data.len()) { "frames" } else { "bad" };
                        c.view[key] = json!(c.view[key].as_u64().unwrap_or(0)+1);
                        if key == "frames" {
                            c.view["size"] = json!(format!("{w}x{h}"));
                            c.snapshot.submit(w, h, frame.data, c.id.clone(), self.sender.clone());
                        }
                    } else {
                        c.frames_in += 1; c.frame_bytes += frame.data.len() as u64;
                        for sample in frame.data.chunks_exact(2).take(960) { c.peak = c.peak.max((i16::from_le_bytes([sample[0],sample[1]]) as i32).abs()); }
                    }
                }
            }
            MediaEvent::PeerVideo(_, on) => {
                if self.calls[i].peer_video != on {
                    self.calls[i].peer_video = on;
                    if on && !self.calls[i].view_on {
                        self.calls[i].view_on = true;
                        let c = &self.calls[i];
                        if let Ok(desc) = c.playback() {
                            if self.media.execute(MediaOp::Sources(user,StreamMode::Playback,desc)).await.is_err() { self.calls[i].view_on = false; }
                        }
                    }
                    self.event("peer_video",i);
                }
            }
        }
    }

    async fn say(&mut self, body: Value) -> Reply {
        let text = body["text"].as_str().unwrap_or("").trim();
        let raw = body["audio_b64"].as_str().filter(|s| !s.is_empty());
        if raw.is_none() && (text.is_empty() || text.chars().count()>1000) { return Err(GwError::bad_request("Cần audio_b64 hoặc text (1-1000 ký tự).")); }
        let i = self.find(&body)?;
        if self.calls[i].state != "active" { return Err(missing()); }
        if self.calls[i].speech_revision.is_some() { return Err(GwError::new(409,"CALL_BUSY","Cuộc gọi đang đọc một đoạn tiếng")); }
        let Some(raw) = raw else { return Err(GwError::new(501,"TTS_UNAVAILABLE","Gateway chưa có bộ đọc giọng (TTS).")); };
        if raw.len()>28_000_000 { return Err(GwError::bad_request("Âm thanh quá lớn.")); }
        let audio = STANDARD.decode(raw).map_err(|_| GwError::bad_request("audio_b64 không phải base64 hợp lệ."))?;
        let pcm = decode_audio(audio).await?;
        self.start_speech(i, &pcm).await
    }

    async fn start_speech(&mut self, i: usize, pcm: &[u8]) -> Reply {
        let seconds = pcm.len() as f64/96_000.0;
        let revision = self.calls[i].capture_revision+1;
        let path = self.data_dir.join("live").join(format!("{}_say_{revision}.wav",self.calls[i].id));
        let mut file = OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&path).map_err(|_| io_error())?;
        file.write_all(&wav(pcm)).map_err(|_| io_error())?;
        let c = &mut self.calls[i];
        let (mic,fmt) = (c.devices.mic.clone(), c.devices.mic_format.clone());
        c.devices.mic = path.to_string_lossy().into_owned(); c.devices.mic_format = "auto".into(); c.capture_revision += 1;
        c.speech_revision = Some(revision);
        let (id,revision) = (c.id.clone(),c.capture_revision);
        if let Err(e) = self.capture(i).await {
            self.calls[i].devices.mic = mic; self.calls[i].devices.mic_format = fmt; self.calls[i].speech_revision = None; let _ = fs::remove_file(path); return Err(e);
        }
        let tx = self.sender.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(seconds+0.4)).await;
            let _ = tx.send(Command::RestoreMic(id,revision,mic,fmt,path));
        });
        Ok(json!({"seconds":(seconds*10.0).round()/10.0}))
    }

    async fn restore_mic(&mut self, id: &str, revision: u64, mic: String, fmt: String, path: PathBuf) {
        if let Some(i) = self.calls.iter().position(|c| c.id == id) {
            if self.calls[i].speech_revision == Some(revision) { self.calls[i].speech_revision = None; }
            if self.calls[i].state != "ended" && self.calls[i].capture_revision == revision {
                self.calls[i].devices.mic = mic;
                self.calls[i].devices.mic_format = fmt;
                if self.capture(i).await.is_err() { self.calls[i].log("không trả được nguồn mic sau say"); }
            }
        }
        let _ = fs::remove_file(path);
    }
}

async fn decode_audio(audio: Vec<u8>) -> Result<Vec<u8>, GwError> {
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut child = tokio::process::Command::new("ffmpeg").args(["-hide_banner","-loglevel","error","-nostdin","-i","pipe:0","-vn","-f","s16le","-ar","48000","-ac","1","pipe:1"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().map_err(|_| GwError::new(501,"AUDIO_UNAVAILABLE","Không chạy được ffmpeg"))?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let write = async { input.write_all(&audio).await?; input.shutdown().await };
        let read = async { let mut pcm = Vec::new(); output.take(48_000_001).read_to_end(&mut pcm).await?; Ok::<_,std::io::Error>(pcm) };
        let (_, pcm) = tokio::try_join!(write,read)?;
        if pcm.is_empty() || pcm.len()>48_000_000 { return Err(std::io::Error::other("Âm thanh trống/quá dài")); }
        if !child.wait().await?.success() { return Err(std::io::Error::other("ffmpeg không đọc được âm thanh")); }
        Ok::<_,std::io::Error>(pcm)
    }).await;
    match result { Ok(Ok(pcm)) => Ok(pcm), _ => Err(GwError::bad_request("Không giải mã được âm thanh hoặc âm thanh quá dài")) }
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44+pcm.len());
    out.extend_from_slice(b"RIFF"); out.extend_from_slice(&(36+pcm.len() as u32).to_le_bytes()); out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); out.extend_from_slice(&1u16.to_le_bytes()); out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&48000u32.to_le_bytes()); out.extend_from_slice(&96000u32.to_le_bytes()); out.extend_from_slice(&2u16.to_le_bytes()); out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data"); out.extend_from_slice(&(pcm.len() as u32).to_le_bytes()); out.extend_from_slice(pcm); out
}

#[cfg(test)]
mod tests;
