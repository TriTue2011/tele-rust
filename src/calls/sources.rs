//! Dựng lệnh cố định; nguồn bên ngoài luôn là đúng một đối số shell.
use std::sync::OnceLock;

use ntgcalls::{AudioDescription, MediaDescription, MediaSource, VideoDescription};
use regex::Regex;
use serde_json::{json, Value};

use crate::contract::GwError;

pub(super) const SAMPLE_RATE: u32 = 48_000;

fn speaker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\Ahttps?://[A-Za-z0-9.\-]+(?::\d{1,5})?/api/(?:r1_speaker|dahua_talk)/(intercom|duplex)/[A-Za-z0-9_\-]+\?k=[A-Za-z0-9_\-%.~]+\z").unwrap())
}

pub fn is_cast_spec(spec: &str) -> bool {
    super::cast::target(spec).is_some()
}

pub fn valid_speaker_url(url: &str) -> bool {
    speaker_re().is_match(url) || is_cast_spec(url)
}

pub(super) fn quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b)) {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\"'\"'"))
    }
}

/// Cờ giảm độ trễ cho NGUỒN TRỰC TIẾP (mic loa R1 s16le16k, RTSP, HTTP). ĐO 21/11/2026 với luồng PCM 16 kHz phát đúng thời gian thực:
/// thiếu cờ này ffmpeg dồn ~3,2 s trước khi xuất rồi giữ nguyên khoảng trễ suốt cuộc gọi (mic đến người nghe chậm ≈ 3 s); có cờ ≈ 64 ms.
/// Mức VỪA (probesize 64 KiB, analyzeduration 0,5 s) thay vì `-probesize 32 -analyzeduration 0`: đo WAV cho thấy mức "mạnh" còn chậm hơn (1,98 s so với 1,35 s)
/// và có thể làm ffmpeg không nhận ra định dạng của luồng không phải PCM thô.
pub(super) const LIVE_IN: &str = "-fflags nobuffer -flags low_delay -probesize 65536 -analyzeduration 500000";

pub fn mic_command(spec: &str, fmt: &str) -> String {
    let base = "ffmpeg -hide_banner -loglevel error -nostdin";
    let out = "-f s16le -ac 1 -ar 48000 pipe:1";
    match spec {
        "silence" | "browser" => format!("{base} -re -f lavfi -i \"anullsrc=r=48000:cl=mono\" {out}"),
        "" | "test" => format!("{base} -re -f lavfi -i \"sine=frequency=440:sample_rate=48000\" -af volume=0.25 {out}"),
        _ if fmt == "s16le16k" => format!("{base} {LIVE_IN} -f s16le -ar 16000 -ac 1 -i {} {out}", quote(spec)),
        _ if spec.starts_with("rtsp://") || spec.starts_with("rtsps://") => format!("{base} {LIVE_IN} -rtsp_transport tcp -i {} -vn {out}", quote(spec)),
        _ => format!("{base} {LIVE_IN} -i {} -vn {out}", quote(spec)),
    }
}

pub fn speaker_command(url: &str) -> Result<String, GwError> {
    let m = speaker_re().captures(url).ok_or_else(|| GwError::bad_request("speaker không phải cửa bộ đàm r1_speaker / dahua_talk"))?;
    let out = if &m[1] == "duplex" { "-ac 1 -ar 16000 -c:a pcm_s16le -f s16le" }
        else { "-ac 1 -ar 8000 -c:a pcm_alaw -f alaw" };
    Ok(format!("ffmpeg -hide_banner -loglevel error -fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 -f s16le -ar 48000 -ac 1 -i pipe:0 {out} -flush_packets 1 -method POST -chunked_post 1 -content_type application/octet-stream {}", quote(url)))
}

pub(super) fn video_profile(value: &str) -> (i16, i16, u8) {
    if value.trim().eq_ignore_ascii_case("low") { (480, 270, 10) } else { (640, 360, 15) }
}

pub fn camera_command(spec: &str, profile: (i16, i16, u8)) -> String {
    let (w, h, fps) = profile;
    let base = "ffmpeg -hide_banner -loglevel error -nostdin";
    let out = format!("-vf scale={w}:{h},fps={fps} -f rawvideo -pix_fmt yuv420p pipe:1");
    match spec {
        "black" | "browser" => format!("{base} -re -f lavfi -i \"color=c=black:size={w}x{h}:rate={fps}\" {out}"),
        "" | "test" => format!("{base} -re -f lavfi -i \"testsrc2=size={w}x{h}:rate={fps}\" {out}"),
        _ if spec.starts_with("rtsp://") || spec.starts_with("rtsps://") => format!("{base} {LIVE_IN} -rtsp_transport tcp -i {} -an {out}", quote(spec)),
        _ => format!("{base} {LIVE_IN} -re -i {} -an {out}", quote(spec)),
    }
}

pub(super) fn mask(value: &str) -> String {
    static PASSWORD: OnceLock<Regex> = OnceLock::new();
    static KEY: OnceLock<Regex> = OnceLock::new();
    let password = PASSWORD.get_or_init(|| Regex::new(r"(://[^/\s:@]+:)[^@/\s]*@").unwrap());
    let key = KEY.get_or_init(|| Regex::new(r"([?&]k=)[^&\s]+").unwrap());
    key.replace_all(&password.replace_all(value, "${1}***@"), "${1}***").into_owned()
}

pub(super) fn source(value: &Value, key: &str) -> Result<String, GwError> {
    let s = value.as_str().ok_or_else(|| GwError::bad_request(format!("{key} phải là chuỗi")))?.trim();
    let s = if s.is_empty() { "test" } else { s };
    let special = matches!(s, "test" | "browser") || (key == "mic" && s == "silence") || (key == "camera" && s == "black");
    if s.contains('\0') || !(special || s.starts_with('/') || s.starts_with("./") || ["rtsp://", "rtsps://", "http://", "https://", "rtmp://"].iter().any(|p| s.starts_with(p))) {
        return Err(GwError::bad_request(format!("{key} phải là nguồn thử, URL rtsp/http hoặc đường dẫn tệp")));
    }
    Ok(s.to_owned())
}

#[derive(Clone)]
pub(super) struct Devices {
    pub mic: String,
    pub mic_format: String,
    pub camera: String,
    pub playback: String,
    pub speaker: String,
}

impl Default for Devices {
    fn default() -> Self {
        Self { mic: "test".into(), mic_format: "auto".into(), camera: "test".into(), playback: "file".into(), speaker: String::new() }
    }
}

impl Devices {
    pub fn mic_source(&self) -> Option<(String, String)> {
        (self.mic != "silence").then(|| (self.mic.clone(), self.mic_format.clone()))
    }

    pub fn json(&self, masked: bool) -> Value {
        let show = |s: &str| if masked { mask(s) } else { s.to_owned() };
        let mut value = json!({"mic":show(&self.mic), "mic_format":self.mic_format, "camera":show(&self.camera), "playback":self.playback, "speaker":show(&self.speaker)});
        if masked { value["auto"] = json!(self.auto()); }
        value
    }

    pub fn auto(&self) -> bool {
        // Các nguồn dựng sẵn (kể cả test của bản cũ) và browser không phải thiết bị do HA cấp.
        !matches!(self.mic.as_str(), "" | "test" | "silence" | "browser")
            || !matches!(self.camera.as_str(), "" | "test" | "black" | "browser")
            || !matches!(self.speaker.as_str(), "" | "browser")
    }

    pub fn check_camera(&self, video: bool, off: bool) -> Result<(), GwError> {
        if video && !off && self.camera == "browser" {
            return Err(GwError::new(501, "BROWSER_VIDEO_UNAVAILABLE", "Camera trình duyệt chưa được hỗ trợ; chọn black hoặc camera HA."));
        }
        Ok(())
    }

    pub fn update(&mut self, body: &Value) -> Result<(), GwError> {
        if !body.is_object() { return Err(GwError::bad_request("Cần JSON object")); }
        for (key, target) in [("mic", &mut self.mic), ("camera", &mut self.camera)] {
            if let Some(value) = body.get(key) {
                let value = source(value, key)?;
                if !value.contains("***@") && !value.contains("k=***") { *target = value; }
            }
        }
        if let Some(value) = body.get("speaker") {
            let value = value.as_str().ok_or_else(|| GwError::bad_request("speaker phải là chuỗi"))?.trim();
            if !value.contains("k=***") {
                if !value.is_empty() && value != "browser" && !valid_speaker_url(value) { return Err(GwError::bad_request("speaker phải là browser, cửa bộ đàm hoặc cast:// nội bộ")); }
                self.speaker = value.to_owned();
            }
        }
        if let Some(value) = body.get("mic_format") {
            self.mic_format = value.as_str().filter(|v| matches!(*v, "auto" | "s16le16k"))
                .ok_or_else(|| GwError::bad_request("mic_format phải là auto hoặc s16le16k"))?.into();
        }
        if matches!(body["playback"].as_str(), Some("external" | "file")) { self.playback = body["playback"].as_str().unwrap().into(); }
        Ok(())
    }

    pub fn capture(&self, video: bool, cam_off: bool, profile: (i16, i16, u8)) -> MediaDescription {
        MediaDescription {
            microphone: Some(audio(MediaSource::Shell, mic_command(&self.mic, &self.mic_format), true)),
            speaker: None,
            camera: (video && !cam_off).then(|| VideoDescription { media_source: MediaSource::Shell, width: profile.0, height: profile.1, fps: profile.2, input: camera_command(&self.camera, profile), keep_open: true }),
            screen: None,
        }
    }
}

fn audio(media_source: MediaSource, input: String, keep_open: bool) -> AudioDescription {
    AudioDescription { media_source, sample_rate: SAMPLE_RATE, channel_count: 1, input, keep_open }
}

pub(super) fn playback_description(kind: &str, cmd: &str, path: &str, video: bool) -> Result<MediaDescription, GwError> {
    let (width, height, fps) = crate::config::rx_video(&std::env::var("TELEGRAM_RX_VIDEO").unwrap_or_else(|_| "640x360@15".into()))
        .map_err(GwError::bad_request)?;
    let (src, input) = match kind {
        "external" => (MediaSource::External, String::new()),
        "file" => (MediaSource::File, path.to_owned()),
        "shell" => (MediaSource::Shell, if cmd.is_empty() { format!("ffmpeg -hide_banner -loglevel error -f s16le -ar 48000 -ac 1 -i pipe:0 -f wav {}", quote(&format!("{path}.wav"))) } else { cmd.to_owned() }),
        "push" => (MediaSource::Shell, speaker_command(cmd)?),
        _ => return Err(GwError::bad_request("kind phải là external, file, shell hoặc push")),
    };
    if input.contains('\0') { return Err(GwError::bad_request("Nguồn chứa ký tự NUL")); }
    Ok(MediaDescription {
        // NTgCalls chỉ bật audio incoming khi PLAYBACK có writer Microphone.
        microphone: Some(audio(src, input, false)),
        speaker: None,
        camera: video.then(|| VideoDescription { media_source: MediaSource::External, width, height, fps, input: String::new(), keep_open: false }),
        screen: None,
    })
}
