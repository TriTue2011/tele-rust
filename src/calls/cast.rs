//! Luồng MP3 riêng từng cuộc gọi dành cho thiết bị Google Cast.
mod session;

use std::{collections::VecDeque, io, net::Ipv4Addr, sync::{Arc, Mutex}, time::Duration};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use subtle::ConstantTimeEq;
use tokio::{io::AsyncReadExt, net::TcpListener, sync::{broadcast, oneshot}, task::JoinHandle};

use crate::contract::CastAudio;

const PRIME_BYTES: usize = 16_000;
const MAX_LISTENERS: usize = 8;

pub(super) enum Target { Ip(Ipv4Addr), Id(String) }

pub(super) fn local_cast_ip(ip: Ipv4Addr) -> bool { ip.is_private() }

pub(super) fn target(spec: &str) -> Option<Target> {
    let host = spec.trim().strip_prefix("cast://")?;
    if let Some(id) = host.strip_prefix("id/") {
        return (id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .then(|| Target::Id(id.to_owned()));
    }
    host.parse().ok().filter(|ip| local_cast_ip(*ip)).map(Target::Ip)
}

struct Audio {
    recent: VecDeque<u8>,
    outgoing: broadcast::Sender<Vec<u8>>,
}

pub(super) struct CastSink {
    port: u16,
    token: String,
    audio: Arc<Mutex<Audio>>,
    reader: JoinHandle<()>,
    control: Option<(oneshot::Sender<()>, JoinHandle<()>)>,
}

impl CastSink {
    pub(super) async fn open() -> io::Result<Self> {
        let mut raw = [0u8; 24];
        rustls::crypto::ring::default_provider().secure_random.fill(&mut raw)
            .map_err(|_| io::Error::other("Không tạo được token Cast."))?;
        let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = local.local_addr()?.port();
        let audio = Arc::new(Mutex::new(Audio { recent: VecDeque::new(), outgoing: broadcast::channel(256).0 }));
        let output = audio.clone();
        let reader = tokio::spawn(async move {
            let mut chunk = [0; 4096];
            while let Ok((mut stream, _)) = local.accept().await {
                // NTgCalls có thể mở lại ffmpeg sau khi đổi cấu hình nguồn.
                while let Ok(size) = stream.read(&mut chunk).await {
                    if size == 0 { break; }
                    let mut audio = output.lock().unwrap();
                    audio.recent.extend(&chunk[..size]);
                    let extra = audio.recent.len().saturating_sub(PRIME_BYTES);
                    audio.recent.drain(..extra);
                    let _ = audio.outgoing.send(chunk[..size].to_vec());
                }
            }
        });
        Ok(Self { port, token: URL_SAFE_NO_PAD.encode(raw), audio, reader, control: None })
    }

    pub(super) fn start(&mut self, target: Target, rest_port: u16, id: &str) {
        let (stop, stopped) = oneshot::channel();
        let path = format!("/api/live/cast/{id}/{}.mp3", self.token);
        let task = tokio::spawn(async move {
            if session::run(target, rest_port, path, stopped).await.is_err() {
                eprintln!("Không kết nối được loa Cast hoặc phiên phát đã ngắt.");
            }
        });
        self.control = Some((stop, task));
    }

    pub(super) fn encoder_command(&self) -> String {
        format!("ffmpeg -hide_banner -loglevel error -fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 \
            -f s16le -ar 48000 -ac 1 -i pipe:0 -c:a libmp3lame -b:a 128k -flush_packets 1 -f mp3 tcp://127.0.0.1:{}", self.port)
    }

    pub(super) fn subscribe(&self, token: &str) -> Option<CastAudio> {
        if !bool::from(self.token.as_bytes().ct_eq(token.as_bytes())) { return None; }
        let audio = self.audio.lock().unwrap();
        if audio.outgoing.receiver_count() >= MAX_LISTENERS { return None; }
        Some(CastAudio { prime: audio.recent.iter().copied().collect(), audio: audio.outgoing.subscribe() })
    }

    pub(super) async fn close(mut self) {
        self.reader.abort();
        if let Some((stop, mut control)) = self.control.take() {
            let _ = stop.send(());
            if tokio::time::timeout(Duration::from_millis(750), &mut control).await.is_err() { control.abort(); }
        }
    }
}

impl Drop for CastSink {
    fn drop(&mut self) {
        self.reader.abort();
        if let Some((stop, _)) = self.control.take() { let _ = stop.send(()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn cast_stream_authenticates_caps_prime_and_closes() {
        let sink = CastSink::open().await.unwrap();
        assert!(sink.subscribe("wrong").is_none());
        let mut stream = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, sink.port)).await.unwrap();
        let mut first = sink.subscribe(&sink.token).unwrap();
        let bytes = vec![6; PRIME_BYTES * 2];
        stream.write_all(&bytes).await.unwrap();
        let received = tokio::time::timeout(Duration::from_secs(2), async {
            let mut got = 0;
            while got < bytes.len() { got += first.audio.recv().await.unwrap().len(); }
            got
        }).await.unwrap();
        assert_eq!(received, bytes.len());
        let mut second = sink.subscribe(&sink.token).unwrap();
        assert_eq!(second.prime, vec![6; PRIME_BYTES]);
        stream.write_all(b"audio").await.unwrap();
        assert_eq!(second.audio.recv().await.unwrap(), b"audio");
        sink.close().await;
        assert!(matches!(second.audio.recv().await, Err(broadcast::error::RecvError::Closed)));
    }
}
