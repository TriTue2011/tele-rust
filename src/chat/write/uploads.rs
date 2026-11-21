use std::{fs::{self, OpenOptions}, io::{self, Write}, os::unix::fs::OpenOptionsExt, path::{Path, PathBuf},
    process::Stdio, sync::atomic::{AtomicU64, Ordering}, time::{Duration, SystemTime, UNIX_EPOCH}};

use base64::{engine::{general_purpose::GeneralPurpose, DecodePaddingMode, GeneralPurposeConfig}, alphabet, Engine};
use serde_json::Value;
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};

use crate::contract::GwError;

pub(super) const MAX_UPLOAD: usize = 64 * 1024 * 1024;
const TRANSCODE_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) fn decode(value: &Value, field: &str) -> Result<Vec<u8>, GwError> {
    // Python validate=True accepts unused nonzero tail bits but requires standard padding.
    const BASE64: GeneralPurpose = GeneralPurpose::new(&alphabet::STANDARD, GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical).with_decode_allow_trailing_bits(true));
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => BASE64.decode(s).map_err(|_| GwError::bad_request(format!("{field} không phải base64 hợp lệ."))),
        _ => Err(GwError::bad_request(format!("{field} không phải base64 hợp lệ."))),
    }
}

pub(super) struct Upload(Option<PathBuf>);

impl Upload {
    pub(super) async fn save(dir: &Path, name: &str, raw: Vec<u8>) -> Result<Self, GwError> {
        let dir = dir.to_owned();
        // Only the basename is used, and the server always adds its own prefix.
        let name = name.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("tep").to_owned();
        tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&dir)?;
            let now = SystemTime::now();
            let stamp = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let path = dir.join(format!("chat-{}-{stamp:x}-{:x}_{name}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
            let mut handle = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
            let file = Self(Some(path));
            handle.write_all(&raw)?;
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().starts_with("chat-") && entry.metadata().ok()
                        .and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok())
                        .is_some_and(|age| age > Duration::from_secs(24 * 3600)) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
            Ok::<_, io::Error>(file)
        }).await.ok().and_then(Result::ok).ok_or_else(|| GwError::new(500, "UPLOAD_STORAGE", "Không lưu được tệp tải lên."))
    }

    pub(super) fn path(&self) -> &Path { self.0.as_deref().unwrap() }
    pub(super) fn keep(mut self) { self.0.take(); }
}

impl Drop for Upload {
    fn drop(&mut self) {
        if let Some(path) = &self.0 { let _ = fs::remove_file(path); }
    }
}

pub(super) async fn voice_ogg(raw: Vec<u8>) -> Result<Vec<u8>, GwError> {
    if raw.starts_with(b"OggS") { return Ok(raw); }
    let mut child = Command::new("ffmpeg").args(["-hide_banner","-loglevel","error","-nostdin","-i","pipe:0",
        "-vn","-t","600","-ac","1","-ar","48000","-c:a","libopus","-b:a","32k","-application","voip","-f","ogg","pipe:1"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn()
        .map_err(|_| GwError::bad_request("máy chủ chưa có ffmpeg nên không đổi được âm thanh sang ogg/opus"))?;
    let mut stdin = child.stdin.take().unwrap();
    let (_, out) = timeout(TRANSCODE_TIMEOUT, async move {
        tokio::join!(async { let result = stdin.write_all(&raw).await; drop(stdin); result }, child.wait_with_output())
    }).await.map_err(|_| GwError::bad_request("đổi âm thanh quá 60 giây"))?;
    if let Ok(out) = out {
        if out.status.success() && !out.stdout.is_empty() { return Ok(out.stdout); }
        let err = String::from_utf8_lossy(&out.stderr);
        let last: String = err.trim().lines().last().unwrap_or("không rõ").chars().take(160).collect();
        return Err(GwError::bad_request(format!("ffmpeg không đọc được tệp âm thanh: {last}")));
    }
    Err(GwError::bad_request("ffmpeg không đọc được tệp âm thanh: không rõ"))
}

pub(super) async fn synthesize(dir: &Path, text: &str, voice: &str, rate: &str) -> Result<Vec<u8>, GwError> {
    let raw = synthesize_audio(dir, text, voice, rate).await.map_err(|e| {
        if e.code == "TTS_UNAVAILABLE" { media_error(&e.message) } else { e }
    })?;
    voice_ogg(raw).await.map_err(|e| media_error(&e.message))
}

pub(crate) async fn synthesize_audio(dir: &Path, text: &str, voice: &str, rate: &str) -> Result<Vec<u8>, GwError> {
    let (voice, rate) = crate::config::tts(voice, rate).map_err(GwError::bad_request)?;
    let input = Upload::save(dir, "tts.txt", text.as_bytes().to_vec()).await?;
    let audio = Upload::save(dir, "tts.mp3", Vec::new()).await?;
    for _ in 0..3 {
        let mut cmd = Command::new("edge-tts");
        cmd.args([format!("--voice={voice}"), format!("--rate={rate}")]).arg("--file").arg(input.path())
            .arg("--write-media").arg(audio.path()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);
        let result = timeout(TRANSCODE_TIMEOUT, cmd.output()).await
            .map_err(|_| media_error("tổng hợp giọng quá 60 giây"))?
            .map_err(|e| if e.kind() == io::ErrorKind::NotFound {
                GwError::new(501, "TTS_UNAVAILABLE", "Gateway chưa có bộ đọc giọng (TTS).")
            } else { media_error("Không chạy được bộ tổng hợp giọng.") })?;
        if result.status.success() {
            let path = audio.path().to_owned();
            let data = tokio::task::spawn_blocking(move || fs::read(path)).await.ok().and_then(Result::ok)
                .ok_or_else(|| media_error("không đọc được âm thanh TTS"))?;
            if data.is_empty() { return Err(media_error("bộ tổng hợp giọng không trả âm thanh")); }
            return Ok(data);
        }
        if !String::from_utf8_lossy(&result.stderr).contains("NoAudioReceived") { break; }
    }
    Err(media_error("tổng hợp giọng thất bại"))
}

fn media_error(message: &str) -> GwError { GwError::new(502, "MEDIA_ENGINE_ERROR", message) }
