//! PCM từ trình duyệt: bộ đệm hữu hạn, nguồn loopback riêng cho NTgCalls.
use std::{collections::VecDeque, net::Ipv4Addr, sync::{Arc, Mutex}, time::{Duration, Instant}};

use axum::{body::Body, extract::{Request, State}, http::{header, StatusCode}, response::{IntoResponse, Response}, routing::get, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use subtle::ConstantTimeEq;
use tokio::{net::TcpListener, sync::{watch, Semaphore}, task::JoinHandle};

use crate::contract::GwError;

pub(crate) const MAX_PCM: usize = 64 * 1024;
const PCM_PER_SECOND: usize = 32_000;
const TICK_BYTES: usize = 640;

pub(super) fn token() -> Result<String, GwError> {
    let mut raw = [0; 24];
    rustls::crypto::ring::default_provider().secure_random.fill(&mut raw)
        .map_err(|_| GwError::new(500, "MEDIA_RANDOM", "Không tạo được khóa media."))?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

struct Queue {
    pcm: VecDeque<u8>,
    requests: VecDeque<Instant>,
    credit: f64,
    updated: Instant,
}

struct Local {
    key: String,
    queue: Mutex<Queue>,
    listener: Arc<Semaphore>,
    alive: watch::Receiver<()>,
}

pub(super) struct BrowserMic {
    local: Arc<Local>,
    port: u16,
    server: JoinHandle<()>,
    _alive: watch::Sender<()>,
}

impl BrowserMic {
    pub async fn open() -> Result<Self, GwError> {
        let key = token()?;
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await
            .map_err(|_| GwError::new(500, "MEDIA_LOCAL", "Không mở được nguồn mic cục bộ."))?;
        let port = socket.local_addr().map_err(|_| super::media_error())?.port();
        let (alive, receiver) = watch::channel(());
        let local = Arc::new(Local { key, queue: Mutex::new(Queue { pcm: VecDeque::new(), requests: VecDeque::new(), credit: MAX_PCM as f64, updated: Instant::now() }),
            listener: Arc::new(Semaphore::new(1)), alive: receiver });
        let app = Router::new().route("/:key", get(local_pcm)).with_state(local.clone());
        let mut stopped = local.alive.clone();
        let server = tokio::spawn(async move {
            let _ = axum::serve(socket, app).with_graceful_shutdown(async move { let _ = stopped.changed().await; }).await;
        });
        Ok(Self { local, port, server, _alive: alive })
    }

    pub fn push(&self, pcm: &[u8]) -> Result<(), GwError> {
        if pcm.len() > MAX_PCM { return Err(GwError::new(413, "AUDIO_TOO_LARGE", "PCM vượt 64 KiB.")); }
        if pcm.is_empty() || pcm.len() % 2 != 0 { return Err(GwError::bad_request("Cần PCM s16le 16 kHz mono, số byte chẵn và khác rỗng.")); }
        let mut q = self.local.queue.lock().unwrap();
        let now = Instant::now();
        while q.requests.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) { q.requests.pop_front(); }
        q.credit = (q.credit + now.duration_since(q.updated).as_secs_f64() * PCM_PER_SECOND as f64).min(MAX_PCM as f64);
        q.updated = now;
        if q.requests.len() >= 30 || q.credit < pcm.len() as f64 || q.pcm.len() + pcm.len() > MAX_PCM {
            return Err(GwError::new(429, "AUDIO_BACKPRESSURE", "Mic quá nhanh hoặc bộ đệm đã đầy."));
        }
        q.credit -= pcm.len() as f64;
        q.requests.push_back(now);
        q.pcm.extend(pcm);
        Ok(())
    }

    pub fn command(&self) -> String {
        // Binding NTgCalls nhận Shell, không có API argv. Chỉ chèn cổng u16 và khóa base64url tự sinh.
        format!("ffmpeg -hide_banner -loglevel quiet -nostdin -protocol_whitelist http,tcp -probesize 32 -analyzeduration 0 -f s16le -ar 16000 -ac 1 -i http://127.0.0.1:{}/{} -f s16le -ac 1 -ar 48000 pipe:1", self.port, self.local.key)
    }
}

impl Drop for BrowserMic {
    fn drop(&mut self) { self.server.abort(); }
}

async fn local_pcm(State(local): State<Arc<Local>>, req: Request) -> Response {
    if !bool::from(req.uri().path().trim_start_matches('/').as_bytes().ct_eq(local.key.as_bytes())) || local.alive.has_changed().is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = local.listener.clone().try_acquire_owned() else { return StatusCode::TOO_MANY_REQUESTS.into_response(); };
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let stopped = local.alive.clone();
    let stream = futures_util::stream::unfold((local, stopped, tick, permit), |(local, mut stopped, mut tick, permit)| async move {
        tokio::select! { biased;
            _ = stopped.changed() => return None,
            _ = tick.tick() => (),
        }
        let mut bytes = vec![0; TICK_BYTES];
        {
            let mut q = local.queue.lock().unwrap();
            for byte in &mut bytes { if let Some(value) = q.pcm.pop_front() { *byte = value; } else { break; } }
        }
        Some((Ok::<_, std::convert::Infallible>(bytes), (local, stopped, tick, permit)))
    });
    ([(header::CONTENT_TYPE, "application/octet-stream"), (header::CACHE_CONTROL, "no-store")], Body::from_stream(stream)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_pcm_requires_key_pads_silence_and_revokes_on_drop() {
        let mic = BrowserMic::open().await.unwrap();
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/{}", mic.port, mic.local.key);
        assert_eq!(client.get(format!("http://127.0.0.1:{}/wrong", mic.port)).send().await.unwrap().status(), 403);
        mic.push(&[1, 2, 3, 4]).unwrap();
        let mut response = client.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let bytes = response.chunk().await.unwrap().unwrap();
        assert_eq!(&bytes[..4], &[1, 2, 3, 4]); assert!(bytes[4..].iter().all(|b| *b == 0));
        assert_eq!(client.get(&url).send().await.unwrap().status(), 429);
        let silence = response.chunk().await.unwrap().unwrap();
        assert!(silence.iter().all(|b| *b == 0));
        drop(mic);
        tokio::time::timeout(Duration::from_secs(1), async { while response.chunk().await.unwrap().is_some() {} }).await.unwrap();
        assert!(client.get(&url).send().await.is_err());
    }

    #[tokio::test]
    async fn pcm_rejects_bad_size_queue_rate_and_byte_rate() {
        let mic = BrowserMic::open().await.unwrap();
        assert_eq!(mic.push(&[]).unwrap_err().status, 400);
        assert_eq!(mic.push(&[1]).unwrap_err().status, 400);
        assert_eq!(mic.push(&vec![0; MAX_PCM + 2]).unwrap_err().status, 413);
        mic.push(&vec![7; MAX_PCM]).unwrap();
        assert_eq!(mic.push(&[0, 0]).unwrap_err().status, 429);
        mic.local.queue.lock().unwrap().pcm.clear();
        assert_eq!(mic.push(&vec![0; 3200]).unwrap_err().status, 429);
        let other = BrowserMic::open().await.unwrap();
        for _ in 0..30 { other.push(&[0, 0]).unwrap(); }
        assert_eq!(other.push(&[0, 0]).unwrap_err().status, 429);
        assert_ne!(mic.local.key, other.local.key);
        assert!(mic.local.key.len() >= 22);
    }
}
