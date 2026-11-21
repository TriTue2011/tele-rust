//! Chính sách cache cho một phiên TDLib. Chỉ công bố số liệu và mã hội thoại.
use std::{collections::BTreeMap, fs, io, path::Path, time::{Instant, SystemTime, UNIX_EPOCH}};

use serde_json::{json, Value};

use crate::contract::{GwError, TdLink};
use crate::storage_settings;

#[cfg(test)]
mod tests;

const MAX_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const MAX_FILES: u64 = 5000;
const TYPES: &[&str] = &[
    "fileTypePhoto", "fileTypeProfilePhoto", "fileTypePhotoStory", "fileTypeVideo", "fileTypeVideoNote",
    "fileTypeVideoStory", "fileTypeAnimation", "fileTypeLivePhotoVideo", "fileTypeDocument", "fileTypeAudio",
    "fileTypeVoiceNote", "fileTypeSticker", "fileTypeThumbnail", "fileTypeWallpaper", "fileTypeNotificationSound",
];

fn unavailable() -> GwError {
    GwError::new(503, "STORAGE_STATS", "Chưa lấy được thống kê lưu trữ từ Telegram.")
}

pub(crate) fn settings_error() -> GwError {
    GwError::new(500, "STORAGE_SETTINGS_IO", "Không đọc hoặc ghi được cấu hình lưu trữ.")
}

pub(crate) async fn settings(session: &Path) -> Result<Value, GwError> {
    let dir = session.to_owned();
    tokio::task::spawn_blocking(move || storage_settings::load(&dir))
        .await.map_err(|_| settings_error())?.map_err(|_| settings_error())
}

pub(crate) async fn check_cache_paths(session: &Path) -> Result<(), GwError> {
    let root = session.join("files");
    let unsafe_path = || GwError::new(409, "STORAGE_CACHE_PATH", "Thư mục bộ nhớ đệm có đường dẫn chuyển hướng hoặc không đọc được.");
    tokio::task::spawn_blocking(move || {
        // TDLib có thể mở thư mục loại tệp gốc qua symlink. Không quét/xoá khi thấy
        // đường dẫn chuyển hướng trong phiên riêng này.
        let root = std::path::absolute(root)?;
        if root.canonicalize()? != root { return Err(io::ErrorKind::InvalidData.into()); }
        let mut directories = vec![root];
        while let Some(dir) = directories.pop() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_symlink() { return Err(io::ErrorKind::InvalidData.into()); }
                if kind.is_dir() { directories.push(entry.path()); }
            }
        }
        Ok::<(), io::Error>(())
    }).await.map_err(|_| unsafe_path())?.map_err(|_| unsafe_path())
}

async fn stats(td: &dyn TdLink, chat_limit: i32) -> Result<Value, GwError> {
    let stats = td.request(json!({"@type":"getStorageStatistics","chat_limit":chat_limit}))
        .await.map_err(|_| unavailable())?;
    if stats["@type"] != "storageStatistics" || !stats["by_chat"].is_array() {
        return Err(unavailable());
    }
    Ok(stats)
}

pub(crate) async fn usage(td: &dyn TdLink) -> Result<Value, GwError> {
    let stats = stats(td, 20).await?;
    let total = stats["size"].as_u64().ok_or_else(unavailable)?;
    let chats = stats["by_chat"].as_array().ok_or_else(unavailable)?;
    let mut by_type = [0_u64; 5];
    let mut top = Vec::new();
    for chat in chats {
        let id = chat["chat_id"].as_i64().ok_or_else(unavailable)?;
        let size = chat["size"].as_u64().ok_or_else(unavailable)?;
        if id != 0 && size > 0 { top.push((id, size)); }
        for info in chat["by_file_type"].as_array().ok_or_else(unavailable)? {
            let group = match info["file_type"]["@type"].as_str() {
                Some("fileTypePhoto" | "fileTypeProfilePhoto" | "fileTypePhotoStory") => 0,
                Some("fileTypeVideo" | "fileTypeVideoNote" | "fileTypeAnimation" | "fileTypeLivePhotoVideo" | "fileTypeVideoStory") => 1,
                Some("fileTypeDocument") => 2,
                Some("fileTypeAudio") => 3,
                _ => 4,
            };
            let bytes = info["size"].as_u64().ok_or_else(unavailable)?;
            by_type[group] = by_type[group].checked_add(bytes).ok_or_else(unavailable)?;
        }
    }
    let tally = by_type.iter().try_fold(0_u64, |sum, &bytes| sum.checked_add(bytes));
    by_type[4] += tally.and_then(|sum| total.checked_sub(sum)).ok_or_else(unavailable)?;
    top.sort_unstable_by_key(|&(id, size)| (std::cmp::Reverse(size), id));
    let top = top.into_iter().take(20).map(|(id, bytes)| json!({"chat_id":id,"bytes":bytes})).collect::<Vec<_>>();
    let [photo, video, file, music, other] = by_type;
    Ok(json!({"total_bytes":total,"by_type":{"photo":photo,"video":video,"file":file,"music":music,"other":other},"top_chats":top}))
}

#[derive(Clone, Copy, Default)]
struct Size { bytes: u64, files: u64 }

impl Size {
    fn read(value: &Value) -> Result<Self, GwError> {
        Ok(Self { bytes: value["size"].as_u64().ok_or_else(unavailable)?,
            files: value["count"].as_u64().ok_or_else(unavailable)? })
    }
    fn add(self, next: Self) -> Self {
        Self { bytes: self.bytes.saturating_add(next.bytes), files: self.files.saturating_add(next.files) }
    }
    fn fits(self, limit: Self) -> bool { self.bytes <= limit.bytes && self.files <= limit.files }
}

pub(crate) enum Clear { All, Types(Vec<&'static str>), Chat(i64) }

impl Clear {
    pub(crate) fn parse(body: &Value) -> Result<Self, GwError> {
        if body["confirm"] != true { return Err(GwError::bad_request("Cần xác nhận xoá bộ nhớ đệm.")); }
        let invalid = || GwError::bad_request("Phạm vi xoá bộ nhớ đệm không hợp lệ.");
        match body["scope"].as_str() {
            Some("all") => Ok(Self::All),
            Some("chat") => body["chat_id"].as_i64().filter(|id| *id != 0 && id.unsigned_abs() < 1 << 53)
                .map(Self::Chat).ok_or_else(invalid),
            Some("type") => {
                let mut types = Vec::new();
                for kind in body["file_types"].as_array().filter(|list| !list.is_empty()).ok_or_else(invalid)? {
                    let group: &[&str] = match kind.as_str() {
                        Some("photo") => &["fileTypePhoto", "fileTypeProfilePhoto", "fileTypePhotoStory"],
                        Some("video") => &["fileTypeVideo", "fileTypeVideoNote", "fileTypeVideoStory", "fileTypeAnimation", "fileTypeLivePhotoVideo"],
                        Some("file") => &["fileTypeDocument"],
                        Some("music") => &["fileTypeAudio"],
                        Some("other") => &["fileTypeVoiceNote", "fileTypeSticker", "fileTypeThumbnail", "fileTypeWallpaper", "fileTypeNotificationSound"],
                        Some(kind) => {
                            types.push(TYPES.iter().copied().find(|&allowed| allowed == kind).ok_or_else(invalid)?);
                            continue;
                        }
                        _ => return Err(invalid()),
                    };
                    types.extend_from_slice(group);
                }
                types.sort_unstable();
                types.dedup();
                Ok(Self::Types(types))
            }
            _ => Err(invalid()),
        }
    }
}

pub(crate) struct Report {
    last_run: Option<u64>,
    deleted: Size,
    error: Option<&'static str>,
    limited: bool,
}

impl Report {
    pub(crate) fn new() -> Self {
        Self { last_run: None, deleted: Size::default(), error: None, limited: false }
    }
    pub(crate) fn json(&self) -> Value {
        json!({"last_run":self.last_run,"freed_bytes":self.deleted.bytes,"freed_files":self.deleted.files,
            "error":self.error,"limited":self.limited})
    }
    pub(crate) fn failed(&mut self, code: &'static str) { self.error = Some(code); }
    pub(crate) fn finish(&mut self) {
        self.last_run = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs());
        log::info!("storage bytes={} files={} limited={} failed={}", self.deleted.bytes, self.deleted.files,
            self.limited, self.error.is_some());
    }
}

struct Sweep<'a> { td: &'a dyn TdLink, report: &'a mut Report, inventory_started: Instant }

impl Sweep<'_> {
    async fn optimize(&mut self, chats: &[i64], exclude: &[i64], types: &[&str], ttl: i32, size: u64, estimate: Size) -> Result<(), GwError> {
        if estimate.files == 0 { return Ok(()); }
        // TDLib đặt giới hạn cho phần *còn lại*. Chỉ gửi một phạm vi mà ngay cả xoá hết
        // cũng nằm trong ngân sách, thay vì dùng size/count làm số byte/tệp được phép xoá.
        if !self.report.deleted.add(estimate).fits(Size { bytes: MAX_BYTES, files: MAX_FILES }) {
            self.report.limited = true;
            return Ok(());
        }
        // Bảo vệ cả tệp mới hoàn tất trong lúc chụp thống kê hoặc TDLib quét lại.
        let immunity = (self.inventory_started.elapsed() + crate::td::REQUEST_TIMEOUT)
            .as_secs().saturating_add(1).min(i32::MAX as u64);
        let result = self.td.request(json!({"@type":"optimizeStorage","chat_ids":chats,"exclude_chat_ids":exclude,
            "file_types":types.iter().map(|kind| json!({"@type":kind})).collect::<Vec<_>>(),
            "size":size,"ttl":ttl,"count":i32::MAX,"immunity_delay":immunity,
            "return_deleted_file_statistics":true,"chat_limit":0}))
            .await.map_err(|_| GwError::new(503, "STORAGE_CLEANUP", "Telegram chưa dọn được bộ nhớ đệm."))?;
        let deleted = Size::read(&result)?;
        self.report.deleted = self.report.deleted.add(deleted);
        if !deleted.fits(estimate) || !self.report.deleted.fits(Size { bytes: MAX_BYTES, files: MAX_FILES }) {
            self.report.limited = true;
            return Err(GwError::new(503, "STORAGE_INVENTORY_CHANGED", "Dung lượng đã thay đổi trong lúc dọn."));
        }
        Ok(())
    }
}

pub(crate) async fn clear(td: &dyn TdLink, scope: Clear, report: &mut Report) -> Result<(), GwError> {
    let inventory_started = Instant::now();
    let inventory = stats(td, -1).await?;
    let mut sweep = Sweep { td, report, inventory_started };
    match scope {
        Clear::All => sweep.optimize(&[], &[], TYPES, 0, 0, Size::read(&inventory)?).await,
        Clear::Chat(id) => {
            for chat in inventory["by_chat"].as_array().ok_or_else(unavailable)? {
                if chat["chat_id"] == id {
                    return sweep.optimize(&[id], &[], TYPES, 0, 0, Size::read(chat)?).await;
                }
            }
            Ok(())
        }
        Clear::Types(types) => {
            let mut estimate = Size::default();
            for chat in inventory["by_chat"].as_array().ok_or_else(unavailable)? {
                for kind in chat["by_file_type"].as_array().ok_or_else(unavailable)? {
                    if kind["file_type"]["@type"].as_str().is_some_and(|kind| types.contains(&kind)) {
                        estimate = estimate.add(Size::read(kind)?);
                    }
                }
            }
            sweep.optimize(&[], &[], &types, 0, 0, estimate).await
        }
    }
}

async fn chat_kind(td: &dyn TdLink, chat_id: i64) -> Option<&'static str> {
    let chat = td.request(json!({"@type":"getChat","chat_id":chat_id})).await.ok()?;
    match chat["type"]["@type"].as_str()? {
        "chatTypePrivate" => Some("private"),
        "chatTypeBasicGroup" => Some("groups"),
        "chatTypeSupergroup" if chat["type"]["is_channel"] == true => Some("channels"),
        "chatTypeSupergroup" => Some("groups"),
        _ => None,
    }
}

pub(crate) async fn auto_download(td: &dyn TdLink, session: &Path, message: &Value) {
    if message["is_outgoing"] == true || message["self_destruct_type"].is_object() { return; }
    let Ok(policy) = settings(session).await else { return; };
    if policy["mode"] != "local" { return; }
    let content = &message["content"];
    let (kind, file) = match content["@type"].as_str() {
        Some("messagePhoto") => {
            let Some(file) = content["photo"]["sizes"].as_array().and_then(|sizes| sizes.last()) else { return; };
            ("photo", &file["photo"])
        }
        Some("messageVideo") => ("video", &content["video"]["video"]),
        Some("messageDocument") => ("file", &content["document"]["document"]),
        Some("messageAudio") => ("file", &content["audio"]["audio"]),
        Some("messageVoiceNote") => ("voice", &content["voice_note"]["voice"]),
        Some("messageVideoNote") => ("video_note", &content["video_note"]["video"]),
        Some("messageAnimation") => ("gif", &content["animation"]["animation"]),
        _ => return,
    };
    if !storage_settings::CHAT_TYPES.iter().any(|chat| policy["auto_download"][chat][kind]["on"] == true) { return; }
    let Some(id) = message["chat_id"].as_i64() else { return; };
    let Some(chat) = chat_kind(td, id).await else { return; };
    let rule = &policy["auto_download"][chat][kind];
    let Some(max) = rule["max_bytes"].as_u64() else { return; };
    let Some(size) = file["size"].as_u64().filter(|&n| n > 0 && n <= max) else { return; };
    let Some(file_id) = file["id"].as_i64().filter(|&n| (1..=i64::from(i32::MAX)).contains(&n)) else { return; };
    if rule["on"] != true || file["local"]["is_downloading_completed"] == true { return; }
    let _ = td.request(json!({"@type":"downloadFile","file_id":file_id,"priority":1,
        "offset":0,"limit":size,"synchronous":true})).await;
}

pub(crate) async fn run(td: &dyn TdLink, policy: &Value, report: &mut Report) -> Result<(), GwError> {
    let max_cache = policy["max_cache_bytes"].as_u64();
    let exceptions = policy["exceptions"].as_array().ok_or_else(settings_error)?;
    let keep = &policy["keep"];
    if policy["mode"] == "local" && max_cache.is_none()
        && storage_settings::CHAT_TYPES.iter().all(|kind| keep[*kind].is_null())
        && exceptions.iter().all(|row| row["keep"].is_null()) { return Ok(()); }
    let inventory_started = Instant::now();
    let inventory = stats(td, -1).await?;
    let total = Size::read(&inventory)?;
    let chats = inventory["by_chat"].as_array().ok_or_else(unavailable)?;
    let mut sweep = Sweep { td, report, inventory_started };
    let no_size_limit = (1 << 53) - 1;
    let (stream, stream_ttl) = (policy["mode"] == "stream", policy["stream_ttl_s"].as_i64().unwrap_or(600) as i32);
    let explicit: BTreeMap<_, _> = exceptions.iter().filter_map(|row| row["chat_id"].as_i64().map(|id| (id, row["keep"].as_i64()))).collect();
    let exclude: Vec<_> = explicit.keys().copied().collect();
    let mut grouped = BTreeMap::<_, Vec<_>>::new();
    for row in chats {
        let id = row["chat_id"].as_i64().ok_or_else(unavailable)?;
        let size = Size::read(row)?;
        let (ttl, exceptions) = if stream { (Some(i64::from(stream_ttl)), false) }
            else if let Some(&ttl) = explicit.get(&id) { (ttl, true) }
            else if id == 0 { (None, false) }
            else { (chat_kind(td, id).await.and_then(|kind| keep[kind].as_i64()), false) };
        if let Some(ttl) = ttl { grouped.entry((ttl as i32, exceptions)).or_default().push((id, size)); }
    }
    for ((ttl, is_exception), group) in grouped {
        let excluded = if stream || is_exception { &[][..] } else { &exclude };
        let mut batch = Vec::new();
        let mut size = Size::default();
        let limit = Size { bytes: MAX_BYTES, files: MAX_FILES };
        for (id, entry) in group {
            // Un chat trop volumineux ne doit pas bloquer les autres.
            if !entry.fits(limit) { sweep.report.limited = true; continue; }
            if !size.add(entry).fits(limit) {
                sweep.optimize(&batch, excluded, TYPES, ttl, no_size_limit, size).await?;
                batch.clear();
                size = Size::default();
            }
            batch.push(id);
            size = size.add(entry);
        }
        sweep.optimize(&batch, excluded, TYPES, ttl, no_size_limit, size).await?;
    }
    if !stream {
        let remaining = total.bytes.saturating_sub(sweep.report.deleted.bytes);
        if let Some(limit) = max_cache.filter(|&limit| remaining > limit) {
            let inventory = stats(td, -1).await?;
            let remaining = Size::read(&inventory)?;
            sweep.optimize(&[], &[], TYPES, i32::MAX, limit, remaining).await?;
        }
    }
    Ok(())
}
