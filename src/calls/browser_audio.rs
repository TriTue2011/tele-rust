//! Phát lại PCM đã được NTgCalls giải mã, không thêm bộ giải mã.
use std::sync::{atomic::{AtomicU64, Ordering}, Arc};
use std::time::{Duration, Instant};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, watch};
use crate::contract::{AudioOut, GwError};

const TOKEN_TTL: Duration = Duration::from_secs(600);
const MAX_LISTENERS: usize = 2;
const CHUNK_BYTES: usize = 1920;
const AUDIO_BROADCAST_CAP: usize = 512;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct BrowserAudioStats {
    pub frames_pushed: u64,
    pub lagged_frames: u64,
    pub no_listener_frames: u64,
    pub invalid_frames: u64,
    pub listeners: usize,
}

pub(super) struct BrowserAudio {
    token: String,
    expires: Instant,
    audio: broadcast::Sender<Vec<u8>>,
    alive: watch::Sender<()>,
    lagged_frames: Arc<AtomicU64>,
    no_listener_frames: AtomicU64,
    invalid_frames: AtomicU64,
    frames_pushed: AtomicU64,
}

impl BrowserAudio {
    pub fn new() -> Result<Self, GwError> {
        Ok(Self {
            token: super::browser::token()?,
            expires: Instant::now() + TOKEN_TTL,
            audio: broadcast::channel(AUDIO_BROADCAST_CAP).0,
            alive: watch::channel(()).0,
            lagged_frames: Arc::new(AtomicU64::new(0)),
            no_listener_frames: AtomicU64::new(0),
            invalid_frames: AtomicU64::new(0),
            frames_pushed: AtomicU64::new(0),
        })
    }

    pub fn descriptor(&mut self, id: &str) -> Result<Value, GwError> {
        if Instant::now() >= self.expires {
            self.token = super::browser::token()?;
            self.expires = Instant::now() + TOKEN_TTL;
        }
        Ok(json!({"url":format!("/api/live/audio/out/{id}?t={}",self.token),
            "content_type":"audio/wav","expires_in":self.expires.saturating_duration_since(Instant::now()).as_secs()}))
    }

    pub fn subscribe(&self, token: &str) -> Result<AudioOut, GwError> {
        if Instant::now() >= self.expires || !bool::from(self.token.as_bytes().ct_eq(token.as_bytes())) {
            return Err(GwError::new(403,"MEDIA_TOKEN","Thiếu, sai hoặc hết hạn token âm thanh."));
        }
        if self.audio.receiver_count() >= MAX_LISTENERS { return Err(GwError::new(429,"AUDIO_LISTENERS","Đã đủ hai người nghe.")); }
        // WAV PCM mono 48 kHz. Độ dài sentinel chẵn cho luồng chưa biết độ dài.
        let mut prime = super::wav(&[]);
        prime[4..8].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
        prime[40..44].copy_from_slice(&(u32::MAX - 37).to_le_bytes());
        Ok(AudioOut {
            prime,
            audio: self.audio.subscribe(),
            stopped: self.alive.subscribe(),
            expires: self.expires,
            lagged_frames: Some(self.lagged_frames.clone()),
        })
    }

    pub fn push(&self, pcm: &[u8]) {
        if pcm.is_empty() || pcm.len() % 2 != 0 || pcm.len() > 96_000 {
            self.invalid_frames.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.audio.receiver_count() == 0 {
            self.no_listener_frames.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for chunk in pcm.chunks(CHUNK_BYTES) {
            let _ = self.audio.send(chunk.to_vec());
            self.frames_pushed.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn stats(&self) -> BrowserAudioStats {
        BrowserAudioStats {
            frames_pushed: self.frames_pushed.load(Ordering::Relaxed),
            lagged_frames: self.lagged_frames.load(Ordering::Relaxed),
            no_listener_frames: self.no_listener_frames.load(Ordering::Relaxed),
            invalid_frames: self.invalid_frames.load(Ordering::Relaxed),
            listeners: self.audio.receiver_count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_is_per_call_expires_rotates_limits_listeners_and_revokes() {
        let mut output=BrowserAudio::new().unwrap();
        let other=BrowserAudio::new().unwrap();
        assert_eq!(output.subscribe("wrong").err().unwrap().status,403);
        assert_eq!(output.subscribe(&other.token).err().unwrap().status,403);
        let old=output.token.clone();
        output.push(&[1, 2]);
        assert_eq!(output.stats().no_listener_frames, 1);
        output.push(&[1]);
        assert_eq!(output.stats().invalid_frames, 1);
        let mut first=output.subscribe(&old).unwrap();
        assert_eq!(&first.prime[..4],b"RIFF");assert_eq!(&first.prime[8..12],b"WAVE");
        assert_eq!(u32::from_le_bytes(first.prime[24..28].try_into().unwrap()),48_000);
        let second=output.subscribe(&old).unwrap();
        assert_eq!(output.subscribe(&old).err().unwrap().status,429);
        drop(second);
        output.push(&[1,2,3,4]);assert_eq!(first.audio.recv().await.unwrap(),[1,2,3,4]);
        for _ in 0..(AUDIO_BROADCAST_CAP + 10) { output.push(&vec![0;CHUNK_BYTES]); }
        assert!(matches!(first.audio.recv().await,Err(broadcast::error::RecvError::Lagged(_))));
        assert!(output.stats().frames_pushed > 0);
        output.expires=Instant::now()-Duration::from_secs(1);
        assert_eq!(output.subscribe(&old).err().unwrap().status,403);
        assert!(!output.descriptor("call").unwrap().to_string().contains(&old));
        assert_ne!(old,output.token);
        drop(output);assert!(first.stopped.changed().await.is_err());
    }
}
