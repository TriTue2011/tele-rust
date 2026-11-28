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

use tokio::{io::AsyncWriteExt, sync::{mpsc, watch}};

use super::sources::speaker_args;
use crate::contract::GwError;

pub(super) struct AudioSink {
    tx: mpsc::Sender<Vec<u8>>,
    _stop: watch::Sender<()>,
}

impl AudioSink {
    /// Spawn ffmpeg CHO ĐÚNG url loa (giống hệt lệnh NTgCalls lẽ ra tự chạy), ghi tiếng vào stdin của nó qua một
    /// tác vụ nền riêng (không bao giờ await ghi I/O trong vòng xử lý sự kiện chính — tránh làm nghẽn signaling).
    pub(super) fn spawn_push(url: &str) -> Result<Self, GwError> {
        let args = speaker_args(url)?;
        let mut cmd = tokio::process::Command::new("ffmpeg");
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        Self::spawn_command_inner(cmd).map(|(sink, _)| sink)
    }

    fn spawn_command_inner(mut cmd: tokio::process::Command) -> Result<(Self, Option<u32>), GwError> {
        let mut child = cmd.spawn().map_err(|_| GwError::new(501, "AUDIO_UNAVAILABLE", "Không chạy được ffmpeg cho loa"))?;
        let pid = child.id();
        let mut stdin = child.stdin.take().ok_or_else(|| GwError::new(500, "AUDIO_UNAVAILABLE", "Không mở được stdin ffmpeg"))?;
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
        let (stop_tx, mut stop_rx) = watch::channel(());
        tokio::spawn(async move {
            loop {
                let data = tokio::select! {
                    biased;
                    _ = stop_rx.changed() => break,
                    item = rx.recv() => match item {
                        Some(data) => data,
                        None => break,
                    },
                };
                let write_ok = tokio::select! {
                    biased;
                    _ = stop_rx.changed() => false,
                    res = stdin.write_all(&data) => res.is_ok(),
                };
                if !write_ok { break; }
            }
            drop(stdin);
            let _ = child.kill().await;
        });
        Ok((Self { tx, _stop: stop_tx }, pid))
    }

    /// Không chặn: mất khung thà hơn kẹt actor xử lý sự kiện cuộc gọi.
    pub(super) fn push(&self, data: Vec<u8>) {
        let _ = self.tx.try_send(data);
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};
    use super::*;

    #[tokio::test]
    async fn dropping_sink_stops_and_reaps_child_even_with_blocked_stdin() {
        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let (sink, pid) = AudioSink::spawn_command_inner(cmd).unwrap();
        let pid = pid.unwrap();
        for _ in 0..8 {
            sink.push(vec![0u8; 65_536]);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(sink);
        let proc_path = PathBuf::from(format!("/proc/{pid}"));
        let reaped = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !proc_path.exists() { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await;
        assert!(reaped.is_ok(), "child {pid} did not exit and get reaped within 1s");
    }
}
