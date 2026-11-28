//! Bộ phát PCM TTS cục bộ đúng nhịp 20 ms qua loopback HTTP cho ffmpeg → NTgCalls.
use std::{net::Ipv4Addr, sync::{Arc, Mutex}, time::{Duration, Instant}};

use axum::{body::Body, extract::{Request, State}, http::{header, StatusCode}, response::{IntoResponse, Response}, routing::get, Router};
use subtle::ConstantTimeEq;
use tokio::{net::TcpListener, sync::{watch, Semaphore}, task::JoinHandle};

use crate::contract::GwError;

#[allow(dead_code)]
pub(super) const BYTES_PER_SEC: usize = 96_000;
pub(super) const FRAME_MS: u64 = 20;
pub(super) const FRAME_BYTES: usize = 1920;
pub(super) const LATE_GAP_MS: u64 = 40;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SpeechSnapshot {
    pub frames_total: u64,
    pub frames_sent: u64,
    pub silence_frames: u64,
    pub late_frames: u64,
    pub missed_frames: u64,
    pub max_gap_ms: u64,
    pub bytes_sent: u64,
    pub connected: bool,
    pub connect_delay_ms: Option<u64>,
}

struct SpeechState {
    pcm: Vec<u8>,
    offset: usize,
    frames_total: u64,
    frames_sent: u64,
    silence_frames: u64,
    late_frames: u64,
    max_gap_ms: u64,
    bytes_sent: u64,
    opened_at: Instant,
    connected: bool,
    connect_delay_ms: Option<u64>,
    last_frame_at: Option<Instant>,
}

struct SpeechLocal {
    key: String,
    state: Mutex<SpeechState>,
    listener: Arc<Semaphore>,
    alive: watch::Receiver<()>,
}

pub(super) struct SpeechStream {
    local: Arc<SpeechLocal>,
    port: u16,
    server: JoinHandle<()>,
    _alive: watch::Sender<()>,
}

impl SpeechStream {
    pub async fn open(pcm: Vec<u8>) -> Result<Self, GwError> {
        let key = super::browser::token()?;
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await
            .map_err(|_| GwError::new(500, "MEDIA_LOCAL", "Không mở được nguồn phát TTS cục bộ."))?;
        let port = socket.local_addr().map_err(|_| super::media_error())?.port();
        let frames_total = if pcm.is_empty() { 0u64 } else { ((pcm.len() + FRAME_BYTES - 1) / FRAME_BYTES) as u64 };
        let (alive, receiver) = watch::channel(());
        let local = Arc::new(SpeechLocal {
            key,
            state: Mutex::new(SpeechState {
                pcm,
                offset: 0,
                frames_total,
                frames_sent: 0,
                silence_frames: 0,
                late_frames: 0,
                max_gap_ms: 0,
                bytes_sent: 0,
                opened_at: Instant::now(),
                connected: false,
                connect_delay_ms: None,
                last_frame_at: None,
            }),
            listener: Arc::new(Semaphore::new(1)),
            alive: receiver,
        });
        let app = Router::new().route("/:key", get(local_speech)).with_state(local.clone());
        let mut stopped = local.alive.clone();
        let server = tokio::spawn(async move {
            let _ = axum::serve(socket, app).with_graceful_shutdown(async move { let _ = stopped.changed().await; }).await;
        });
        Ok(Self { local, port, server, _alive: alive })
    }

    pub fn command(&self) -> String {
        format!(
            "ffmpeg -hide_banner -loglevel error -nostdin -protocol_whitelist http,tcp,pipe -fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 -f s16le -ar 48000 -ac 1 -i http://127.0.0.1:{}/{} -f s16le -ac 1 -ar 48000 -flush_packets 1 pipe:1",
            self.port, self.local.key
        )
    }

    pub fn stats(&self) -> SpeechSnapshot {
        let st = self.local.state.lock().unwrap();
        SpeechSnapshot {
            frames_total: st.frames_total,
            frames_sent: st.frames_sent,
            silence_frames: st.silence_frames,
            late_frames: st.late_frames,
            missed_frames: st.frames_total.saturating_sub(st.frames_sent),
            max_gap_ms: st.max_gap_ms,
            bytes_sent: st.bytes_sent,
            connected: st.connected,
            connect_delay_ms: st.connect_delay_ms,
        }
    }

    #[cfg(test)]
    pub(super) fn loopback_url(&self) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, self.local.key)
    }
}

impl Drop for SpeechStream {
    fn drop(&mut self) { self.server.abort(); }
}

async fn local_speech(State(local): State<Arc<SpeechLocal>>, req: Request) -> Response {
    if !bool::from(req.uri().path().trim_start_matches('/').as_bytes().ct_eq(local.key.as_bytes())) || local.alive.has_changed().is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = local.listener.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    {
        let mut st = local.state.lock().unwrap();
        st.connected = true;
        st.connect_delay_ms = Some(st.opened_at.elapsed().as_millis() as u64);
    }
    let mut tick = tokio::time::interval(Duration::from_millis(FRAME_MS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let stopped = local.alive.clone();
    let stream = futures_util::stream::unfold((local, stopped, tick, permit), |(local, mut stopped, mut tick, permit)| async move {
        tokio::select! { biased;
            _ = stopped.changed() => return None,
            _ = tick.tick() => (),
        }
        let frame = {
            let mut st = local.state.lock().unwrap();
            let now = Instant::now();
            if st.offset < st.pcm.len() {
                let remaining = st.pcm.len() - st.offset;
                let take = remaining.min(FRAME_BYTES);
                let mut frame = vec![0u8; FRAME_BYTES];
                frame[..take].copy_from_slice(&st.pcm[st.offset..st.offset + take]);
                st.offset += take;
                if let Some(prev) = st.last_frame_at {
                    let gap_ms = now.duration_since(prev).as_millis() as u64;
                    if gap_ms > st.max_gap_ms {
                        st.max_gap_ms = gap_ms;
                    }
                    if gap_ms > LATE_GAP_MS {
                        st.late_frames += 1;
                    }
                }
                st.last_frame_at = Some(now);
                st.frames_sent += 1;
                st.bytes_sent += take as u64;
                frame
            } else {
                st.silence_frames += 1;
                st.last_frame_at = Some(now);
                vec![0u8; FRAME_BYTES]
            }
        };
        Some((Ok::<_, std::convert::Infallible>(frame), (local, stopped, tick, permit)))
    });
    ([(header::CONTENT_TYPE, "application/octet-stream"), (header::CACHE_CONTROL, "no-store")], Body::from_stream(stream)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn speech_stream_paces_fake_pcm_evenly_without_gaps_over_40ms_and_pads_silence() {
        let pcm_len = 10 * FRAME_BYTES + 320;
        let pcm: Vec<u8> = (0..pcm_len).map(|i| ((i / 2 + i % 251) % 251 + 1) as u8).collect();
        let stream = SpeechStream::open(pcm.clone()).await.unwrap();
        let client = reqwest::Client::new();
        assert_eq!(client.get(format!("http://127.0.0.1:{}/wrong", stream.port)).send().await.unwrap().status(), 403);
        let url = stream.loopback_url();
        let mut response = client.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(client.get(&url).send().await.unwrap().status(), 429);

        let mut received = Vec::with_capacity(13 * FRAME_BYTES);
        let mut first_frame_at: Option<Instant> = None;
        let mut eleventh_frame_at: Option<Instant> = None;
        while received.len() < 13 * FRAME_BYTES {
            let chunk = response.chunk().await.unwrap().expect("luồng phải tiếp tục đệm im lặng");
            received.extend_from_slice(&chunk);
            if first_frame_at.is_none() && received.len() >= FRAME_BYTES {
                first_frame_at = Some(Instant::now());
            }
            if eleventh_frame_at.is_none() && received.len() >= 11 * FRAME_BYTES {
                eleventh_frame_at = Some(Instant::now());
            }
        }
        let elapsed_11 = eleventh_frame_at.unwrap().duration_since(first_frame_at.unwrap());
        assert!(elapsed_11 >= Duration::from_millis(160), "elapsed_11={elapsed_11:?}");

        assert_eq!(&received[..pcm.len()], &pcm[..]);
        assert!(received[pcm.len()..11 * FRAME_BYTES].iter().all(|&b| b == 0));
        assert!(received[11 * FRAME_BYTES..13 * FRAME_BYTES].iter().all(|&b| b == 0));

        let st = stream.stats();
        assert!(st.connected);
        assert!(st.connect_delay_ms.is_some());
        assert_eq!(st.frames_total, 11);
        assert_eq!(st.frames_sent, 11);
        assert_eq!(st.missed_frames, 0);
        assert_eq!(st.late_frames, 0);
        assert!(st.max_gap_ms <= 40, "max_gap_ms={}", st.max_gap_ms);
        assert!(st.silence_frames >= 2, "silence_frames={}", st.silence_frames);
        assert_eq!(st.bytes_sent, pcm.len() as u64);

        let cmd = stream.command();
        assert!(cmd.contains("-flush_packets 1 pipe:1"), "{cmd}");
        assert!(cmd.contains("-protocol_whitelist http,tcp,pipe"), "{cmd}");
        assert!(cmd.contains("-fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0"), "{cmd}");
        assert!(cmd.contains("-f s16le -ar 48000 -ac 1 -i http://127.0.0.1:"), "{cmd}");
        assert!(!cmd.contains(" -re "), "{cmd}");

        drop(stream);
        tokio::time::timeout(Duration::from_secs(1), async {
            while response.chunk().await.unwrap().is_some() {}
        }).await.unwrap();
    }

    #[tokio::test]
    async fn speech_stream_tracks_missed_frames_when_stopped_early() {
        let pcm = vec![0x5Au8; 20 * FRAME_BYTES];
        let stream = SpeechStream::open(pcm).await.unwrap();
        let client = reqwest::Client::new();
        let mut response = client.get(stream.loopback_url()).send().await.unwrap();
        assert_eq!(response.status(), 200);

        let mut read_bytes = 0usize;
        while read_bytes < 3 * FRAME_BYTES {
            let chunk = response.chunk().await.unwrap().unwrap();
            read_bytes += chunk.len();
        }
        let st = stream.stats();
        assert!(st.connected);
        assert_eq!(st.frames_total, 20);
        assert!(st.frames_sent < 20, "frames_sent={}", st.frames_sent);
        assert!(st.missed_frames > 0, "missed_frames={}", st.missed_frames);

        drop(response);
        drop(stream);
    }
}
