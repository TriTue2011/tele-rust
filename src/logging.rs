//! Lọc bí mật trước cả stderr và tệp WebRTC; callback C++ chỉ xếp hàng có giới hạn.
use std::{fs::{self, OpenOptions}, io::{self, Write}, os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf, sync::mpsc::{self, SyncSender}, time::{SystemTime, UNIX_EPOCH}};

use log::{LevelFilter, Log, Metadata, Record};
use regex::Regex;

use crate::calls::trace::mask;

pub fn level(get: impl Fn(&str) -> Option<String>) -> Result<LevelFilter, String> {
    let debug = match get("TELEGRAM_DEBUG").as_deref().unwrap_or("0").trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "no" | "off" => false,
        "1" | "true" | "yes" | "on" => true,
        _ => return Err("TELEGRAM_DEBUG phải là 0/1, false/true, no/yes hoặc off/on.".into()),
    };
    match get("LOG_LEVEL").unwrap_or_else(|| if debug { "DEBUG" } else { "INFO" }.into()).trim().to_ascii_uppercase().as_str() {
        "DEBUG" => Ok(LevelFilter::Debug), "INFO" => Ok(LevelFilter::Info),
        "WARNING" | "WARN" => Ok(LevelFilter::Warn), "ERROR" => Ok(LevelFilter::Error),
        "CRITICAL" | "OFF" => Ok(LevelFilter::Off),
        _ => Err("LOG_LEVEL phải là DEBUG, INFO, WARNING, ERROR, CRITICAL hoặc OFF.".into()),
    }
}

struct Logger { stderr: LevelFilter, webrtc: LevelFilter, filter: Option<Regex>, file: Option<SyncSender<String>> }

impl Log for Logger {
    fn enabled(&self, m: &Metadata<'_>) -> bool {
        m.level() <= if m.target().starts_with("ntgcalls") { self.webrtc } else { self.stderr }
    }
    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) { return; }
        let text: String = mask(&record.args().to_string()).chars().take(8192).collect();
        if record.target().starts_with("ntgcalls") {
            if self.filter.as_ref().is_some_and(|f| !f.is_match(&text)) { return; }
            if let Some(file) = &self.file {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                let _ = file.try_send(format!("{:02}:{:02}:{:02}.{:03} {} {} {}\n", (now.as_secs()/3600)%24,
                    (now.as_secs()/60)%60, now.as_secs()%60, now.subsec_millis(), record.target(), record.level(), text));
            }
        } else { eprintln!("{} {}", record.level(), text); }
    }
    fn flush(&self) {}
}

/// Gọi một lần từ main, trước khi mở thư viện native.
pub fn init(data_dir: PathBuf) -> Result<(), String> {
    let stderr = level(|key| std::env::var(key).ok())?;
    let webrtc = match std::env::var("TELEGRAM_WEBRTC_LOG").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "" => LevelFilter::Off, "info" => LevelFilter::Info, "debug" => LevelFilter::Debug,
        _ => return Err("TELEGRAM_WEBRTC_LOG phải để trống, info hoặc debug.".into()),
    };
    let mb = std::env::var("TELEGRAM_WEBRTC_LOG_MB").unwrap_or_else(|_| "20".into()).trim().parse::<u64>()
        .ok().filter(|n| (1..=1024).contains(n)).ok_or("TELEGRAM_WEBRTC_LOG_MB phải là số từ 1 đến 1024.")?;
    let filter = std::env::var("TELEGRAM_WEBRTC_LOG_LOC").ok().filter(|v| !v.trim().is_empty())
        .map(|v| Regex::new(&v).map_err(|_| "TELEGRAM_WEBRTC_LOG_LOC không phải biểu thức chính quy hợp lệ.")).transpose()?;
    let file = if webrtc != LevelFilter::Off {
        let mut writer = Rotating::open(data_dir.join("calltrace"), mb * 1024 * 1024)
            .map_err(|_| "Không mở được tệp nhật ký WebRTC.")?;
        let (tx, rx) = mpsc::sync_channel::<String>(1024);
        std::thread::Builder::new().name("tggw-webrtc-log".into()).spawn(move || {
            while let Ok(line) = rx.recv() { let _ = writer.write(&line); }
        }).map_err(|_| "Không tạo được luồng ghi nhật ký WebRTC.")?;
        Some(tx)
    } else { None };
    log::set_boxed_logger(Box::new(Logger { stderr, webrtc, filter, file })).map_err(|_| "Bộ ghi nhật ký đã được khởi tạo.")?;
    log::set_max_level(stderr.max(webrtc));
    ntgcalls::init_logging();
    Ok(())
}

struct Rotating { dir: PathBuf, file: fs::File, bytes: u64, max: u64 }

impl Rotating {
    fn open(dir: PathBuf, max: u64) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        if fs::symlink_metadata(&dir)?.file_type().is_symlink() { return Err(io::ErrorKind::InvalidInput.into()); }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let file = Self::file(&dir)?;
        let bytes = file.metadata()?.len();
        Ok(Self { dir, file, bytes, max })
    }
    fn file(dir: &std::path::Path) -> io::Result<fs::File> {
        let file = OpenOptions::new().create(true).append(true).mode(0o600)
            .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32).open(dir.join("webrtc.log"))?;
        if !file.metadata()?.is_file() { return Err(io::ErrorKind::InvalidInput.into()); }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    fn write(&mut self, line: &str) -> io::Result<()> {
        let line = mask(line.trim_end()) + "\n";
        if line.len() as u64 > self.max { return Ok(()); }
        if self.bytes + line.len() as u64 > self.max {
            let _ = fs::remove_file(self.dir.join("webrtc.log.2"));
            if self.dir.join("webrtc.log.1").exists() { fs::rename(self.dir.join("webrtc.log.1"), self.dir.join("webrtc.log.2"))?; }
            fs::rename(self.dir.join("webrtc.log"), self.dir.join("webrtc.log.1"))?;
            self.file = Self::file(&self.dir)?;
            self.bytes = 0;
        }
        self.file.write_all(line.as_bytes())?;
        self.bytes += line.len() as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_callback_filter_level_and_secret_mask_apply_before_queueing() {
        let (tx, rx) = mpsc::sync_channel(4);
        let logger = Logger { stderr: LevelFilter::Error, webrtc: LevelFilter::Info,
            filter: Some(Regex::new("bwe").unwrap()), file: Some(tx) };
        logger.log(&Record::builder().args(format_args!("bwe token=private +84900000000 https://test.invalid/?k=private"))
            .target("ntgcalls::webrtc").level(log::Level::Info).build());
        let line = rx.try_recv().unwrap();
        assert!(line.contains("bwe") && line.contains("INFO"));
        assert!(!line.contains("private") && !line.contains("84900000000") && !line.contains("https://"));
        logger.log(&Record::builder().args(format_args!("bwe debug"))
            .target("ntgcalls::webrtc").level(log::Level::Debug).build());
        logger.log(&Record::builder().args(format_args!("other message"))
            .target("ntgcalls::webrtc").level(log::Level::Info).build());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn webrtc_rotates_two_backups_and_masks_secrets() {
        let dir = PathBuf::from(format!("target/webrtc-test-{}", std::process::id()));
        let mut writer = Rotating::open(dir.clone(), 128).unwrap();
        for i in 0..20 { writer.write(&format!("{i} ntgcalls INFO token=private +84900000000 https://test.invalid/key abcdefghijklmnop")).unwrap(); }
        drop(writer);
        for name in ["webrtc.log", "webrtc.log.1", "webrtc.log.2"] {
            let path = dir.join(name);
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.len() <= 128);
            assert!(!text.contains("private") && !text.contains("84900000000") && !text.contains("https://"));
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn log_level_and_debug_validate_and_explicit_level_wins() {
        let get = |level: Option<&str>, debug: Option<&str>| super::level(|k| match k { "LOG_LEVEL" => level.map(str::to_owned), _ => debug.map(str::to_owned) });
        assert_eq!(get(None, None).unwrap(), LevelFilter::Info);
        assert_eq!(get(None, Some("1")).unwrap(), LevelFilter::Debug);
        assert_eq!(get(Some("ERROR"), Some("1")).unwrap(), LevelFilter::Error);
        assert!(get(Some("invalid"), None).unwrap_err().contains("LOG_LEVEL"));
        assert!(get(None, Some("invalid")).unwrap_err().contains("TELEGRAM_DEBUG"));
        for level in ["DEBUG", "INFO", "WARNING", "WARN", "ERROR", "CRITICAL", "OFF"] { assert!(get(Some(level), Some("0")).is_ok()); }
    }
}
