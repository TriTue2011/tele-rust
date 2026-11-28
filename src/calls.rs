//! Cuộc gọi P2P: một CallManager cho một TdLink/tài khoản.
//!
//! `new` tự đăng ký updates(); tầng REST giữ một bản clone làm `dyn CallControl`.
//! `events` phát incoming/active/ended/peer_video. NTgCalls nằm trong một thread
//! có Tokio runtime riêng, vì binding 3.0.7 không Sync. Không giữ khóa qua await.
mod media;
mod cast;
mod sources;
mod snapshot;
mod browser;
mod browser_audio;
mod audio_sink;
mod catalog;
mod speech;
pub(crate) mod trace;
pub mod versions;

pub use sources::{camera_command, is_cast_spec, mic_command, speaker_command, valid_speaker_url};

use std::{collections::{HashSet, VecDeque}, fs::{self, OpenOptions}, io::Write,
    os::unix::fs::OpenOptionsExt, path::{Path, PathBuf}, sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

use base64::{engine::general_purpose::STANDARD, Engine};
use ntgcalls::{ConnectionState, NTgCalls, Protocol, StreamDevice, StreamMode};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::contract::{AudioOut, Binary, BoxFut, CallControl, CastAudio, GwError, TdLink, VideoFrames};
use crate::events::{mask_peer, mask_peer_id};
use media::{Media, MediaEvent, MediaOp, NativeMedia, Ready};
use sources::{cast_playback_description, Devices, playback_description, video_profile};

type Reply = Result<Value, GwError>;

#[derive(Clone, Copy)]
enum Action { Calls, Resolve, CallOut, Accept, Hangup, Settings, Devices, Media, Playback, Say }

enum Command {
    Api(Action, Value, oneshot::Sender<Reply>),
    Update(Value, oneshot::Sender<Reply>),
    Media(MediaEvent),
    Snapshot(String, oneshot::Sender<Result<Binary, GwError>>),
    VideoFrames(String, oneshot::Sender<Result<VideoFrames, GwError>>),
    VideoJpeg(String, Option<Vec<u8>>),
    CastAudio(String, String, oneshot::Sender<Option<CastAudio>>),
    AudioIn(String, Vec<u8>, oneshot::Sender<Reply>, tokio::sync::OwnedSemaphorePermit),
    VideoIn(String, Vec<u8>, oneshot::Sender<Reply>),
    AudioOut(String, String, oneshot::Sender<Result<AudioOut, GwError>>),
    OwnsCall(String, oneshot::Sender<bool>),
    HasActiveCall(oneshot::Sender<bool>),
    RestoreMic(String, u64, String, String),
    SayReady(String, Result<Vec<u8>, GwError>, oneshot::Sender<Reply>),
    RefreshCapture(String),
    Shutdown(Option<std::sync::mpsc::Sender<()>>, Instant),
}

struct Handle {
    sender: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<Value>,
    shutting_down: AtomicBool,
    audio_slots: Arc<tokio::sync::Semaphore>,
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
        Self::with_cast(td, data_dir.as_ref(), None)
    }

    pub(crate) fn with_cast(td: Arc<dyn TdLink>, data_dir: &Path, cast_port: Option<u16>) -> Result<Self, GwError> {
        trace::Config::from_env().map_err(GwError::bad_request)?;
        Self::spawn(td, data_dir.to_path_buf(), Box::new(NativeMedia(NTgCalls::new())),
            p2p_enabled(std::env::var("TELEGRAM_P2P").ok().as_deref()), cast_port)
    }

    fn spawn(td: Arc<dyn TdLink>, data_dir: PathBuf, mut media: Box<dyn Media>, p2p: bool, cast_port: Option<u16>) -> Result<Self, GwError> {
        let updates = td.updates(); // Đăng ký trước khi bất kỳ createCall nào có thể gửi đi.
        let protocol = media.protocol().map_err(|_| GwError::new(502, "MEDIA_FAILED", "Không đọc được protocol NTgCalls"))?;
        let (sender, receiver) = mpsc::unbounded_channel();
        let (frame_sender, frame_receiver) = mpsc::channel(3);
        let (audio_sender, audio_receiver) = mpsc::channel(256);
        let audio_rx_dropped = Arc::new(AtomicU64::new(0));
        let (events, _) = broadcast::channel(256);
        let handle = Arc::new(Handle { sender: sender.clone(), events: events.clone(), shutting_down: AtomicBool::new(false), audio_slots: Arc::new(tokio::sync::Semaphore::new(8)) });
        let (startup_tx, startup_rx) = std::sync::mpsc::sync_channel(1);
        let audio_dropped_cb = audio_rx_dropped.clone();
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
                if let Ok(entries) = fs::read_dir(data_dir.join("live")) {
                    for entry in entries.flatten() {
                        // Chỉ tệp tạm lời nói «<cuộc>_say_<rev>.pcm|wav» do start_speech sinh — không đụng tệp âm thanh khác trong live/.
                        if entry.path().extension().is_some_and(|ext| ext == "pcm" || ext == "wav") && entry.file_name().to_string_lossy().contains("_say_") {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
                }
                media.callbacks(sender.clone(), frame_sender, audio_sender, audio_dropped_cb);
                let catalog = match catalog::load(&data_dir) { Ok(c) => c, Err(e) => { let _ = startup_tx.send(Err(e)); return; } };
                let mut actor = Actor { td, media, protocol, p2p, data_dir, defaults, calls: Vec::new(),
                    catalog, handled: HashSet::new(), handled_order: VecDeque::new(), events, sender, profile: video_profile(&std::env::var("VIDEO_PROFILE").unwrap_or_default()), cast_port, audio_rx_dropped };
                let _ = startup_tx.send(Ok(()));
                actor.run(receiver, updates, frame_receiver, audio_receiver).await;
            });
        }).map_err(|_| GwError::new(500, "CALL_RUNTIME", "Không tạo được luồng cuộc gọi"))?;
        startup_rx.recv().map_err(|_| runtime_error())??;
        Ok(Self { handle })
    }

    pub fn events(&self) -> broadcast::Receiver<Value> { self.handle.events.subscribe() }

    pub(crate) async fn resolve(&self,body:Value)->Reply { self.request(Action::Resolve,body).await }

    pub(crate) async fn owns_call(&self, id: &str) -> bool {
        let (tx, rx) = oneshot::channel();
        if self.handle.sender.send(Command::OwnsCall(id.to_owned(), tx)).is_err() { return false; }
        rx.await.unwrap_or(false)
    }

    pub(crate) async fn has_active_call(&self) -> Result<bool, GwError> {
        let (tx, rx) = oneshot::channel();
        self.handle.sender.send(Command::HasActiveCall(tx)).map_err(|_| runtime_error())?;
        rx.await.map_err(|_| runtime_error())
    }

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
    fn devices(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Devices, body)) }
    fn media(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Media, body)) }
    fn playback(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Playback, body)) }
    fn say(&self, body: Value) -> BoxFut<'_, Reply> { Box::pin(self.request(Action::Say, body)) }
    fn audio_in(&self, id: &str, pcm: Vec<u8>) -> BoxFut<'_, Reply> {
        let id = id.to_owned();
        Box::pin(async move {
            if pcm.len() > browser::MAX_PCM { return Err(GwError::new(413, "AUDIO_TOO_LARGE", "PCM vượt 64 KiB.")); }
            let permit = self.handle.audio_slots.clone().try_acquire_owned()
                .map_err(|_| GwError::new(429, "AUDIO_BACKPRESSURE", "Quá nhiều khối mic đang chờ."))?;
            let (tx, rx) = oneshot::channel();
            self.handle.sender.send(Command::AudioIn(id, pcm, tx, permit)).map_err(|_| runtime_error())?;
            rx.await.map_err(|_| runtime_error())?
        })
    }
    fn video_in(&self, id: &str, jpeg: Vec<u8>) -> BoxFut<'_, Reply> {
        let id = id.to_owned();
        Box::pin(async move {
            if jpeg.len() > browser::MAX_JPEG { return Err(GwError::new(413, "VIDEO_TOO_LARGE", "Khung JPEG vượt 256 KiB.")); }
            let (tx, rx) = oneshot::channel();
            self.handle.sender.send(Command::VideoIn(id, jpeg, tx)).map_err(|_| runtime_error())?;
            rx.await.map_err(|_| runtime_error())?
        })
    }
    fn audio_out(&self, id: &str, token: &str) -> BoxFut<'_, Result<AudioOut, GwError>> {
        let (id, token) = (id.to_owned(), token.to_owned());
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            self.handle.sender.send(Command::AudioOut(id, token, tx)).map_err(|_| runtime_error())?;
            rx.await.map_err(|_| runtime_error())?
        })
    }
    fn snapshot(&self, id: &str) -> BoxFut<'_, Result<Binary, GwError>> {
        let id = id.to_owned();
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            self.handle.sender.send(Command::Snapshot(id, tx)).map_err(|_| runtime_error())?;
            rx.await.map_err(|_| runtime_error())?
        })
    }
    fn video_frames(&self, id: &str) -> BoxFut<'_, Result<VideoFrames, GwError>> {
        let id = id.to_owned();
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            self.handle.sender.send(Command::VideoFrames(id, tx)).map_err(|_| runtime_error())?;
            rx.await.map_err(|_| runtime_error())?
        })
    }
    fn cast_audio(&self, id: &str, token: &str) -> BoxFut<'_, Option<CastAudio>> {
        let (id, token) = (id.to_owned(), token.to_owned());
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            self.handle.sender.send(Command::CastAudio(id, token, tx)).ok()?;
            rx.await.ok().flatten()
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

const MAX_PENDING_SIGNALING: usize = 256;

struct CaptureHandles {
    mic: Option<browser::BrowserMic>,
    cam: Option<browser::BrowserCam>,
    speech: Option<speech::SpeechStream>,
    drop_mic: bool,
    drop_cam: bool,
    drop_speech: bool,
}

#[derive(Default)]
struct TtsDiag {
    utterances: u64,
    frames_sent: u64,
    frames_total: u64,
    late_frames: u64,
    missed_frames: u64,
    max_gap_ms: u64,
    last_connect_ms: Option<u64>,
}

impl TtsDiag {
    fn record(&mut self, snap: speech::SpeechSnapshot) {
        self.utterances += 1;
        self.frames_sent += snap.frames_sent;
        self.frames_total += snap.frames_total;
        self.late_frames += snap.late_frames;
        self.missed_frames += snap.missed_frames;
        if snap.max_gap_ms > self.max_gap_ms { self.max_gap_ms = snap.max_gap_ms; }
        if snap.connect_delay_ms.is_some() { self.last_connect_ms = snap.connect_delay_ms; }
    }

    fn json(&self, active: Option<speech::SpeechSnapshot>) -> Value {
        let (mut utterances, mut frames_sent, mut frames_total, mut late_frames, mut missed_frames, mut max_gap_ms, mut last_connect_ms) = (
            self.utterances, self.frames_sent, self.frames_total, self.late_frames, self.missed_frames, self.max_gap_ms, self.last_connect_ms,
        );
        if let Some(snap) = active {
            utterances += 1;
            frames_sent += snap.frames_sent;
            frames_total += snap.frames_total;
            late_frames += snap.late_frames;
            missed_frames += snap.missed_frames;
            if snap.max_gap_ms > max_gap_ms { max_gap_ms = snap.max_gap_ms; }
            if snap.connect_delay_ms.is_some() { last_connect_ms = snap.connect_delay_ms; }
        }
        json!({
            "utterances": utterances,
            "frames_sent": frames_sent,
            "frames_total": frames_total,
            "late_frames": late_frames,
            "missed_frames": missed_frames,
            "max_gap_ms": max_gap_ms,
            "last_connect_ms": last_connect_ms,
        })
    }
}

struct SttDiag {
    active_frames: u64,
    muted_frames_in: u64,
    saying_frames_in: u64,
    sum_sq: u128,
    samples: u64,
    last_frame_at: Option<Instant>,
    max_gap_ms: u64,
    gap_events: u64,
    gap_cut_ms: u64,
    rx_dropped_base: u64,
    rx_dropped: u64,
    browser_lagged: u64,
    browser_no_listener: u64,
    browser_invalid: u64,
    win_frames: u64,
    win_active: u64,
    win_peak: i32,
    win_sum_sq: u128,
    win_samples: u64,
    win_max_gap_ms: u64,
    win_gap_cut_ms: u64,
    win_prev_dropped: u64,
}

impl SttDiag {
    fn new(rx_dropped_base: u64) -> Self {
        Self {
            active_frames: 0,
            muted_frames_in: 0,
            saying_frames_in: 0,
            sum_sq: 0,
            samples: 0,
            last_frame_at: None,
            max_gap_ms: 0,
            gap_events: 0,
            gap_cut_ms: 0,
            rx_dropped_base,
            rx_dropped: 0,
            browser_lagged: 0,
            browser_no_listener: 0,
            browser_invalid: 0,
            win_frames: 0,
            win_active: 0,
            win_peak: 0,
            win_sum_sq: 0,
            win_samples: 0,
            win_max_gap_ms: 0,
            win_gap_cut_ms: 0,
            win_prev_dropped: 0,
        }
    }

    fn on_frame(&mut self, pcm: &[u8], mic_muted: bool, saying: bool, now: Instant) -> i32 {
        let mut frame_peak = 0i32;
        let mut frame_sum_sq = 0u128;
        let mut frame_samples = 0u64;
        for sample in pcm.chunks_exact(2) {
            let v = i32::from(i16::from_le_bytes([sample[0], sample[1]]));
            let abs = v.abs();
            if abs > frame_peak { frame_peak = abs; }
            let vv = i64::from(v);
            frame_sum_sq += (vv * vv) as u128;
            frame_samples += 1;
        }
        if let Some(prev) = self.last_frame_at {
            let gap_ms = now.duration_since(prev).as_millis() as u64;
            if gap_ms > self.max_gap_ms { self.max_gap_ms = gap_ms; }
            if gap_ms > self.win_max_gap_ms { self.win_max_gap_ms = gap_ms; }
            if gap_ms > 40 {
                self.gap_events += 1;
                let cut = gap_ms.saturating_sub(10);
                self.gap_cut_ms += cut;
                self.win_gap_cut_ms += cut;
            }
        }
        self.last_frame_at = Some(now);
        if frame_peak >= 300 {
            self.active_frames += 1;
            self.win_active += 1;
        }
        if mic_muted { self.muted_frames_in += 1; }
        if saying { self.saying_frames_in += 1; }
        self.sum_sq += frame_sum_sq;
        self.samples += frame_samples;
        self.win_frames += 1;
        if frame_peak > self.win_peak { self.win_peak = frame_peak; }
        self.win_sum_sq += frame_sum_sq;
        self.win_samples += frame_samples;
        frame_peak
    }

    fn sync_drops(&mut self, actor_rx_dropped: u64, browser: Option<&browser_audio::BrowserAudio>) {
        self.rx_dropped = actor_rx_dropped.saturating_sub(self.rx_dropped_base);
        if let Some(browser) = browser {
            let st = browser.stats();
            self.browser_lagged = st.lagged_frames;
            self.browser_no_listener = st.no_listener_frames;
            self.browser_invalid = st.invalid_frames;
        }
    }

    fn total_dropped(&self) -> u64 {
        self.rx_dropped + self.browser_lagged + self.browser_invalid
    }

    fn take_window_line(&mut self) -> Option<String> {
        let cur_dropped = self.total_dropped();
        let win_dropped = cur_dropped.saturating_sub(self.win_prev_dropped);
        self.win_prev_dropped = cur_dropped;
        let cut_ms = self.win_gap_cut_ms + win_dropped * 10;
        if self.win_frames == 0 && cut_ms == 0 {
            return None;
        }
        let rms = if self.win_samples == 0 {
            0u64
        } else {
            ((self.win_sum_sq / u128::from(self.win_samples)) as f64).sqrt().round() as u64
        };
        let line = format!(
            "khung={} peak={} rms={} noi={} ho_max_ms={} cat_ms={}",
            self.win_frames, self.win_peak, rms, self.win_active, self.win_max_gap_ms, cut_ms
        );
        self.win_frames = 0;
        self.win_active = 0;
        self.win_peak = 0;
        self.win_sum_sq = 0;
        self.win_samples = 0;
        self.win_max_gap_ms = 0;
        self.win_gap_cut_ms = 0;
        Some(line)
    }

    fn json(&self, frames_in: u64, peak: i32) -> Value {
        let rms = if self.samples == 0 {
            0u64
        } else {
            ((self.sum_sq / u128::from(self.samples)) as f64).sqrt().round() as u64
        };
        let cut_ms = self.gap_cut_ms + self.total_dropped() * 10;
        json!({
            "frames_in": frames_in,
            "active_frames": self.active_frames,
            "muted_frames_in": self.muted_frames_in,
            "saying_frames_in": self.saying_frames_in,
            "peak": peak,
            "rms": rms,
            "max_gap_ms": self.max_gap_ms,
            "gap_events": self.gap_events,
            "rx_dropped": self.rx_dropped,
            "lagged_frames": self.browser_lagged,
            "no_listener_frames": self.browser_no_listener,
            "invalid_frames": self.browser_invalid,
            "cut_ms": cut_ms,
        })
    }
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
    mic_source: Option<(String, String)>,
    playback_kind: String,
    playback_cmd: String,
    playback_path: PathBuf,
    playback_final: Option<u64>,
    cast: Option<cast::CastSink>,
    browser_mic: Option<browser::BrowserMic>,
    browser_cam: Option<browser::BrowserCam>,
    browser_audio: Option<browser_audio::BrowserAudio>,
    audio_sink: Option<audio_sink::AudioSink>,
    speech_stream: Option<speech::SpeechStream>,
    tts_diag: TtsDiag,
    stt_diag: SttDiag,
    cam_off: bool,
    view_on: bool,
    peer_video: bool,
    view: Value,
    snapshot: snapshot::Snapshot,
    trace: Option<trace::Trace>,
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
    speech_pending: bool,
}

impl LiveCall {
    fn new(user_id: i64, outgoing: bool, video: bool, devices: Devices, data_dir: &Path) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let id = format!("{:x}{:x}{:x}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(), SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let playback_kind = if devices.speaker == "browser" { "browser".into() } else if devices.speaker.is_empty() { devices.playback.clone() } else { "push".into() };
        let playback_cmd = if playback_kind == "push" { devices.speaker.clone() } else { String::new() };
        let mic_source = devices.mic_source();
        let playback_path = data_dir.join("live").join(format!("{id}_in.pcm"));
        let mut c = Self { id, td_id: None, user_id, outgoing, video, state: if outgoing { "dialing" } else { "ringing" },
            accepted: false, ready_done: false, media_created: false, pending_signaling: Vec::new(), devices, mic_source, playback_kind, playback_cmd,
            playback_path, playback_final: None, cast: None, browser_mic: None, browser_cam: None, browser_audio: None, audio_sink: None,
            speech_stream: None, tts_diag: TtsDiag::default(), stt_diag: SttDiag::new(0),
            cam_off: false, view_on: video, peer_video: false,
            view: json!({"frames":0,"jpeg":0,"size":"","dropped":0,"bad":0}), snapshot: Default::default(), trace: None, sig: [0;4], frames_in: 0, frame_bytes: 0,
            peak: 0, error: String::new(), emojis: json!([]), started: Instant::now(), active: None, ended: None,
            events: VecDeque::new(), capture_revision: 0, speech_revision: None, speech_pending: false };
        c.trace = trace::Trace::open(data_dir, &c.id);
        c.log(if outgoing { "tạo cuộc gọi đi" } else { "tạo cuộc gọi đến" });
        c
    }

    fn log(&mut self, text: &str) {
        // Không ghi nguồn, URL hoặc command ra nhật ký cuộc gọi.
        log::debug!("Cuộc gọi {}: {}", self.id, text);
        self.events.push_back(json!({"t":(self.started.elapsed().as_secs_f64()*100.0).round()/100.0,"e":sources::mask(text)}));
        if let Some(trace) = &self.trace { trace.line("su-kien", text); }
        while self.events.len() > 200 { self.events.pop_front(); }
    }

    fn finish_speech_metrics(&mut self, rev: u64) {
        if let Some(stream) = self.speech_stream.take() {
            let snap = stream.stats();
            self.tts_diag.record(snap);
            if let Some(trace) = &self.trace {
                let connect = snap.connect_delay_ms.map_or_else(|| "none".to_owned(), |ms| ms.to_string());
                trace.line(
                    "tts-do",
                    &format!(
                        "rev={rev} khung={}/{} tre={} thieu={} ho_max_ms={} noi_ms={connect} lang={}",
                        snap.frames_sent, snap.frames_total, snap.late_frames, snap.missed_frames, snap.max_gap_ms, snap.silence_frames
                    ),
                );
            }
        }
    }

    fn playback_size(&self) -> u64 {
        self.playback_final.unwrap_or_else(|| fs::metadata(&self.playback_path).map(|m| m.len()).unwrap_or(0))
    }

    fn info(&self) -> Value {
        let duration = self.ended.unwrap_or_else(Instant::now).duration_since(self.active.unwrap_or(self.started)).as_secs_f64();
        json!({"id":self.id,"user_id":self.user_id,"outgoing":self.outgoing,"video":self.video,
            "state":self.state,"emojis":self.emojis,"error":self.error,"seconds":(duration*10.0).round()/10.0,
            "peer_video":self.peer_video,"view":self.view,
            "diag":{"tts":self.tts_diag.json(self.speech_stream.as_ref().map(speech::SpeechStream::stats)),"stt":self.stt_diag.json(self.frames_in,self.peak)},
            "sig":{"tx_n":self.sig[0],"tx_b":self.sig[1],"rx_n":self.sig[2],"rx_b":self.sig[3]},
            "cpu_max":0.0,"mic_muted":self.devices.mic=="silence","saying":self.speech_revision.is_some(),"cam_off":self.cam_off,"frames_in":self.frames_in,"frame_bytes":self.frame_bytes,"peak":self.peak,
            "playback":{"kind":self.playback_kind,"bytes":self.playback_size(),"speaker_configured":!self.devices.speaker.is_empty()},"events":self.events.iter().skip(self.events.len().saturating_sub(40)).collect::<Vec<_>>()})
    }

    fn playback(&self) -> Result<ntgcalls::MediaDescription, GwError> {
        self.playback_with(&self.playback_kind, &self.playback_cmd, self.cast.as_ref())
    }

    async fn prepare_current_capture(&self, profile: (i16, i16, u8)) -> Result<(ntgcalls::MediaDescription, CaptureHandles), GwError> {
        self.prepare_capture(&self.devices, self.video, self.cam_off, profile).await
    }

    /// Chuẩn bị mô tả nguồn gửi đi không làm thay đổi trạng thái hiện tại: chỉ mở BrowserMic/BrowserCam mới nếu cần và chưa có sẵn.
    async fn prepare_capture(&self, devices: &Devices, video: bool, cam_off: bool, profile: (i16, i16, u8)) -> Result<(ntgcalls::MediaDescription, CaptureHandles), GwError> {
        let mut description = devices.capture(video, cam_off, profile);
        let mut handles = CaptureHandles { mic: None, cam: None, speech: None, drop_mic: false, drop_cam: false, drop_speech: false };
        if devices.mic == "browser" {
            handles.drop_speech = true;
            if let Some(existing) = self.browser_mic.as_ref() {
                description.microphone.as_mut().unwrap().input = existing.command();
            } else {
                let mic = browser::BrowserMic::open().await?;
                description.microphone.as_mut().unwrap().input = mic.command();
                handles.mic = Some(mic);
            }
        } else if devices.mic == "speech" {
            handles.drop_mic = true;
            if let Some(existing) = self.speech_stream.as_ref() {
                description.microphone.as_mut().unwrap().input = existing.command();
            } else {
                handles.drop_speech = true;
            }
        } else {
            handles.drop_mic = true;
            handles.drop_speech = true;
        }
        if devices.camera == "browser" && video && !cam_off {
            if let Some(existing) = self.browser_cam.as_ref() {
                if let Some(camera) = description.camera.as_mut() { camera.input = existing.command(profile); }
            } else {
                let cam = browser::BrowserCam::open(profile.2).await?;
                if let Some(camera) = description.camera.as_mut() { camera.input = cam.command(profile); }
                handles.cam = Some(cam);
            }
        } else {
            handles.drop_cam = true;
        }
        Ok((description, handles))
    }

    fn commit_capture(&mut self, handles: CaptureHandles) {
        if let Some(mic) = handles.mic {
            self.browser_mic = Some(mic);
        } else if handles.drop_mic {
            self.browser_mic = None;
        }
        if let Some(speech) = handles.speech {
            if let Some(prev) = self.speech_stream.take() {
                self.tts_diag.record(prev.stats());
            }
            self.speech_stream = Some(speech);
        } else if handles.drop_speech {
            if let Some(prev) = self.speech_stream.take() {
                self.tts_diag.record(prev.stats());
            }
        }
        if let Some(cam) = handles.cam {
            self.browser_cam = Some(cam);
        } else if handles.drop_cam {
            self.browser_cam = None;
        }
    }

    fn playback_with(&self, kind: &str, cmd: &str, cast: Option<&cast::CastSink>) -> Result<ntgcalls::MediaDescription, GwError> {
        let path = self.playback_path.to_string_lossy();
        if kind == "browser" { return playback_description("external", "", &path, self.view_on); }
        // AI đang nghe (browser_audio đã mở): LUÔN đổi sang "external" để NTgCalls giao khung tiếng qua on_frames
        // thay vì tự xử lý nội bộ — bất kể kind là gì (không chỉ "push"). Quan trọng: khi AI bật, connect_now loại
        // trừ mic/speaker (tránh tranh tiếng, xem _answer_by_rule phía HA) nên loa KHÔNG BAO GIỜ là "push" lúc này
        // — nếu chỉ kiểm kind=="push" thì AI sẽ "điếc" đúng lúc cần nghe nhất (đo thật 26/11: frames_in=0 trở lại
        // sau khi sửa connect_now loại trừ speaker). audio_sink.rs chỉ thật sự spawn tiến trình loa khi kind gốc
        // LÀ "push" (xem set_media/set_playback) — call không có loa thật thì audio_sink=None, chỉ tee cho AI thôi.
        if self.browser_audio.is_some() { return playback_description("external", "", &path, self.view_on); }
        if let Some(cast) = cast { return cast_playback_description(&cast.encoder_command(), self.view_on); }
        playback_description(kind, cmd, &path, self.view_on)
    }

    fn check_playback(&self, kind: &str, cmd: &str, cast_port: Option<u16>) -> Result<(), GwError> {
        if kind == "browser" {
            return playback_description("external", "", &self.playback_path.to_string_lossy(), self.view_on).map(|_| ());
        }
        if kind == "push" && is_cast_spec(cmd) {
            return cast_port.map(|_| ()).ok_or_else(|| GwError::new(501, "NOT_IMPLEMENTED", "Chưa có cổng HTTP cho luồng Cast."));
        }
        playback_description(kind, cmd, &self.playback_path.to_string_lossy(), self.view_on).map(|_| ())
    }

    async fn open_cast(kind: &str, cmd: &str) -> Result<Option<cast::CastSink>, GwError> {
        if kind != "push" || !is_cast_spec(cmd) { return Ok(None); }
        cast::CastSink::open().await.map(Some).map_err(|_| media_error())
    }
}

struct Actor {
    td: Arc<dyn TdLink>,
    media: Box<dyn Media>,
    protocol: Protocol,
    p2p: bool,
    data_dir: PathBuf,
    defaults: Devices,
    catalog: Value,
    calls: Vec<LiveCall>,
    // Tombstone tách khỏi lịch sử hiển thị: prune không làm cuộc gọi cũ sống lại.
    handled: HashSet<i64>,
    handled_order: VecDeque<i64>,
    events: broadcast::Sender<Value>,
    sender: mpsc::UnboundedSender<Command>,
    profile: (i16, i16, u8),
    cast_port: Option<u16>,
    audio_rx_dropped: Arc<AtomicU64>,
}

impl Actor {
    fn record_handled(handled: &mut HashSet<i64>, handled_order: &mut VecDeque<i64>, id: i64) {
        if handled.insert(id) {
            handled_order.push_back(id);
            if handled_order.len() > 256 {
                if let Some(old) = handled_order.pop_front() {
                    handled.remove(&old);
                }
            }
        }
    }

    fn mark_handled(&mut self, id: i64) {
        Self::record_handled(&mut self.handled, &mut self.handled_order, id);
    }

    async fn run(&mut self, mut commands: mpsc::UnboundedReceiver<Command>, mut updates: broadcast::Receiver<Value>, mut frames: mpsc::Receiver<MediaEvent>, mut audio_frames: mpsc::Receiver<MediaEvent>) {
        let mut updates_open = true;
        let mut shutdown_ack = None;
        let mut shutdown_deadline = None;
        let mut expire_frames = tokio::time::interval(Duration::from_secs(1));
        expire_frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
                        if matches!(action, Action::Say) {
                            self.say(body, reply);
                        } else {
                            let result = self.api(action, body).await;
                            let _ = reply.send(result);
                        }
                    }
                    Some(Command::SayReady(id, pcm, reply)) => {
                        let result = self.finish_speech(&id, pcm).await;
                        let _ = reply.send(result);
                    }
                    Some(Command::Update(update, reply)) => { let result = self.update(update).await.map(|_| json!({"ok":true})); let _ = reply.send(result); }
                    Some(Command::Media(event)) => self.media_event(event).await,
                    Some(Command::Snapshot(id, reply)) => {
                        let result = self.calls.iter().find(|c| c.id == id && c.state != "ended")
                            .ok_or_else(|| GwError::not_found("Không có cuộc gọi đang diễn ra này."))
                            .and_then(|c| c.snapshot.response());
                        let _ = reply.send(result);
                    }
                    Some(Command::VideoFrames(id, reply)) => {
                        let result = self.calls.iter().find(|c| c.id == id && c.state != "ended")
                            .map(|c| c.snapshot.subscribe())
                            .ok_or_else(|| GwError::not_found("Không có cuộc gọi đang diễn ra này."));
                        let _ = reply.send(result);
                    }
                    Some(Command::VideoJpeg(id, jpeg)) => {
                        if let Some(c) = self.calls.iter_mut().find(|c| c.id == id && c.state != "ended") {
                            c.snapshot.finish(jpeg);
                            c.view["jpeg"] = json!(c.snapshot.sequence);
                        }
                    }
                    Some(Command::CastAudio(id, token, reply)) => {
                        let stream = self.calls.iter().find(|c| c.id == id && c.state != "ended")
                            .and_then(|c| c.cast.as_ref()).and_then(|cast| cast.subscribe(&token));
                        let _ = reply.send(stream);
                    }
                    Some(Command::AudioIn(id, pcm, reply, _permit)) => {
                        let result = self.calls.iter().find(|c| c.id == id && c.state != "ended").ok_or_else(missing)
                            .and_then(|c| {
                                if c.state != "active" || c.devices.mic != "browser" { return Err(GwError::new(409, "CALL_NOT_ACTIVE", "Cuộc gọi chưa nhận hoặc không dùng mic browser.")); }
                                c.browser_mic.as_ref().ok_or_else(media_error)?.push(&pcm)?;
                                Ok(json!({"ok":true,"bytes":pcm.len()}))
                            });
                        let _ = reply.send(result);
                    }
                    Some(Command::VideoIn(id, jpeg, reply)) => {
                        let result = self.calls.iter().find(|c| c.id == id && c.state != "ended").ok_or_else(missing)
                            .and_then(|c| {
                                if c.state != "active" || c.devices.camera != "browser" || c.cam_off { return Err(GwError::new(409, "CALL_NOT_ACTIVE", "Cuộc gọi chưa nhận hoặc không dùng camera browser.")); }
                                c.browser_cam.as_ref().ok_or_else(media_error)?.push(&jpeg)?;
                                Ok(json!({"ok":true,"bytes":jpeg.len()}))
                            });
                        let _ = reply.send(result);
                    }
                    Some(Command::AudioOut(id, token, reply)) => {
                        let result = self.calls.iter().find(|c| c.id == id && c.state != "ended").ok_or_else(missing)
                            .and_then(|c| c.browser_audio.as_ref().ok_or_else(|| GwError::new(403,"MEDIA_TOKEN","Thiếu, sai hoặc hết hạn token âm thanh.")))
                            .and_then(|audio| audio.subscribe(&token));
                        let _ = reply.send(result);
                    }
                    Some(Command::OwnsCall(id, reply)) => { let _ = reply.send(self.calls.iter().any(|c| c.id == id)); }
                    Some(Command::HasActiveCall(reply)) => { let _ = reply.send(self.calls.iter().any(|c| c.state != "ended")); }
                    Some(Command::RestoreMic(id, revision, mic, fmt)) => {
                        self.restore_mic(&id, revision, mic, fmt).await;
                    }
                    Some(Command::RefreshCapture(id)) => {
                        if let Some(i) = self.calls.iter().position(|c| c.id == id && c.state == "active" && c.speech_revision.is_none()) {
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
                Some(frame) = audio_frames.recv() => self.media_event(frame).await,
                _ = expire_frames.tick(), if !self.calls.is_empty() => {
                    let dropped = self.audio_rx_dropped.load(Ordering::Relaxed);
                    for c in &mut self.calls {
                        c.snapshot.expire();
                        if c.state != "ended" {
                            c.stt_diag.sync_drops(dropped, c.browser_audio.as_ref());
                            if let Some(trace) = &mut c.trace {
                                trace.sample(&[c.sig[0], c.sig[1], c.sig[2], c.sig[3], c.frames_in, c.frame_bytes,
                                    c.view["frames"].as_u64().unwrap_or(0), c.view["bad"].as_u64().unwrap_or(0)]);
                                if let Some(line) = c.stt_diag.take_window_line() {
                                    trace.line("stt-1s", &line);
                                }
                            }
                        }
                    }
                }
            }
        }
        for i in 0..self.calls.len() {
            if self.calls[i].state != "ended" {
                let deadline = shutdown_deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(5));
                if let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    let discard_wait = (left / 2).min(Duration::from_secs(2));
                    let _ = tokio::time::timeout(discard_wait, self.discard(i)).await;
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
        if !matches!(action, Action::Calls | Action::Settings | Action::Devices) && !body.is_object() { return Err(GwError::bad_request("Cần JSON object")); }
        match action {
            Action::Calls => Ok(json!({"calls":self.calls.iter().skip(self.calls.len().saturating_sub(10)).map(LiveCall::info).collect::<Vec<_>>()})),
            Action::Resolve => self.resolve_user(&body).await.map(|id|json!(id)),
            Action::Settings => self.settings(body),
            Action::Devices => {
                if !body.is_null() { self.catalog = catalog::save(&self.data_dir, &body)?; }
                Ok(json!({"settings":self.defaults.json(true),"auto":self.defaults.auto(),"catalog":self.catalog}))
            }
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
            Action::Say => unreachable!("Say chuẩn bị âm thanh ngoài actor"),
        }
    }

    fn resolve_catalog_fields(&self, map: &mut serde_json::Map<String, Value>, keys: &[&str]) {
        for &key in keys {
            if let Some(id) = map.get(key).and_then(Value::as_str) {
                if let Some(url) = catalog::resolve(&self.data_dir, id) {
                    map.insert(key.to_owned(), json!(url));
                }
            }
        }
    }

    fn settings(&mut self, mut body: Value) -> Reply {
        if body.is_null() { return Ok(self.defaults.json(true)); }
        // Trình duyệt chỉ gửi MÃ ẨN DANH của thiết bị HA (catalog.rs không trả URL thật ra ngoài) — tra lại URL thật TRƯỚC
        // khi kiểm hợp lệ (update() chỉ chấp nhận test/browser/đường dẫn/URL, không chấp nhận mã ẩn danh dạng hex).
        // Không khớp mã nào (id HA thường, hoặc giá trị "Nhập URL" người tự gõ) thì giữ nguyên giá trị gốc.
        if let Some(map) = body.as_object_mut() {
            self.resolve_catalog_fields(map, &["mic", "camera", "speaker"]);
        }
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
        self.resolve_catalog_fields(&mut overrides, &["mic", "camera", "speaker"]);
        for key in ["mic", "camera"] {
            if overrides.get(key).is_some_and(|v| v.is_null() || v.as_str() == Some("")) {
                overrides.remove(key);
            }
        }
        if let Some(speaker) = overrides.remove("speaker") {
            let speaker = speaker.as_str().ok_or_else(|| GwError::bad_request("speaker phải là chuỗi"))?.trim();
            if !speaker.is_empty() && speaker != "browser" && !valid_speaker_url(speaker) {
                return Err(GwError::bad_request("speaker phải là browser, cửa bộ đàm hoặc cast:// nội bộ"));
            }
            devices.speaker = speaker.to_owned();
        }
        devices.update(&Value::Object(overrides))?;
        if body["playback"] == "shell" { devices.playback = "shell".into(); }
        let video = body["video"].as_bool().unwrap_or(false);
        devices.check_camera(video, false)?;
        let mut call = LiveCall::new(user, true, video, devices, &self.data_dir);
        call.stt_diag = SttDiag::new(self.audio_rx_dropped.load(Ordering::Relaxed));
        call.check_playback(&call.playback_kind, &call.playback_cmd, self.cast_port)?;
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
                        let peer = body["peer"].as_str().filter(|s| !s.trim().is_empty())
                            .map_or_else(|| mask_peer_id(&json!(user)), |s| mask_peer(s.trim()));
                        let _ = self.events.send(json!({"kind":"started","call":self.calls[i].info(),"peer":peer}));
                        // Phản hồi createCall cũng là Call; ready/discard có thể đã tới ở đây.
                        if reply["state"].is_object() { self.update(json!({"@type":"updateCall","call":reply})).await?; }
                        Ok(self.calls[i].info())
                    }
                    Err(e) => { self.fail(i, "createCall không trả mã cuộc gọi").await; Err(e) }
                }
            }
            Err(e) => { self.fail(i, "createCall thất bại").await; Err(e) }
        }
    }

    async fn accept(&mut self, body: Value) -> Reply {
        let i = self.find(&body)?;
        let c = &self.calls[i];
        if c.outgoing || c.accepted || c.ready_done || c.state != "ringing" { return Err(missing()); }
        let mut devices = self.defaults.clone();
        // apply_defaults: chỉ external/file được thừa kế nếu không có speaker.
        devices.check_camera(c.video, c.cam_off)?;
        if !matches!(devices.playback.as_str(), "external" | "file") { devices.playback = "file".into(); }
        let kind = if devices.speaker == "browser" { "browser".into() } else if devices.speaker.is_empty() { devices.playback.clone() } else { "push".into() };
        c.check_playback(&kind, &devices.speaker, self.cast_port)?;
        self.td_request(json!({"@type":"acceptCall","call_id":c.td_id,"protocol":self.protocol_json()})).await?;
        let c = &mut self.calls[i];
        c.accepted = true;
        c.mic_source = devices.mic_source();
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
        let dropped = self.audio_rx_dropped.load(Ordering::Relaxed);
        let c = &mut self.calls[i];
        c.state = "ended";
        c.stt_diag.sync_drops(dropped, c.browser_audio.as_ref());
        if let Some(rev) = c.speech_revision.take() {
            c.finish_speech_metrics(rev);
        } else if let Some(stream) = c.speech_stream.take() {
            c.tts_diag.record(stream.stats());
        }
        c.snapshot = Default::default();
        c.browser_mic = None;
        c.browser_cam = None;
        c.browser_audio = None;
        c.audio_sink = None;
        if let Some(cast) = c.cast.take() { cast.close().await; }
        c.ended = Some(Instant::now());
        if let Some(id) = c.td_id { Self::record_handled(&mut self.handled, &mut self.handled_order, id); }
        c.log(why);
        c.pending_signaling.clear();
        if c.media_created {
            let _ = self.media.execute(MediaOp::Stop(c.user_id)).await;
            c.media_created = false;
        }
        c.playback_final = Some(c.playback_size());
        let _ = fs::remove_file(&c.playback_path);
        let _ = fs::remove_file(format!("{}.wav", c.playback_path.to_string_lossy()));
        if let Ok(entries) = fs::read_dir(self.data_dir.join("live")) {
            let prefix = format!("{}_", c.id);
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let reason = c.td_id.map(|_| sources::mask(why));
        let summary = c.info();
        let _ = self.events.send(json!({"kind":"ended","call":summary,"reason":reason}));
        if let Some(trace) = &mut c.trace { trace.finish(&summary); }
    }

    async fn fail(&mut self, i: usize, why: &str) {
        if let Some(trace) = &self.calls[i].trace { trace.bad(why); }
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
                        self.mark_handled(id); // Cuộc gọi cũ / đã được thiết bị khác xử lý.
                        return Ok(());
                    }
                    if kind != "callStatePending" || call["is_outgoing"] != false { return Ok(()); }
                    let user = call["user_id"].as_i64().filter(|id| *id > 0).ok_or_else(|| GwError::bad_request("updateCall thiếu user_id"))?;
                    if self.calls.iter().any(|c| c.user_id == user && c.state != "ended") { return Ok(()); }
                    self.prune();
                    let mut c = LiveCall::new(user, false, call["is_video"].as_bool().unwrap_or(false), Devices::default(), &self.data_dir);
                    c.td_id = Some(id);
                    c.stt_diag = SttDiag::new(self.audio_rx_dropped.load(Ordering::Relaxed));
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
                    "callStateError" => {
                        self.calls[i].error = sources::mask(state["error"]["message"].as_str().unwrap_or("lỗi TDLib"));
                        if let Some(trace) = &self.calls[i].trace { trace.bad("TDLib: lỗi cuộc gọi"); }
                        self.end(i,"TDLib: lỗi cuộc gọi").await;
                    }
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
                    if let Some(trace) = &self.calls[i].trace { trace.line("goi-rx", &format!("#{} baohieu len={}", self.calls[i].sig[2], bytes.len())); }
                    if self.calls[i].media_created {
                        if self.media.execute(MediaOp::Signaling(self.calls[i].user_id, bytes)).await.is_err() { self.fail(i,"NTgCalls không nhận được báo hiệu").await; return Err(media_error()); }
                    } else if self.calls[i].pending_signaling.len() < MAX_PENDING_SIGNALING {
                        self.calls[i].pending_signaling.push(bytes);
                    }
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
        let (capture, handles) = c.prepare_current_capture(self.profile).await?;
        self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Capture, capture)).await.map_err(|_| media_error())?;
        c.commit_capture(handles);
        let next_cast = if c.browser_audio.is_none() {
            LiveCall::open_cast(&c.playback_kind, &c.playback_cmd).await?
        } else {
            None
        };
        let playback_desc = c.playback_with(&c.playback_kind, &c.playback_cmd, next_cast.as_ref())?;
        self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Playback, playback_desc)).await.map_err(|_| media_error())?;
        c.cast = next_cast;
        if let Some(cast) = &mut c.cast {
            cast.start(cast::target(&c.playback_cmd).unwrap(), self.cast_port.unwrap(), &c.id);
        }
        if c.browser_audio.is_some() && c.playback_kind == "push" && !is_cast_spec(&c.playback_cmd) && c.audio_sink.is_none() {
            match audio_sink::AudioSink::spawn_push(&c.playback_cmd) {
                Ok(sink) => c.audio_sink = Some(sink),
                Err(err) => c.log(&format!("audio_sink: không spawn được ffmpeg loa ({})", err.message)),
            }
        }
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
        let c = &mut self.calls[i];
        if c.media_created {
            let (capture, handles) = c.prepare_current_capture(self.profile).await?;
            self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Capture, capture)).await.map_err(|_| media_error())?;
            c.commit_capture(handles);
        }
        Ok(())
    }

    async fn set_media(&mut self, body: Value) -> Reply {
        let i = self.find(&body)?;
        if self.calls[i].state == "ended" { return Err(missing()); }
        if body["action"] == "audio_out" {
            if self.calls[i].browser_audio.is_none() {
                let mut next_audio = browser_audio::BrowserAudio::new()?;
                // Mới mở lần đầu: LUÔN đổi Playback sang "external" (on_frames) để AI nghe được, BẤT KỂ kind — đo thật 26/11:
                // AI bật thì connect_now bỏ loa nên kind thường là "file", nếu chỉ đổi khi kind=="push" thì tiếng (5MB/cuộc)
                // vẫn chảy vào file, AI điếc. Chỉ khi có loa thật ("push") mới TỰ spawn ffmpeg-loa để tee — xem audio_sink.rs.
                if self.calls[i].media_created {
                    let path = self.calls[i].playback_path.to_string_lossy().into_owned();
                    let desc = playback_description("external", "", &path, self.calls[i].view_on)?;
                    self.media.execute(MediaOp::Sources(self.calls[i].user_id, StreamMode::Playback, desc)).await.map_err(|_| media_error())?;
                    if let Some(cast) = self.calls[i].cast.take() { cast.close().await; }
                    if self.calls[i].playback_kind == "push" && !is_cast_spec(&self.calls[i].playback_cmd) && self.calls[i].audio_sink.is_none() {
                        let url = self.calls[i].playback_cmd.clone();
                        match audio_sink::AudioSink::spawn_push(&url) {
                            Ok(sink) => self.calls[i].audio_sink = Some(sink),
                            Err(err) => self.calls[i].log(&format!("audio_sink: không spawn được ffmpeg loa ({})", err.message)),
                        }
                    }
                }
                let descriptor = next_audio.descriptor(&self.calls[i].id)?;
                self.calls[i].browser_audio = Some(next_audio);
                return Ok(json!({"audio_out": descriptor}));
            }
            let id = self.calls[i].id.clone();
            let audio = self.calls[i].browser_audio.as_mut().unwrap();
            return Ok(json!({"audio_out": audio.descriptor(&id)?}));
        }
        if !["mic","mic_muted","camera","camera_off","video"].iter().any(|k| body.get(*k).is_some()) && !matches!(body["mic_format"].as_str(),Some("auto"|"s16le16k")) {
            return Err(GwError::bad_request("Cần mic, mic_muted, camera, camera_off hoặc video."));
        }
        let c = &self.calls[i];
        let cam_off = body.get("camera_off").map(|v| v.as_bool().ok_or_else(|| GwError::bad_request("camera_off phải là boolean"))).transpose()?;
        let mic_muted = body.get("mic_muted").map(|v| v.as_bool().ok_or_else(|| GwError::bad_request("mic_muted phải là boolean"))).transpose()?;
        let mut devices = c.devices.clone();
        // Chỉ cập nhật capture, không nhận nhầm speaker/playback từ endpoint media.
        let mut capture_map = serde_json::Map::new();
        for key in ["mic","camera","mic_format"] { if let Some(v) = body.get(key) { capture_map.insert(key.to_owned(), v.clone()); } }
        self.resolve_catalog_fields(&mut capture_map, &["mic", "camera"]);
        devices.update(&Value::Object(capture_map))?;
        let mut mic_source = c.mic_source.clone();
        if devices.mic != c.devices.mic && devices.mic != "silence" {
            mic_source = devices.mic_source();
        } else if matches!(body["mic_format"].as_str(), Some("auto" | "s16le16k")) {
            if let Some((_, format)) = &mut mic_source { format.clone_from(&devices.mic_format); }
        }
        if let Some(muted) = mic_muted {
            if muted {
                devices.mic = "silence".into();
                if let Some((_, format)) = &mic_source { devices.mic_format.clone_from(format); }
            } else {
                (devices.mic, devices.mic_format) = mic_source.clone()
                    .ok_or_else(|| GwError::bad_request("Chưa chọn nguồn mic để bật mic."))?;
            }
        }
        if c.speech_revision.is_some() && body.get("mic").is_none() && body.get("mic_muted").is_none() {
            devices.mic_format = c.devices.mic_format.clone();
        }
        let upgrade = body["video"] == true;
        let video = c.video || upgrade;
        let cam_off = !upgrade && cam_off.unwrap_or(c.cam_off);
        devices.check_camera(video, cam_off)?;
        if c.media_created {
            // Mô tả nguồn phải đi qua prepare_capture: mic "browser" cần nối vào nguồn loopback, gọi thẳng devices.capture sẽ ra mic im lặng.
            let (desc, handles) = self.calls[i].prepare_capture(&devices, video, cam_off, self.profile).await?;
            self.media.execute(MediaOp::Sources(self.calls[i].user_id, StreamMode::Capture, desc)).await.map_err(|_| media_error())?;
            if devices.mic != self.calls[i].devices.mic {
                if let Some(rev) = self.calls[i].speech_revision.take() {
                    self.calls[i].finish_speech_metrics(rev);
                }
            }
            self.calls[i].commit_capture(handles);
        }
        let c = &mut self.calls[i];
        if devices.mic != c.devices.mic {
            if let Some(rev) = c.speech_revision.take() {
                c.finish_speech_metrics(rev);
            }
            c.capture_revision += 1;
        } else if devices.mic_format != c.devices.mic_format {
            c.capture_revision += 1;
        }
        c.mic_source = mic_source;
        c.devices = devices;
        c.cam_off = cam_off;
        c.video = video;
        c.log("đổi nguồn gửi đi");
        Ok(c.info())
    }

    async fn set_playback(&mut self, body: Value) -> Reply {
        let i = self.find(&body)?;
        if self.calls[i].state == "ended" { return Err(missing()); }
        let kind = body["kind"].as_str().filter(|s| !s.is_empty()).unwrap_or("file").to_owned();
        if kind == "shell" && body["cmd"].as_str().is_some_and(|s| !s.is_empty()) {
            return Err(GwError::bad_request("Không hỗ trợ lệnh shell tùy ý."));
        }
        // push CHỈ lấy url đã kiểm tra; cmd không thể ghi đè URL như một lệnh shell.
        let cmd = if kind == "push" {
            match body["url"].as_str().filter(|s| !s.is_empty()) {
                Some(url) => catalog::resolve(&self.data_dir, url).unwrap_or_else(|| url.to_owned()),
                None => self.calls[i].devices.speaker.clone(),
            }
        } else {
            String::new()
        };
        let c = &self.calls[i];
        c.check_playback(&kind, &cmd, self.cast_port)?;
        if c.cast.is_some() && kind == c.playback_kind && cmd == c.playback_cmd { return Ok(c.info()); }
        let mut next_cast = None;
        if c.media_created {
            if c.browser_audio.is_none() {
                next_cast = LiveCall::open_cast(&kind, &cmd).await?;
            }
            let desc = c.playback_with(&kind, &cmd, next_cast.as_ref())?;
            self.media.execute(MediaOp::Sources(c.user_id, StreamMode::Playback, desc)).await.map_err(|_| media_error())?;
        }
        let next_sink = if self.calls[i].media_created && self.calls[i].browser_audio.is_some() && kind == "push" && !is_cast_spec(&cmd) {
            Some(audio_sink::AudioSink::spawn_push(&cmd)?)
        } else {
            None
        };
        let c = &mut self.calls[i];
        if let Some(cast) = c.cast.take() { cast.close().await; }
        if let Some(cast) = &mut next_cast {
            cast.start(cast::target(&cmd).unwrap(), self.cast_port.unwrap(), &c.id);
        }
        c.audio_sink = next_sink;
        c.cast = next_cast;
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
                if let Some(trace) = &self.calls[i].trace { trace.line("goi-tx", &format!("#{} baohieu len={}", self.calls[i].sig[0], bytes.len())); }
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
                let dropped = self.audio_rx_dropped.load(Ordering::Relaxed);
                let c = &mut self.calls[i];
                for frame in frames {
                    if matches!(device, StreamDevice::Camera | StreamDevice::Screen) {
                        let w = frame.frame_data.width as usize; let h = frame.frame_data.height as usize;
                        let key = if snapshot::valid(w, h, frame.data.len()) { "frames" } else { "bad" };
                        c.view[key] = json!(c.view[key].as_u64().unwrap_or(0)+1);
                        if key == "frames" {
                            if c.view["frames"] == 1 { c.log(&format!("NTgCalls: khung hình đầu {w}x{h}")); }
                            c.view["size"] = json!(format!("{w}x{h}"));
                            let rotation = rx_rotation(frame.frame_data.rotation);
                            if c.view["frames"] == 1 { c.log(&format!("NTgCalls: cờ xoay khung hình {rotation}°")); }
                            c.view["rotation"] = json!(rotation);
                            c.snapshot.submit(w, h, frame.data, c.id.clone(), self.sender.clone(), rotation);
                        }
                    } else {
                        if let Some(audio) = &c.browser_audio { audio.push(&frame.data); }
                        if let Some(sink) = &c.audio_sink { sink.push(frame.data.clone()); }
                        if c.frames_in == 0 { c.log("NTgCalls: khung tiếng đầu"); }
                        let frame_peak = c.stt_diag.on_frame(&frame.data, c.devices.mic == "silence", c.speech_revision.is_some(), Instant::now());
                        c.frames_in += 1; c.frame_bytes += frame.data.len() as u64;
                        c.peak = c.peak.max(frame_peak);
                        c.stt_diag.sync_drops(dropped, c.browser_audio.as_ref());
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

    fn speech_target(&self, body: &Value) -> Result<usize, GwError> {
        let i = self.find(body)?;
        if self.calls[i].state != "active" { return Err(missing()); }
        if self.calls[i].speech_pending || self.calls[i].speech_revision.is_some() {
            return Err(GwError::new(409, "CALL_BUSY", "Cuộc gọi đang đọc một đoạn tiếng"));
        }
        Ok(i)
    }

    fn say(&mut self, body: Value, reply: oneshot::Sender<Reply>) {
        let (i, raw) = match self.prepare_speech(&body) {
            Ok(input) => input,
            Err(e) => { let _ = reply.send(Err(e)); return; }
        };
        self.calls[i].speech_pending = true;
        let id = self.calls[i].id.clone();
        let tx = self.sender.clone();
        let dir = self.data_dir.join("uploads");
        tokio::spawn(async move {
            let pcm = async {
                let audio = match raw {
                    Some(audio) => audio,
                    None => crate::chat::synthesize_audio(&dir, body["text"].as_str().unwrap().trim(),
                        body["voice"].as_str().unwrap_or(""), body["rate"].as_str().unwrap_or("")).await
                        .map_err(|e| if e.status == 502 { GwError::new(502, "TTS_FAILED", "Không đọc được giọng.") } else { e })?,
                };
                decode_audio(audio).await
            };
            let _ = tx.send(Command::SayReady(id, pcm.await, reply));
        });
    }

    fn prepare_speech(&self, body: &Value) -> Result<(usize, Option<Vec<u8>>), GwError> {
        if !body.is_object() { return Err(GwError::bad_request("Cần JSON object")); }
        let text = body["text"].as_str().unwrap_or("").trim();
        let raw = body["audio_b64"].as_str().filter(|s| !s.is_empty());
        if raw.is_none() && (text.is_empty() || text.chars().count()>1000) { return Err(GwError::bad_request("Cần audio_b64 hoặc text (1-1000 ký tự).")); }
        let i = self.speech_target(body)?;
        let raw = raw.map(|raw| {
            if raw.len()>28_000_000 { return Err(GwError::bad_request("Âm thanh quá lớn.")); }
            STANDARD.decode(raw).map_err(|_| GwError::bad_request("audio_b64 không phải base64 hợp lệ."))
        }).transpose()?;
        Ok((i, raw))
    }

    async fn finish_speech(&mut self, id: &str, pcm: Result<Vec<u8>, GwError>) -> Reply {
        // Cuộc gọi có thể kết thúc trong lúc Edge TTS đang trả âm thanh.
        let i = self.calls.iter().position(|c| c.id == id && c.state == "active").ok_or_else(missing)?;
        self.calls[i].speech_pending = false;
        self.start_speech(i, &pcm?).await
    }

    async fn start_speech(&mut self, i: usize, pcm: &[u8]) -> Reply {
        let seconds = pcm.len() as f64 / 96_000.0;
        let stream = speech::SpeechStream::open(pcm.to_vec()).await?;
        let cmd = stream.command();
        let revision = self.calls[i].capture_revision + 1;
        let (prev_mic, prev_fmt, next_devices, video, cam_off) = {
            let c = &self.calls[i];
            let mut d = c.devices.clone();
            let prev = (c.devices.mic.clone(), c.devices.mic_format.clone());
            d.mic = "speech".into();
            d.mic_format = "s16le48k".into();
            (prev.0, prev.1, d, c.video, c.cam_off)
        };
        let (mut desc, mut handles) = self.calls[i].prepare_capture(&next_devices, video, cam_off, self.profile).await?;
        desc.microphone.as_mut().unwrap().input = cmd;
        handles.speech = Some(stream);
        handles.drop_speech = false;
        self.media.execute(MediaOp::Sources(self.calls[i].user_id, StreamMode::Capture, desc)).await.map_err(|_| media_error())?;
        let c = &mut self.calls[i];
        c.commit_capture(handles);
        c.devices = next_devices;
        c.capture_revision = revision;
        c.speech_revision = Some(revision);
        let id = c.id.clone();
        c.log(&format!("say: phát {:.1}s qua nguồn mic tạm (rev {revision}), trả mic sau {:.1}s", seconds, seconds + SAY_TAIL_S));
        let tx = self.sender.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(seconds + SAY_TAIL_S)).await;
            let _ = tx.send(Command::RestoreMic(id, revision, prev_mic, prev_fmt));
        });
        Ok(json!({"seconds": (seconds * 10.0).round() / 10.0}))
    }

    async fn restore_mic(&mut self, id: &str, revision: u64, mic: String, fmt: String) {
        if let Some(i) = self.calls.iter().position(|c| c.id == id) {
            if self.calls[i].speech_revision == Some(revision) || self.calls[i].capture_revision == revision {
                if let Some(rev) = self.calls[i].speech_revision.take() {
                    self.calls[i].finish_speech_metrics(rev);
                }
            }
            if self.calls[i].state != "ended" && self.calls[i].capture_revision == revision {
                let fmt = self.calls[i].mic_source.as_ref()
                    .filter(|(src, _)| *src == mic)
                    .map(|(_, f)| f.clone())
                    .unwrap_or(fmt);
                self.calls[i].devices.mic = mic;
                self.calls[i].devices.mic_format = fmt;
                if self.capture(i).await.is_err() { self.calls[i].log("không trả được nguồn mic sau say"); }
                else { self.calls[i].log(&format!("say xong: trả nguồn mic (rev {revision})")); }
            }
        }
    }
}

/// Thời gian giữ nguồn mic tạm THÊM sau thời lượng PCM để bù trễ khởi động thật sau khi đã bật `-re`, bỏ `LIVE_IN`
/// và đọc thẳng `.pcm` thô (`-f s16le -ar 48000 -ac 1`): không còn ~1,35 s probe WAV, chỉ còn trễ spawn `sh`+`ffmpeg`
/// và xả đệm NTgCalls (~0,2–0,5 s). Chọn 0,8 s (>= trễ khởi động thật kể cả lúc tải vừa) thay vì 2,0 s để không tạo
/// khoảng lặng dư quá dài sau mỗi lượt nói; HA poll trường `"saying"` nên không lệch nhịp hay dính 409 CALL_BUSY.
const SAY_TAIL_S: f64 = 0.8;

async fn decode_audio(audio: Vec<u8>) -> Result<Vec<u8>, GwError> {
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if audio.is_empty() { return Err(GwError::bad_request("Âm thanh đầu vào trống.")); }
    let audio_len = audio.len();
    let mut child = tokio::process::Command::new("ffmpeg").args(["-hide_banner","-loglevel","error","-nostdin","-i","pipe:0","-vn","-f","s16le","-ar","48000","-ac","1","pipe:1"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn().map_err(|_| GwError::new(501,"AUDIO_UNAVAILABLE","Không chạy được ffmpeg"))?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let err_pipe = child.stderr.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        // ChildStdin::shutdown() không đóng fd/không gửi EOF trên Unix (pipe không có half-close kiểu socket) —
        // PHẢI drop(input) sau khi ghi xong để ffmpeg nhận EOF, không nó treo vô hạn chờ thêm dữ liệu trên pipe:0.
        let write = async move { let res = input.write_all(&audio).await; drop(input); res };
        let read = async move { let mut pcm = Vec::new(); output.take(48_000_001).read_to_end(&mut pcm).await?; Ok::<_,std::io::Error>(pcm) };
        let read_err = async move { let mut buf = Vec::new(); let _ = err_pipe.take(4096).read_to_end(&mut buf).await; String::from_utf8_lossy(&buf).trim().to_owned() };
        let (write_res, read_res, stderr) = tokio::join!(write, read, read_err);
        let pcm = read_res.map_err(|e| { log::warn!("decode_audio: lỗi đọc stdout ffmpeg (input={audio_len}B): {e}"); GwError::bad_request("Lỗi đọc luồng giải mã âm thanh từ ffmpeg.") })?;
        if pcm.len() > 48_000_000 {
            let _ = child.kill().await;
            return Err(GwError::bad_request("Âm thanh quá dài (tối đa 500 giây)."));
        }
        let status = child.wait().await.map_err(|e| { log::warn!("decode_audio: không chờ được ffmpeg (input={audio_len}B): {e}"); GwError::bad_request("Lỗi khi chờ ffmpeg kết thúc.") })?;
        if !status.success() {
            log::warn!("decode_audio: ffmpeg thoát mã {:?} (input={audio_len}B, stderr={stderr:?})", status.code());
            return Err(GwError::bad_request("ffmpeg không giải mã được định dạng âm thanh."));
        }
        if let Err(e) = write_res {
            log::warn!("decode_audio: lỗi ghi stdin ffmpeg (input={audio_len}B): {e}");
            return Err(GwError::bad_request("Lỗi gửi dữ liệu âm thanh tới ffmpeg."));
        }
        if pcm.is_empty() {
            log::warn!("decode_audio: ffmpeg trả PCM rỗng (input={audio_len}B, stderr={stderr:?})");
            return Err(GwError::bad_request("Âm thanh giải mã ra bị trống."));
        }
        Ok::<Vec<u8>, GwError>(pcm)
    }).await;
    match result {
        Ok(inner) => inner,
        Err(_) => { log::warn!("decode_audio: quá thời gian 30 giây khi giải mã âm thanh (input={audio_len}B)"); Err(GwError::new(504, "AUDIO_DECODE_TIMEOUT", "Giải mã âm thanh quá thời gian (30 giây).")) }
    }
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44+pcm.len());
    out.extend_from_slice(b"RIFF"); out.extend_from_slice(&(36+pcm.len() as u32).to_le_bytes()); out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); out.extend_from_slice(&1u16.to_le_bytes()); out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&48000u32.to_le_bytes()); out.extend_from_slice(&96000u32.to_le_bytes()); out.extend_from_slice(&2u16.to_le_bytes()); out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data"); out.extend_from_slice(&(pcm.len() as u32).to_le_bytes()); out.extend_from_slice(pcm); out
}

/// Độ xoay (theo chiều kim đồng hồ) cần áp lên khung hình nhận được: cờ của bên gửi + `TELEGRAM_RX_ROTATE` (0/90/180/270, mặc định 0)
/// để chỉnh khi một thiết bị gửi sai cờ.
fn rx_rotation(flag: ntgcalls::VideoRotation) -> u16 {
    let base = match flag { ntgcalls::VideoRotation::VideoRotation90 => 90, ntgcalls::VideoRotation::VideoRotation180 => 180, ntgcalls::VideoRotation::VideoRotation270 => 270, _ => 0 };
    let extra = std::env::var("TELEGRAM_RX_ROTATE").ok().and_then(|v| v.trim().parse::<u16>().ok()).filter(|v| v % 90 == 0).unwrap_or(0);
    (base + (extra % 360)) % 360
}

#[cfg(test)]
mod tests;
