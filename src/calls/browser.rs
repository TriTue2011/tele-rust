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
/// Độ trễ tối đa của hàng đợi mic: 160 ms (5120 byte). Luồng phát đọc đúng 20 ms/lần nên phần dư KHÔNG BAO GIỜ tiêu hết — trước đây trần là 64 KiB (hơn 2 giây)
/// và tiếng sang máy bên kia chậm vài trăm ms đến 2 s mãi mãi khi trình duyệt gửi dư một chút. Vượt thì bỏ phần mới đến, không báo lỗi (trình duyệt không ngắt mic).
const LAG_MAX_BYTES: usize = PCM_PER_SECOND / 1000 * 160;

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
    skipped: u64,
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
        let local = Arc::new(Local { key, queue: Mutex::new(Queue { pcm: VecDeque::new(), requests: VecDeque::new(), credit: MAX_PCM as f64, updated: Instant::now(), skipped: 0 }),
            listener: Arc::new(Semaphore::new(1)), alive: receiver });
        let app = Router::new().route("/:key", get(local_pcm)).with_state(local.clone());
        let mut stopped = local.alive.clone();
        let server = tokio::spawn(async move {
            let _ = axum::serve(socket, app).with_graceful_shutdown(async move { let _ = stopped.changed().await; }).await;
        });
        Ok(Self { local, port, server, _alive: alive })
    }

    /// Số byte PCM đã bỏ vì hàng đợi vượt 160 ms (chẩn đoán).
    pub fn skipped(&self) -> u64 { self.local.queue.lock().unwrap().skipped }

    pub fn push(&self, pcm: &[u8]) -> Result<(), GwError> {
        if pcm.len() > MAX_PCM { return Err(GwError::new(413, "AUDIO_TOO_LARGE", "PCM vượt 64 KiB.")); }
        if pcm.is_empty() || pcm.len() % 2 != 0 { return Err(GwError::bad_request("Cần PCM s16le 16 kHz mono, số byte chẵn và khác rỗng.")); }
        let mut q = self.local.queue.lock().unwrap();
        let now = Instant::now();
        while q.requests.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) { q.requests.pop_front(); }
        q.credit = (q.credit + now.duration_since(q.updated).as_secs_f64() * PCM_PER_SECOND as f64).min(MAX_PCM as f64);
        q.updated = now;
        if q.pcm.len() + pcm.len() > LAG_MAX_BYTES { q.skipped += pcm.len() as u64; return Ok(()); }
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
        format!("ffmpeg -hide_banner -loglevel quiet -nostdin -protocol_whitelist http,tcp -fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 -f s16le -ar 16000 -ac 1 -i http://127.0.0.1:{}/{} -f s16le -ac 1 -ar 48000 -flush_packets 1 pipe:1", self.port, self.local.key)
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

/// Camera từ trình duyệt (webcam USB/có sẵn): trình duyệt gửi từng khung JPEG, gateway giữ khung MỚI NHẤT và phát lại thành luồng MJPEG
/// cục bộ (đúng nhịp `fps`) cho ffmpeg → NTgCalls. Chưa có khung nào thì phát hình đen.
pub(crate) const MAX_JPEG: usize = 256 * 1024;
const MAX_FRAMES_PER_SECOND: usize = 30;

struct CamLocal {
    key: String,
    frame: watch::Sender<Arc<Vec<u8>>>,
    pushes: Mutex<VecDeque<Instant>>,
    listener: Arc<Semaphore>,
    alive: watch::Receiver<()>,
    fps: u64,
}

pub(super) struct BrowserCam {
    local: Arc<CamLocal>,
    port: u16,
    server: JoinHandle<()>,
    _alive: watch::Sender<()>,
}

fn black_jpeg() -> Vec<u8> {
    let (w, h) = (64usize, 36usize);
    let pixels: Vec<u8> = (0..w * h).flat_map(|_| [16u8, 128, 128]).collect();
    let mut out = Vec::new();
    let _ = jpeg_encoder::Encoder::new(&mut out, 50).encode(&pixels, w as u16, h as u16, jpeg_encoder::ColorType::Ycbcr);
    out
}

impl BrowserCam {
    pub async fn open(fps: u8) -> Result<Self, GwError> {
        let key = token()?;
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await
            .map_err(|_| GwError::new(500, "MEDIA_LOCAL", "Không mở được nguồn camera cục bộ."))?;
        let port = socket.local_addr().map_err(|_| super::media_error())?.port();
        let (alive, receiver) = watch::channel(());
        let local = Arc::new(CamLocal { key, frame: watch::Sender::new(Arc::new(black_jpeg())), pushes: Mutex::new(VecDeque::new()),
            listener: Arc::new(Semaphore::new(1)), alive: receiver, fps: u64::from(fps.max(1)) });
        let app = Router::new().route("/:key", get(local_frames)).with_state(local.clone());
        let mut stopped = local.alive.clone();
        let server = tokio::spawn(async move {
            let _ = axum::serve(socket, app).with_graceful_shutdown(async move { let _ = stopped.changed().await; }).await;
        });
        Ok(Self { local, port, server, _alive: alive })
    }

    pub fn push(&self, jpeg: &[u8]) -> Result<(), GwError> {
        if jpeg.len() > MAX_JPEG { return Err(GwError::new(413, "VIDEO_TOO_LARGE", "Khung JPEG vượt 256 KiB.")); }
        if jpeg.len() < 4 || jpeg[..2] != [0xFF, 0xD8] || jpeg[jpeg.len() - 2..] != [0xFF, 0xD9] {
            return Err(GwError::bad_request("Cần một khung JPEG hoàn chỉnh (bắt đầu FFD8, kết thúc FFD9)."));
        }
        let now = Instant::now();
        let mut pushes = self.local.pushes.lock().unwrap();
        while pushes.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) { pushes.pop_front(); }
        if pushes.len() >= MAX_FRAMES_PER_SECOND { return Err(GwError::new(429, "VIDEO_BACKPRESSURE", "Camera gửi quá nhanh.")); }
        pushes.push_back(now);
        self.local.frame.send_replace(Arc::new(jpeg.to_vec()));
        Ok(())
    }

    pub fn command(&self, profile: (i16, i16, u8)) -> String {
        let (w, h, fps) = profile;
        format!("ffmpeg -hide_banner -loglevel error -nostdin -protocol_whitelist http,tcp -fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 \
            -f mjpeg -use_wallclock_as_timestamps 1 -i http://127.0.0.1:{}/{} -vf scale={w}:{h},fps={fps} -f rawvideo -pix_fmt yuv420p pipe:1", self.port, self.local.key)
    }
}

impl Drop for BrowserCam {
    fn drop(&mut self) { self.server.abort(); }
}

async fn local_frames(State(local): State<Arc<CamLocal>>, req: Request) -> Response {
    if !bool::from(req.uri().path().trim_start_matches('/').as_bytes().ct_eq(local.key.as_bytes())) || local.alive.has_changed().is_err() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = local.listener.clone().try_acquire_owned() else { return StatusCode::TOO_MANY_REQUESTS.into_response(); };
    let mut tick = tokio::time::interval(Duration::from_millis(1000 / local.fps));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let stopped = local.alive.clone();
    let stream = futures_util::stream::unfold((local, stopped, tick, permit), |(local, mut stopped, mut tick, permit)| async move {
        tokio::select! { biased;
            _ = stopped.changed() => return None,
            _ = tick.tick() => (),
        }
        let frame = local.frame.borrow().clone();
        Some((Ok::<_, std::convert::Infallible>(frame.as_ref().clone()), (local, stopped, tick, permit)))
    });
    ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "no-store")], Body::from_stream(stream)).into_response()
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

    #[tokio::test]
    async fn camera_serves_latest_jpeg_at_rate_requires_key_and_validates_frames() {
        let cam = BrowserCam::open(15).await.unwrap();
        let client = reqwest::Client::new();
        assert_eq!(client.get(format!("http://127.0.0.1:{}/wrong", cam.port)).send().await.unwrap().status(), 403);
        let jpeg = |tag: u8| vec![0xFF, 0xD8, tag, 0xFF, 0xD9];
        assert_eq!(cam.push(&[1, 2, 3]).unwrap_err().status, 400);
        assert_eq!(cam.push(&vec![0xFF; MAX_JPEG + 1]).unwrap_err().status, 413);
        let mut response = client.get(format!("http://127.0.0.1:{}/{}", cam.port, cam.local.key)).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let first = response.chunk().await.unwrap().unwrap();
        assert_eq!(&first[..2], &[0xFF, 0xD8], "chưa có khung ⇒ phát hình đen hợp lệ");
        cam.push(&jpeg(7)).unwrap();
        let mut seen = false;
        for _ in 0..5 { if response.chunk().await.unwrap().unwrap().as_ref() == jpeg(7).as_slice() { seen = true; break; } }
        assert!(seen, "khung mới nhất phải xuất hiện trong luồng");
        for _ in 0..40 { let _ = cam.push(&jpeg(1)); }
        assert_eq!(cam.push(&jpeg(1)).unwrap_err().status, 429, "quá 30 khung/giây bị chặn");
        assert!(cam.command((640, 360, 15)).contains("-f mjpeg") && cam.command((640, 360, 15)).contains("scale=640:360,fps=15"));
    }
}
