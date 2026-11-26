use std::{future::Future, pin::Pin};

use base64::{engine::general_purpose::STANDARD, Engine};
use ntgcalls::{ConnectionState, Frame, MediaDescription, NTgCalls, Protocol, RTCServer, StreamDevice, StreamMode, StreamStatus};
use serde_json::Value;
use tokio::sync::mpsc;

use super::Command;

pub(super) type MediaFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + 'a>>;

#[derive(Debug, Clone)]
pub(super) struct Ready {
    pub servers: Vec<RTCServer>,
    pub versions: Vec<String>,
    pub key: Vec<u8>,
    pub p2p: bool,
    pub custom: Option<String>,
}

impl Ready {
    pub fn parse(value: &Value, p2p: bool) -> Result<Self, String> {
        let decode = |v: &Value| STANDARD.decode(v.as_str().unwrap_or("")).map_err(|_| "Trường bytes TDLib không phải base64".to_owned());
        let text = |v: &Value| -> Result<String, String> {
            let s = v.as_str().unwrap_or("");
            if s.contains('\0') { return Err("Trường TDLib chứa NUL".into()); }
            Ok(s.to_owned())
        };
        let mut servers = Vec::new();
        for s in value["servers"].as_array().into_iter().flatten() {
            let t = &s["type"];
            servers.push(RTCServer {
                // int64 của TDLib trong JSON có thể là chuỗi (id reflector).
                id: s["id"].as_u64().or_else(|| s["id"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0),
                ipv4: text(&s["ip_address"])?, ipv6: text(&s["ipv6_address"])?,
                port: s["port"].as_u64().and_then(|p| p.try_into().ok()).ok_or("Cổng RTC không hợp lệ")?,
                username: text(&t["username"])?, password: text(&t["password"])?,
                turn: t["supports_turn"].as_bool().unwrap_or(false), stun: t["supports_stun"].as_bool().unwrap_or(false),
                tcp: t["is_tcp"].as_bool().unwrap_or(false), peer_tag: decode(&t["peer_tag"])?,
            });
        }
        let versions = value["protocol"]["library_versions"].as_array().into_iter().flatten().map(&text).collect::<Result<_, _>>()?;
        let custom = text(&value["custom_parameters"])?;
        Ok(Self { servers, versions, key: decode(&value["encryption_key"])?, p2p: p2p && value["allow_p2p"].as_bool().unwrap_or(false), custom: (!custom.is_empty()).then_some(custom) })
    }
}

#[derive(Debug, Clone)]
pub(super) enum MediaOp {
    Create(i64),
    Sources(i64, StreamMode, MediaDescription),
    Skip(i64, Vec<u8>, bool),
    Connect(i64, Ready),
    Signaling(i64, Vec<u8>),
    Stop(i64),
}

pub(super) enum MediaEvent {
    Signaling(i64, Vec<u8>),
    Connection(i64, ConnectionState),
    Frames(i64, StreamMode, StreamDevice, Vec<Frame>),
    PeerVideo(i64, bool),
}

pub(super) fn dispatch_playback_frames(
    id: i64,
    mode: StreamMode,
    device: StreamDevice,
    frames: Vec<Frame>,
    frame_sender: &mpsc::Sender<MediaEvent>,
    audio_sender: &mpsc::Sender<MediaEvent>,
) {
    if mode == StreamMode::Playback {
        // Tách kênh tiếng khỏi kênh hình: khung video lớn/dồn dập (kênh 3) không thể làm rớt khung tiếng.
        // Cả hai đều dùng try_send: không chặn luồng NTgCalls hoặc dồn media vào RAM khi TDLib/ffmpeg chậm.
        let tx = if matches!(device, StreamDevice::Camera | StreamDevice::Screen) { frame_sender } else { audio_sender };
        let _ = tx.try_send(MediaEvent::Frames(id, mode, device, frames));
    }
}

// Futures cố ý không Send: NTgCalls chỉ Send, không Sync. Toàn bộ Media ở một thread.
pub(super) trait Media: Send + 'static {
    fn protocol(&self) -> Result<Protocol, String>;
    fn callbacks(&mut self, sender: mpsc::UnboundedSender<Command>, frames: mpsc::Sender<MediaEvent>, audio: mpsc::Sender<MediaEvent>);
    fn execute(&mut self, op: MediaOp) -> MediaFuture<'_>;
}

pub(super) struct NativeMedia(pub NTgCalls);

impl Media for NativeMedia {
    fn protocol(&self) -> Result<Protocol, String> {
        NTgCalls::get_protocol().map_err(|_| "Không đọc được protocol NTgCalls".into())
    }

    fn callbacks(&mut self, sender: mpsc::UnboundedSender<Command>, frame_sender: mpsc::Sender<MediaEvent>, audio_sender: mpsc::Sender<MediaEvent>) {
        let tx = sender.clone();
        self.0.on_signaling_data(move |id, data| { let _ = tx.send(Command::Media(MediaEvent::Signaling(id, data))); });
        let tx = sender.clone();
        self.0.on_connection_change(move |id, info| { let _ = tx.send(Command::Media(MediaEvent::Connection(id, info.state))); });
        self.0.on_frames(move |id, mode, device, frames| {
            dispatch_playback_frames(id, mode, device, frames, &frame_sender, &audio_sender);
        });
        let tx = sender.clone();
        self.0.on_remote_source_change(move |id, src| {
            if src.device == StreamDevice::Camera { let _ = tx.send(Command::Media(MediaEvent::PeerVideo(id, src.state == StreamStatus::Active))); }
        });
        self.0.on_upgrade(move |id, state| { let _ = sender.send(Command::Media(MediaEvent::PeerVideo(id, !state.video_stopped && !state.video_paused))); });
    }

    fn execute(&mut self, op: MediaOp) -> MediaFuture<'_> {
        Box::pin(async move {
            let result = match op {
                MediaOp::Create(id) => self.0.create_p2p_call(id).await,
                MediaOp::Sources(id, mode, desc) => self.0.set_stream_sources(id, mode, &desc).await,
                MediaOp::Skip(id, key, outgoing) => self.0.skip_exchange(id, &key, outgoing).await,
                MediaOp::Connect(id, ready) => self.0.connect_p2p(id, &ready.servers, &ready.versions, ready.p2p, ready.custom.as_deref()).await,
                MediaOp::Signaling(id, data) => self.0.send_signaling_data(id, &data).await,
                MediaOp::Stop(id) => self.0.stop(id).await,
            };
            // Lỗi binding có thể chứa command/URL có khóa: KHÔNG công bố chuỗi lỗi gốc, chỉ tên LOẠI lỗi
            // (enum cố định như ShellError/ConnectionError) để chẩn đoán được mà không lộ bí mật.
            result.map_err(|e| {
                log::error!("NTgCalls: thao tác lỗi, loại {:?}", e.kind);
                "Thao tác NTgCalls thất bại".into()
            })
        })
    }
}
