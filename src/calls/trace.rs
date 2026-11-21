//! Nhật ký riêng: I/O ở luồng ghi, không đưa bí mật hoặc payload signaling vào tệp.
use std::{collections::HashSet, fs::{self, File, OpenOptions}, io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf},
    sync::{mpsc::{self, SyncSender}, Mutex, OnceLock}, time::{Instant, SystemTime, UNIX_EPOCH}};

use regex::Regex;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub(crate) struct Config { pub keep: usize, pub keep_bad: usize, pub max_bytes: u64 }

impl Config {
    pub(crate) fn from_env() -> Result<Self, String> {
        fn number(name: &str, default: u64, max: u64) -> Result<u64, String> {
            let n = match std::env::var(name) {
                Ok(v) => v.trim().parse::<u64>().map_err(|_| format!("{name} phải là số nguyên không âm."))?,
                Err(_) => default,
            };
            if n > max { return Err(format!("{name} vượt giới hạn {max}.")); }
            Ok(n)
        }
        let kb = number("TELEGRAM_CALLTRACE_MAX_KB", 6144, 65536)?;
        if kb == 0 { return Err("TELEGRAM_CALLTRACE_MAX_KB phải lớn hơn 0.".into()); }
        Ok(Self { keep: number("TELEGRAM_CALLTRACE_KEEP", 3, 10000)? as usize,
            keep_bad: number("TELEGRAM_CALLTRACE_KEEP_BAD", 30, 10000)? as usize, max_bytes: kb * 1024 })
    }
}

fn active() -> &'static Mutex<HashSet<PathBuf>> {
    static ACTIVE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    ACTIVE.get_or_init(Mutex::default)
}

enum Record { Line(String, String), Bad(String), Finish(String) }

pub(super) struct Trace { sender: SyncSender<Record>, worker: Option<std::thread::JoinHandle<()>>, counters: Vec<u64>, tick: u64 }

impl Trace {
    pub(super) fn open(data_dir: &Path, id: &str) -> Option<Self> {
        let cfg = Config::from_env().ok()?;
        Self::with_config(data_dir, id, cfg)
    }

    fn with_config(data_dir: &Path, id: &str, cfg: Config) -> Option<Self> {
        if cfg.keep == 0 { return None; }
        let dir = data_dir.join("calltrace");
        fs::create_dir_all(&dir).ok()?;
        // Không theo symlink, kể cả tệp có sẵn. Mã cuộc gọi được sinh nội bộ.
        if fs::symlink_metadata(&dir).ok()?.file_type().is_symlink() { return None; }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).ok()?;
        let path = dir.join(format!("{id}.log"));
        let file = OpenOptions::new().create_new(true).write(true).mode(0o600).open(&path).ok()?;
        active().lock().unwrap().insert(path.clone());
        let (sender, receiver) = mpsc::sync_channel(2048);
        let mut writer = Writer { file, path: path.clone(), started: Instant::now(), bytes: 0, lines: 0, cfg, full: true, bad: false };
        let header = format!("call={id} gateway={} pid={} keep={} max_kb={}", env!("CARGO_PKG_VERSION"), std::process::id(), cfg.keep, cfg.max_bytes / 1024);
        let worker = std::thread::Builder::new().name("tggw-trace".into()).spawn(move || {
            writer.line("mo-dau", &header, false);
            while let Ok(record) = receiver.recv() {
                match record {
                    Record::Line(kind, text) => writer.line(&kind, &text, false),
                    Record::Bad(reason) => writer.bad(&reason),
                    Record::Finish(summary) => {
                        writer.line("tong-ket", &summary, true);
                        let conclusion = if writer.bad { "LOI" } else { "tot" };
                        let footer = format!("dong={} byte={} ket_luan={conclusion}", writer.lines, writer.bytes);
                        writer.line("ket-thuc-nhat-ky", &footer, true);
                        break;
                    }
                }
            }
            active().lock().unwrap().remove(&writer.path);
            prune(&dir, cfg);
        });
        match worker {
            Ok(worker) => Some(Self { sender, worker: Some(worker), counters: Vec::new(), tick: 0 }),
            Err(_) => { active().lock().unwrap().remove(&path); None }
        }
    }

    pub(super) fn line(&self, kind: &str, text: &str) {
        let _ = self.sender.try_send(Record::Line(kind.into(), mask(text).chars().take(3000).collect()));
    }

    pub(super) fn bad(&self, reason: &str) {
        let _ = self.sender.send(Record::Bad(mask(reason).chars().take(80).collect()));
    }

    pub(super) fn sample(&mut self, counters: &[u64]) {
        const NAMES: [&str; 8] = ["bao_hieu_tx_goi", "bao_hieu_tx_byte", "bao_hieu_rx_goi", "bao_hieu_rx_byte",
            "khung_tieng_vao", "byte_tieng_vao", "khung_hinh_vao", "khung_hinh_hong"];
        let diffs = counters.iter().enumerate().filter_map(|(i, n)| {
            let d = n.saturating_sub(self.counters.get(i).copied().unwrap_or(0));
            (d > 0).then(|| format!("{}+{d}", NAMES[i]))
        }).collect::<Vec<_>>();
        if !diffs.is_empty() { self.line("dem-1s", &diffs.join(" ")); }
        if self.tick % 5 == 0 {
            self.line("dem-tong", &counters.iter().enumerate().map(|(i,n)| format!("{}={n}", NAMES[i])).collect::<Vec<_>>().join(" "));
        }
        self.counters = counters.to_vec();
        self.tick += 1;
    }

    pub(super) fn finish(&mut self, summary: &Value) {
        if self.worker.is_none() { return; }
        let mut summary = summary.clone();
        if let Some(map) = summary.as_object_mut() {
            for key in ["events", "user_id", "error", "emojis"] { map.remove(key); }
        }
        let _ = self.sender.send(Record::Finish(mask(&summary.to_string())));
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

impl Drop for Trace {
    fn drop(&mut self) { self.finish(&json!({"state":"ended","reason":"runtime dừng"})); }
}

struct Writer { file: File, path: PathBuf, started: Instant, bytes: u64, lines: u64, cfg: Config, full: bool, bad: bool }

impl Writer {
    fn line(&mut self, kind: &str, text: &str, force: bool) {
        if !self.full && !force { return; }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let prefix = format!("{:02}:{:02}:{:02}.{:03} +{:.3}s", (now.as_secs() / 3600) % 24,
            (now.as_secs() / 60) % 60, now.as_secs() % 60, now.subsec_millis(), self.started.elapsed().as_secs_f64());
        // Chừa chỗ cho dấu trần, dấu lỗi, tổng kết và chân trang ngay cả ở mức 1 KB.
        let reserve = (self.cfg.max_bytes / 2).min(4096);
        let limit = if force { self.cfg.max_bytes.saturating_sub(self.bytes + 100) as usize } else { 3000 };
        let mut end = text.len().min(limit);
        while !text.is_char_boundary(end) { end -= 1; }
        let text = &text[..end];
        let mut line = format!("{prefix} {kind} {text}\n");
        if !force && self.bytes + line.len() as u64 > self.cfg.max_bytes - reserve {
            self.full = false;
            line = format!("{prefix} trần dung lượng {} KB: ngừng ghi chi tiết (chỉ còn dòng kết thúc)\n", self.cfg.max_bytes / 1024);
        }
        if self.bytes + line.len() as u64 <= self.cfg.max_bytes && self.file.write_all(line.as_bytes()).is_ok() {
            self.bytes += line.len() as u64;
            self.lines += 1;
        }
    }

    fn bad(&mut self, reason: &str) {
        if self.bad { return; }
        self.bad = true;
        self.line("danh-dau-loi", reason, true);
        if let Ok(mut file) = OpenOptions::new().create_new(true).write(true).mode(0o600).open(self.path.with_extension("loi")) {
            let _ = writeln!(file, "{}", mask(reason));
        }
    }
}

pub(crate) fn mask(text: &str) -> String {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| [
        r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s"<>]+"#,
        r#"(?i)(?:token|api_hash|api_id|password|otp|code|session|key|secret|ufrag|pwd|cookie)["']?\s*[:=]\s*(?:"[^"]*"|'[^']*'|[^"',}\s]+)"#,
        r"(?i)Bearer\s+\S+", r"\+?\d(?:[ ()-]?\d){8,}", r"[A-Za-z0-9+/=_\-]{40,}",
        r"\b(?:\d{1,3}\.){3}\d{1,3}\b",
    ].iter().map(|p| Regex::new(p).unwrap()).collect());
    let mut text = text.replace(['\r', '\n'], " ");
    for pattern in patterns { text = pattern.replace_all(&text, "<ẩn>").into_owned(); }
    for name in ["GATEWAY_TOKEN", "TG_API_HASH", "TELEGRAM_API_HASH", "TG_PHONE", "TG_API_ID", "TELEGRAM_API_ID", "HA_WEBHOOK_TOKEN"] {
        if let Ok(secret) = std::env::var(name) { if !secret.is_empty() { text = text.replace(&secret, "<ẩn>"); } }
    }
    text
}

pub(crate) fn valid_id(id: &str) -> bool { (6..=32).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) }

fn logs(dir: &Path) -> Vec<(PathBuf, fs::Metadata)> {
    if !fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()) { return Vec::new(); }
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut out = entries.filter_map(Result::ok).filter_map(|e| {
        let path = e.path();
        if path.extension()?.to_str()? != "log" || !valid_id(path.file_stem()?.to_str()?) { return None; }
        let meta = e.path().symlink_metadata().ok()?;
        meta.is_file().then_some((path, meta))
    }).collect::<Vec<_>>();
    out.sort_by_key(|(p, m)| (m.modified().ok(), p.clone()));
    out.reverse();
    out
}

fn prune(dir: &Path, cfg: Config) {
    let active = active().lock().unwrap();
    let (mut good, mut bad) = (0, 0);
    for (path, _) in logs(dir) {
        if active.contains(&path) { continue; }
        let is_bad = path.with_extension("loi").symlink_metadata().is_ok_and(|meta| meta.is_file());
        let remove = if is_bad { bad += 1; bad > cfg.keep_bad } else { good += 1; good > cfg.keep };
        if remove { let _ = fs::remove_file(&path); if is_bad { let _ = fs::remove_file(path.with_extension("loi")); } }
    }
}

pub(crate) fn list(data_dir: &Path) -> Result<Value, String> {
    let cfg = Config::from_env()?;
    let active = active().lock().unwrap();
    let calls: Vec<_> = logs(&data_dir.join("calltrace")).into_iter().map(|(path, meta)| {
        let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
        let reason = OpenOptions::new().read(true).custom_flags(flags.bits() as i32).open(path.with_extension("loi"))
            .ok().and_then(|f| {
                if !f.metadata().ok()?.is_file() { return None; }
                let mut s = Vec::new();
                f.take(512).read_to_end(&mut s).ok()?;
                let s = String::from_utf8_lossy(&s);
                Some(if s.trim().is_empty() { "loi".into() } else { mask(s.trim()) })
            });
        json!({"id":path.file_stem().unwrap().to_str().unwrap(),"bytes":meta.len(),
            "modified":meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0),
            "loi":reason,"live":active.contains(&path)})
    }).collect();
    Ok(json!({"ok":true,"enabled":cfg.keep>0,"keep":cfg.keep,"calls":calls}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_format_retention_errors_cap_and_redaction() {
        let dir = PathBuf::from(format!("target/trace-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cfg = Config { keep: 1, keep_bad: 1, max_bytes: 1024 };
        let mut live = Trace::with_config(&dir, "abcdef0000", cfg).unwrap();
        live.line("su-kien", "TDLib Ready");
        for (id, bad) in [("abcdef0001", true), ("abcdef0002", false), ("abcdef0003", false)] {
            let mut t = Trace::with_config(&dir, id, cfg).unwrap();
            t.line("su-kien", "NTgCalls CONNECTED");
            t.line("su-kien", "https://test.invalid/?token=test-key +84900000000 api_hash=secret Bearer private");
            for _ in 0..100 { t.line("su-kien", "dòng đủ dài để đạt dung lượng"); }
            if bad { t.bad("NTgCalls FAILED"); }
            t.finish(&json!({"state":"ended"}));
        }
        let calls = list(&dir).unwrap();
        assert_eq!(calls["calls"].as_array().unwrap().len(), 3);
        assert!(dir.join("calltrace/abcdef0000.log").exists());
        assert!(!dir.join("calltrace/abcdef0002.log").exists());
        let path = dir.join("calltrace/abcdef0001.log");
        let text = fs::read_to_string(&path).unwrap();
        assert!(Regex::new(r"(?m)^\d{2}:\d{2}:\d{2}\.\d{3} \+\d+\.\d{3}s su-kien ").unwrap().is_match(&text));
        assert_eq!(text.matches("trần dung lượng").count(), 1);
        assert!(text.contains("tong-ket") && text.contains("danh-dau-loi") && text.contains("ket-thuc-nhat-ky"));
        for secret in ["test-key", "+84900000000", "secret", "private"] { assert!(!text.contains(secret)); }
        assert!(text.len() <= 1024);
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, 0o600);
        live.finish(&json!({"state":"ended"}));
        assert!(Trace::with_config(&dir, "abcdef0004", Config { keep: 0, ..cfg }).is_none());
        for (path, _) in logs(&dir.join("calltrace")) { fs::remove_file(&path).unwrap(); let _ = fs::remove_file(path.with_extension("loi")); }
        fs::remove_dir(dir.join("calltrace")).unwrap(); fs::remove_dir(dir).unwrap();
    }
}
