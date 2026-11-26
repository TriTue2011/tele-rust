//! Tee tiếng nhận được (Playback) ra loa vật lý KHI AI cũng đang nghe cùng cuộc gọi.
//!
//! NTgCalls tự quản lý "push" (kind=push, MediaSource::Shell): nó tự spawn ffmpeg và bơm PCM trực tiếp vào stdin,
//! KHÔNG đi qua callback on_frames của Rust — nên khi AI cũng cần nghe (qua on_frames -> browser_audio), hai nhu
//! cầu (loa vật lý + AI nghe) tranh nhau MỘT nguồn và loa luôn thắng, AI "điếc" hoàn toàn (đo thật 26/11: STT báo
//! "stt-no-text-recognized" ở MỌI cuộc gọi dùng loa vật lý, dù NTgCalls nhận bitrate audio thật).
//!
//! Sửa: khi AI cần nghe, đổi MediaDescription Playback sang "external" để NTgCalls giao khung tiếng cho on_frames,
//! rồi TỰ spawn đúng tiến trình ffmpeg mà NTgCalls lẽ ra tự chạy, ghi khung tiếng vào stdin của nó. Tiếng vào
//! browser_audio (AI) và vào đây (loa) từ CÙNG một nguồn — giống hệt cách zalo-rs tee bằng broadcast::channel
//! (src/calls/audio.rs: một kênh, nhiều người nghe), chỉ khác cơ chế do Tele dùng NTgCalls thay vì engine riêng.
use std::process::Stdio;

use tokio::{io::AsyncWriteExt, sync::mpsc};

use super::sources::speaker_args;
use crate::contract::GwError;

pub(super) struct AudioSink {
    tx: mpsc::Sender<Vec<u8>>,
}

impl AudioSink {
    /// Spawn ffmpeg CHO ĐÚNG url loa (giống hệt lệnh NTgCalls lẽ ra tự chạy), ghi tiếng vào stdin của nó qua một
    /// tác vụ nền riêng (không bao giờ await ghi I/O trong vòng xử lý sự kiện chính — tránh làm nghẽn signaling).
    pub(super) fn spawn_push(url: &str) -> Result<Self, GwError> {
        let args = speaker_args(url)?;
        let mut child = tokio::process::Command::new("ffmpeg").args(&args)
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
            .kill_on_drop(true).spawn().map_err(|_| GwError::new(501, "AUDIO_UNAVAILABLE", "Không chạy được ffmpeg cho loa"))?;
        let mut stdin = child.stdin.take().ok_or_else(|| GwError::new(500, "AUDIO_UNAVAILABLE", "Không mở được stdin ffmpeg"))?;
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        tokio::spawn(async move {
            let _child = child; // giữ tiến trình sống tới khi kênh đóng; kill_on_drop tự dọn khi tác vụ này thoát
            while let Some(data) = rx.recv().await {
                if stdin.write_all(&data).await.is_err() { break; }
            }
        });
        Ok(Self { tx })
    }

    /// Không chặn: mất khung thà hơn kẹt actor xử lý sự kiện cuộc gọi.
    pub(super) fn push(&self, data: Vec<u8>) {
        let _ = self.tx.try_send(data);
    }
}
