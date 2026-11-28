//! Chính sách cache cho một phiên TDLib. Chỉ công bố số liệu và mã hội thoại.
use std::{collections::BTreeMap, fs, io, path::Path, time::{Instant, SystemTime, UNIX_EPOCH}};

use serde_json::{json, Value};

use crate::contract::{GwError, TdLink};
use crate::storage_settings;

#[cfg(test)]
mod tests;

pub(crate) const MAX_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub(crate) const MAX_FILES: u64 = 5000;
const NO_SIZE_LIMIT: u64 = (1 << 53) - 1;
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

pub(crate) async fn settings_read_only(session: &Path) -> Value {
    let dir = session.to_owned();
    match tokio::task::spawn_blocking(move || storage_settings::load(&dir)).await {
        Ok(Ok(value)) => value,
        Ok(Err(err)) => {
            log::warn!("Cấu hình lưu trữ không đọc được ({:?}); dùng mặc định an toàn.", err.kind());
            storage_settings::safe_default()
        }
        Err(_) => {
            log::warn!("Không đọc được cấu hình lưu trữ; dùng mặc định an toàn.");
            storage_settings::safe_default()
        }
    }
}

pub(crate) fn policy_is_noop(policy: &Value) -> Result<bool, GwError> {
    let exceptions = policy["exceptions"].as_array().ok_or_else(settings_error)?;
    let keep = &policy["keep"];
    Ok(policy["mode"] == "local"
        && policy["max_cache_bytes"].is_null()
        && storage_settings::CHAT_TYPES.iter().all(|kind| keep[*kind].is_null())
        && exceptions.iter().all(|row| row["keep"].is_null()))
}

pub(crate) fn forever_exceptions(policy: &Value) -> Result<Vec<i64>, GwError> {
    let exceptions = policy["exceptions"].as_array().ok_or_else(settings_error)?;
    Ok(exceptions.iter()
        .filter_map(|row| row["keep"].is_null().then(|| row["chat_id"].as_i64()).flatten())
        .collect())
}

pub(crate) async fn check_cache_paths(session: &Path) -> Result<(), GwError> {
    let session = session.to_owned();
    let root = session.join("files");
    let unsafe_path = || GwError::new(409, "STORAGE_CACHE_PATH", "Thư mục bộ nhớ đệm có đường dẫn chuyển hướng hoặc không đọc được.");
    tokio::task::spawn_blocking(move || {
        // TDLib có thể mở thư mục loại tệp gốc qua symlink. Không quét/xoá khi thấy
        // đường dẫn chuyển hướng trong phiên riêng này.
        for path in [&session, &root] {
            let kind = fs::symlink_metadata(path)?.file_type();
            if kind.is_symlink() || !kind.is_dir() {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        let mut directories = vec![root.canonicalize()?];
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

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Size { bytes: u64, files: u64 }

impl Size {
    fn read(value: &Value) -> Result<Self, GwError> {
        Ok(Self { bytes: value["size"].as_u64().ok_or_else(unavailable)?,
            files: value["count"].as_u64().ok_or_else(unavailable)? })
    }
    fn add(self, next: Self) -> Self {
        Self { bytes: self.bytes.saturating_add(next.bytes), files: self.files.saturating_add(next.files) }
    }
    fn sub(self, used: Self) -> Self {
        Self { bytes: self.bytes.saturating_sub(used.bytes), files: self.files.saturating_sub(used.files) }
    }
    fn fits(self, limit: Self) -> bool { self.bytes <= limit.bytes && self.files <= limit.files }
    fn is_empty(self) -> bool { self.bytes == 0 && self.files == 0 }
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
    remaining: Size,
    error: Option<String>,
    limited: bool,
    policy_changed: bool,
}

impl Report {
    pub(crate) fn new() -> Self {
        Self { last_run: None, deleted: Size::default(), remaining: Size::default(), error: None, limited: false, policy_changed: false }
    }
    pub(crate) fn policy_changed(&self) -> bool { self.policy_changed }
    pub(crate) fn json(&self) -> Value {
        json!({
            "last_run": self.last_run,
            "freed_bytes": self.deleted.bytes,
            "freed_files": self.deleted.files,
            "remaining_bytes": self.remaining.bytes,
            "remaining_files": self.remaining.files,
            "error": self.error,
            "limited": self.limited,
            "policy_changed": self.policy_changed,
        })
    }
    pub(crate) fn failed(&mut self, code: impl Into<String>) { self.error = Some(code.into()); }
    pub(crate) fn finish(&mut self) {
        self.last_run = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs());
        if self.policy_changed {
            log::info!(
                "storage dừng lượt dọn do cấu hình thay đổi; đã giải phóng {} byte ({} tệp) failed={}",
                self.deleted.bytes, self.deleted.files, self.error.is_some()
            );
        } else if self.limited {
            log::info!(
                "storage đã giải phóng {} byte ({} tệp), còn {} byte ({} tệp), sẽ tiếp tục failed={}",
                self.deleted.bytes, self.deleted.files, self.remaining.bytes, self.remaining.files, self.error.is_some()
            );
        } else {
            log::info!(
                "storage đã giải phóng {} byte ({} tệp) limited=false failed={}",
                self.deleted.bytes, self.deleted.files, self.error.is_some()
            );
        }
    }
}

struct Sweep<'a> {
    td: &'a dyn TdLink,
    report: &'a mut Report,
    inventory_started: Instant,
    cancelled: &'a (dyn Fn() -> bool + Send + Sync),
}

impl Sweep<'_> {
    fn is_cancelled(&mut self) -> bool {
        if (self.cancelled)() {
            self.report.policy_changed = true;
            return true;
        }
        false
    }

    fn budget_left(&self) -> Size {
        Size {
            bytes: MAX_BYTES.saturating_sub(self.report.deleted.bytes),
            files: MAX_FILES.saturating_sub(self.report.deleted.files),
        }
    }

    fn budget_exhausted(&self) -> bool {
        self.report.deleted.bytes >= MAX_BYTES || self.report.deleted.files >= MAX_FILES
    }

    async fn send_optimize(
        &mut self,
        chats: &[i64],
        exclude: &[i64],
        types: &[&str],
        ttl: i32,
        size: u64,
        count: i32,
    ) -> Result<Size, GwError> {
        if self.is_cancelled() { return Ok(Size::default()); }
        // Bảo vệ cả tệp mới hoàn tất trong lúc chụp thống kê hoặc TDLib quét lại.
        let immunity = (self.inventory_started.elapsed() + crate::td::REQUEST_TIMEOUT)
            .as_secs().saturating_add(1).min(i32::MAX as u64);
        let result = self.td.request(json!({
            "@type": "optimizeStorage",
            "chat_ids": chats,
            "exclude_chat_ids": exclude,
            "file_types": types.iter().map(|kind| json!({"@type": kind})).collect::<Vec<_>>(),
            "size": size,
            "ttl": ttl,
            "count": count,
            "immunity_delay": immunity,
            "return_deleted_file_statistics": true,
            "chat_limit": 0
        })).await.map_err(|_| GwError::new(503, "STORAGE_CLEANUP", "Telegram chưa dọn được bộ nhớ đệm."))?;
        let deleted = Size::read(&result)?;
        self.report.deleted = self.report.deleted.add(deleted);
        if self.budget_exhausted() {
            self.report.limited = true;
        }
        Ok(deleted)
    }

    async fn clear_scope(
        &mut self,
        chats: &[i64],
        exclude: &[i64],
        types: &[&str],
        total: Size,
        scope: Size,
    ) -> Result<(), GwError> {
        if self.is_cancelled() || scope.is_empty() { return Ok(()); }
        let budget = self.budget_left();
        if budget.bytes == 0 || budget.files == 0 {
            self.report.limited = true;
            self.report.remaining = self.report.remaining.add(scope);
            return Ok(());
        }
        let (deleted, capped) = if scope.fits(budget) {
            (self.send_optimize(chats, exclude, types, 0, 0, i32::MAX).await?, false)
        } else if u128::from(budget.files) * u128::from(scope.bytes) <= u128::from(budget.bytes) * u128::from(scope.files) {
            // Giới hạn theo số tệp mỗi lượt; TDLib nhận đích tổng còn lại sau khi xoá.
            let keep_files = total.files.saturating_sub(budget.files).min(i32::MAX as u64) as i32;
            (self.send_optimize(chats, exclude, types, i32::MAX, NO_SIZE_LIMIT, keep_files).await?, true)
        } else {
            let keep_bytes = total.bytes.saturating_sub(budget.bytes);
            (self.send_optimize(chats, exclude, types, i32::MAX, keep_bytes, i32::MAX).await?, true)
        };
        if capped && !deleted.is_empty() {
            let left = scope.sub(deleted);
            if !left.is_empty() {
                self.report.limited = true;
                self.report.remaining = self.report.remaining.add(left);
            }
        }
        Ok(())
    }

    async fn optimize_ttl(&mut self, chats: &[i64], exclude: &[i64], types: &[&str], ttl: i32) -> Result<Size, GwError> {
        self.send_optimize(chats, exclude, types, ttl, NO_SIZE_LIMIT, i32::MAX).await
    }

    async fn trim_cache(
        &mut self,
        exclude: &[i64],
        types: &[&str],
        total: Size,
        deletable: Size,
        limit: u64,
    ) -> Result<(), GwError> {
        if self.is_cancelled() { return Ok(()); }
        let excess_bytes = total.bytes.saturating_sub(limit);
        if excess_bytes == 0 || deletable.files == 0 { return Ok(()); }
        let budget = self.budget_left();
        if budget.bytes == 0 || budget.files == 0 {
            self.report.limited = true;
            self.report.remaining.bytes = self.report.remaining.bytes.max(excess_bytes);
            return Ok(());
        }
        let to_delete = excess_bytes.min(deletable.bytes);
        let step_bytes = to_delete.min(budget.bytes);
        let file_tight = deletable.files > budget.files
            && u128::from(step_bytes) * u128::from(deletable.files) > u128::from(budget.files) * u128::from(deletable.bytes);
        let step = if file_tight {
            let est_bytes = (u128::from(budget.files) * u128::from(deletable.bytes) / u128::from(deletable.files)).max(1) as u64;
            step_bytes.min(est_bytes)
        } else {
            step_bytes
        };
        let target_after = total.bytes.saturating_sub(step);
        let (deleted, capped) = (
            self.send_optimize(&[], exclude, types, i32::MAX, target_after, i32::MAX).await?,
            step < to_delete,
        );
        if capped && !deleted.is_empty() {
            let left_bytes = to_delete.saturating_sub(deleted.bytes);
            if left_bytes > 0 {
                self.report.limited = true;
                self.report.remaining.bytes = self.report.remaining.bytes.saturating_add(left_bytes);
            }
        }
        Ok(())
    }
}

pub(crate) async fn clear(td: &dyn TdLink, scope: Clear, report: &mut Report) -> Result<(), GwError> {
    clear_with_policy(td, scope, &[], report, &|| false).await
}

pub(crate) async fn clear_with_policy(
    td: &dyn TdLink,
    scope: Clear,
    forever: &[i64],
    report: &mut Report,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<(), GwError> {
    if cancelled() {
        report.policy_changed = true;
        return Ok(());
    }
    let inventory_started = Instant::now();
    let inventory = stats(td, -1).await?;
    if cancelled() {
        report.policy_changed = true;
        return Ok(());
    }
    let total = Size::read(&inventory)?;
    let mut sweep = Sweep { td, report, inventory_started, cancelled };
    match scope {
        Clear::All => {
            let mut excluded_size = Size::default();
            for chat in inventory["by_chat"].as_array().ok_or_else(unavailable)? {
                if chat["chat_id"].as_i64().is_some_and(|id| forever.contains(&id)) {
                    excluded_size = excluded_size.add(Size::read(chat)?);
                }
            }
            if !forever.is_empty() {
                log::info!("storage xoá thủ công toàn bộ bỏ qua {} hội thoại giữ mãi mãi", forever.len());
            }
            sweep.clear_scope(&[], forever, TYPES, total, total.sub(excluded_size)).await
        }
        Clear::Chat(id) => {
            for chat in inventory["by_chat"].as_array().ok_or_else(unavailable)? {
                if chat["chat_id"] == id {
                    return sweep.clear_scope(&[id], &[], TYPES, total, Size::read(chat)?).await;
                }
            }
            Ok(())
        }
        Clear::Types(types) => {
            let mut estimate = Size::default();
            for chat in inventory["by_chat"].as_array().ok_or_else(unavailable)? {
                if chat["chat_id"].as_i64().is_some_and(|id| forever.contains(&id)) {
                    continue;
                }
                for kind in chat["by_file_type"].as_array().ok_or_else(unavailable)? {
                    if kind["file_type"]["@type"].as_str().is_some_and(|kind| types.contains(&kind)) {
                        estimate = estimate.add(Size::read(kind)?);
                    }
                }
            }
            if !forever.is_empty() {
                log::info!("storage xoá thủ công theo loại bỏ qua {} hội thoại giữ mãi mãi", forever.len());
            }
            sweep.clear_scope(&[], forever, &types, total, estimate).await
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
    let policy = settings_read_only(session).await;
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

struct ChatEntry {
    id: i64,
    size: Size,
    by_type: Vec<(&'static str, Size)>,
}

fn file_type_sizes(row: &Value) -> Vec<(&'static str, Size)> {
    let Some(items) = row["by_file_type"].as_array() else { return Vec::new(); };
    let mut out = Vec::new();
    for item in items {
        let Some(kind) = item["file_type"]["@type"].as_str() else { continue; };
        let Some(&allowed) = TYPES.iter().find(|&&t| t == kind) else { continue; };
        if let Ok(size) = Size::read(item) {
            if !size.is_empty() { out.push((allowed, size)); }
        }
    }
    out
}

fn add_remaining(report: &mut Report, groups: &[((i32, bool), Vec<ChatEntry>)], start_group: usize, start_chat: usize) {
    let mut rest = Size::default();
    for (gi, (_, group)) in groups.iter().enumerate().skip(start_group) {
        let skip_chat = if gi == start_group { start_chat } else { 0 };
        for entry in group.iter().skip(skip_chat) {
            rest = rest.add(entry.size);
        }
    }
    if !rest.is_empty() {
        report.limited = true;
        report.remaining = report.remaining.add(rest);
    }
}

pub(crate) async fn run(td: &dyn TdLink, policy: &Value, report: &mut Report) -> Result<(), GwError> {
    run_cancellable(td, policy, report, &|| false).await
}

pub(crate) async fn run_cancellable(
    td: &dyn TdLink,
    policy: &Value,
    report: &mut Report,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<(), GwError> {
    if policy_is_noop(policy)? { return Ok(()); }
    if cancelled() {
        report.policy_changed = true;
        return Ok(());
    }
    let max_cache = policy["max_cache_bytes"].as_u64();
    let exceptions = policy["exceptions"].as_array().ok_or_else(settings_error)?;
    let keep = &policy["keep"];
    let inventory_started = Instant::now();
    let inventory = stats(td, -1).await?;
    if cancelled() {
        report.policy_changed = true;
        return Ok(());
    }
    let total = Size::read(&inventory)?;
    let chats = inventory["by_chat"].as_array().ok_or_else(unavailable)?;
    let mut sweep = Sweep { td, report, inventory_started, cancelled };
    let (stream, stream_ttl) = (policy["mode"] == "stream", policy["stream_ttl_s"].as_i64().unwrap_or(600) as i32);
    let explicit: BTreeMap<_, _> = exceptions.iter()
        .filter_map(|row| row["chat_id"].as_i64().map(|id| (id, row["keep"].as_i64())))
        .collect();
    let exclude: Vec<_> = explicit.keys().copied().collect();
    let forever: Vec<_> = explicit.iter().filter_map(|(&id, ttl)| ttl.is_none().then_some(id)).collect();
    let mut grouped = BTreeMap::<_, Vec<ChatEntry>>::new();
    for row in chats {
        if sweep.is_cancelled() { return Ok(()); }
        let id = row["chat_id"].as_i64().ok_or_else(unavailable)?;
        let size = Size::read(row)?;
        let by_type = file_type_sizes(row);
        // Ngoại lệ theo từng chat (đặc biệt «Mãi mãi») được ưu tiên ở cả chế độ stream và local.
        let (ttl, is_exception) = if let Some(&ttl) = explicit.get(&id) {
            (ttl, true)
        } else if stream {
            (Some(i64::from(stream_ttl)), false)
        } else if id == 0 {
            (None, false)
        } else {
            (chat_kind(td, id).await.and_then(|kind| keep[kind].as_i64()), false)
        };
        if let Some(ttl) = ttl {
            grouped.entry((ttl as i32, is_exception)).or_default().push(ChatEntry { id, size, by_type });
        }
    }
    let groups: Vec<_> = grouped.into_iter().collect();
    'groups: for (gi, ((ttl, is_exception), group)) in groups.iter().enumerate() {
        if sweep.is_cancelled() { return Ok(()); }
        let excluded = if *is_exception { &[][..] } else { &exclude };
        let mut batch = Vec::new();
        let mut batch_size = Size::default();
        for (ci, entry) in group.iter().enumerate() {
            if sweep.is_cancelled() { return Ok(()); }
            if sweep.budget_exhausted() {
                add_remaining(sweep.report, &groups, gi, ci);
                break 'groups;
            }
            if !batch.is_empty() && !batch_size.add(entry.size).fits(sweep.budget_left()) {
                sweep.optimize_ttl(&batch, excluded, TYPES, *ttl).await?;
                batch.clear();
                batch_size = Size::default();
                if sweep.is_cancelled() { return Ok(()); }
                if sweep.budget_exhausted() {
                    add_remaining(sweep.report, &groups, gi, ci);
                    break 'groups;
                }
            }
            if entry.size.fits(sweep.budget_left()) {
                batch.push(entry.id);
                batch_size = batch_size.add(entry.size);
            } else {
                // Không bỏ qua chat lớn; tách theo từng loại tệp nếu có chi tiết, hoặc dọn trực tiếp rồi xét lượng đã xoá.
                if entry.by_type.len() > 1 {
                    let mut type_batch = Vec::new();
                    let mut type_size = Size::default();
                    for (ti, &(kind, size)) in entry.by_type.iter().enumerate() {
                        if sweep.is_cancelled() { return Ok(()); }
                        if sweep.budget_exhausted() {
                            let mut left = Size::default();
                            for &(_, s) in &entry.by_type[ti..] { left = left.add(s); }
                            sweep.report.limited = true;
                            sweep.report.remaining = sweep.report.remaining.add(left);
                            add_remaining(sweep.report, &groups, gi, ci + 1);
                            break 'groups;
                        }
                        let left_budget = sweep.budget_left();
                        if !type_batch.is_empty() && !type_size.add(size).fits(left_budget) {
                            sweep.optimize_ttl(&[entry.id], excluded, &type_batch, *ttl).await?;
                            type_batch.clear();
                            type_size = Size::default();
                            if sweep.is_cancelled() { return Ok(()); }
                            if sweep.budget_exhausted() {
                                let mut left = Size::default();
                                for &(_, s) in &entry.by_type[ti..] { left = left.add(s); }
                                sweep.report.limited = true;
                                sweep.report.remaining = sweep.report.remaining.add(left);
                                add_remaining(sweep.report, &groups, gi, ci + 1);
                                break 'groups;
                            }
                        }
                        type_batch.push(kind);
                        type_size = type_size.add(size);
                    }
                    if !type_batch.is_empty() {
                        if sweep.is_cancelled() { return Ok(()); }
                        sweep.optimize_ttl(&[entry.id], excluded, &type_batch, *ttl).await?;
                    }
                } else {
                    if sweep.is_cancelled() { return Ok(()); }
                    sweep.optimize_ttl(&[entry.id], excluded, TYPES, *ttl).await?;
                }
                if sweep.is_cancelled() { return Ok(()); }
                if sweep.budget_exhausted() {
                    add_remaining(sweep.report, &groups, gi, ci + 1);
                    break 'groups;
                }
            }
        }
        if !batch.is_empty() {
            if sweep.is_cancelled() { return Ok(()); }
            sweep.optimize_ttl(&batch, excluded, TYPES, *ttl).await?;
            if sweep.is_cancelled() { return Ok(()); }
            if sweep.budget_exhausted() {
                add_remaining(sweep.report, &groups, gi + 1, 0);
                break 'groups;
            }
        }
    }
    if sweep.is_cancelled() { return Ok(()); }
    if !stream {
        let remaining_est = total.bytes.saturating_sub(sweep.report.deleted.bytes);
        if let Some(limit) = max_cache.filter(|&limit| remaining_est > limit) {
            if sweep.budget_exhausted() {
                sweep.report.limited = true;
                sweep.report.remaining.bytes = sweep.report.remaining.bytes.max(remaining_est - limit);
            } else {
                let inventory = stats(td, -1).await?;
                if sweep.is_cancelled() { return Ok(()); }
                let current_total = Size::read(&inventory)?;
                let mut excluded_size = Size::default();
                for row in inventory["by_chat"].as_array().ok_or_else(unavailable)? {
                    if let Some(id) = row["chat_id"].as_i64() {
                        if forever.contains(&id) {
                            excluded_size = excluded_size.add(Size::read(row)?);
                        }
                    }
                }
                let deletable = current_total.sub(excluded_size);
                sweep.trim_cache(&forever, TYPES, current_total, deletable, limit).await?;
            }
        }
    }
    Ok(())
}
