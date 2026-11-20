use std::time::{Duration, Instant};

use jpeg_encoder::{ColorType, Encoder};
use tokio::sync::mpsc;

use super::{Binary, Command, GwError};

#[derive(Default)]
pub(super) struct Snapshot {
    jpeg: Option<Vec<u8>>,
    pending: bool,
    last_sample: Option<Instant>,
    pub(super) sequence: u64,
}

pub(super) fn valid(w: usize, h: usize, bytes: usize) -> bool {
    (2..=1920).contains(&w) && (2..=1920).contains(&h) && w % 2 == 0 && h % 2 == 0
        && w * h <= 1920 * 1080 && bytes == w * h * 3 / 2
}

impl Snapshot {
    pub(super) fn submit(&mut self, w: usize, h: usize, raw: Vec<u8>, id: String, tx: mpsc::UnboundedSender<Command>) {
        if self.pending || self.last_sample.is_some_and(|t| t.elapsed() < Duration::from_millis(500)) { return; }
        self.pending = true;
        self.last_sample = Some(Instant::now());
        tokio::task::spawn_blocking(move || {
            let pixels = w * h;
            let mut ycbcr = Vec::with_capacity(3 * pixels);
            for y in 0..h {
                for x in 0..w {
                    let chroma = (y / 2) * (w / 2) + x / 2;
                    ycbcr.extend_from_slice(&[raw[y*w+x], raw[pixels+chroma], raw[pixels+pixels/4+chroma]]);
                }
            }
            let mut jpeg = Vec::new();
            let encoded = Encoder::new(&mut jpeg, 75).encode(&ycbcr, w as u16, h as u16, ColorType::Ycbcr);
            let _ = tx.send(Command::VideoJpeg(id, encoded.ok().map(|()| jpeg)));
        });
    }

    pub(super) fn finish(&mut self, jpeg: Option<Vec<u8>>) {
        self.pending = false;
        if let Some(jpeg) = jpeg {
            self.jpeg = Some(jpeg);
            self.sequence += 1;
        }
    }

    pub(super) fn response(&self) -> Result<Binary, GwError> {
        let bytes = self.jpeg.clone().ok_or_else(|| GwError::new(409, "NO_VIDEO", "Chưa có hình video của bên kia."))?;
        Ok(Binary { bytes, content_type: "image/jpeg".into(), cache_control: "no-store", disposition: None })
    }
}
