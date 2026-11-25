use std::{fs::File, io::{self, Read}, path::{Component, Path}};

use rustix::fs::{open, openat, Mode, OFlags};
use serde_json::json;

use super::{path_id, telegram_error, Binary, ChatService, GwError, Query};

pub(super) const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const TD_FILES: &str = "tdlib/files";
const UPLOADS: &str = "uploads";

impl ChatService {
    pub(super) async fn binary(&self, path: String, query: String) -> Result<Binary, GwError> {
        if let Some(id) = path.strip_prefix("/api/file/") {
            self.account(&Query(&query).get("account").unwrap_or_default())?;
            let id = path_id(id)?;
            return self.download(id).await;
        }
        if let Some(id) = path.strip_prefix("/api/calltrace/") {
            let missing = || GwError::not_found("Không có nhật ký cho cuộc gọi này.");
            if !crate::calls::trace::valid_id(id) {
                return Err(missing());
            }
            let bytes = read_bounded(self.data_dir.clone(), format!("calltrace/{id}.log").into(), missing).await?;
            return Ok(Binary {
                bytes: String::from_utf8_lossy(&bytes).into_owned().into_bytes(),
                content_type: "text/plain; charset=utf-8".into(),
                cache_control: "no-store",
                disposition: Some(format!("attachment; filename=\"calltrace-{id}.log\"")),
            });
        }
        Err(GwError::not_found("Không có đường dẫn này."))
    }

    async fn download(&self, id: i64) -> Result<Binary, GwError> {
        let missing = || GwError::not_found("Tệp chưa tải xong.");
        if !(1..=i64::from(i32::MAX)).contains(&id) { return Err(missing()); }
        let file = self.td.request(json!({"@type":"downloadFile","file_id":id,
            "priority":16,"offset":0,"limit":0,"synchronous":true})).await.map_err(|mut error| {
            // Không gửi đường dẫn từ thông báo downloadFile. Giữ FLOOD_WAIT đã chuẩn hoá.
            let mut api_error = telegram_error(error.clone());
            if error.code.is_some() && api_error.retry_after_seconds.is_none() {
                error.message.clear();
                api_error = telegram_error(error);
            }
            api_error
        })?;
        let local = &file["local"];
        let path = local["path"].as_str().filter(|s| !s.is_empty()).ok_or_else(missing)?;
        if local["is_downloading_completed"] != true { return Err(missing()); }

        let root = std::path::absolute(&self.data_dir).map_err(|_| missing())?;
        let path = std::path::absolute(path).map_err(|_| missing())?;
        // Tệp TỰ GỬI (đo thật 25/11: 7/7 video tự gửi 404): TDLib không copy vào kho tdlib/files/ của nó, mà GIỮ
        // NGUYÊN đường dẫn gốc đã dùng lúc gửi (uploads/chat-...) — strip_prefix(TD_FILES) luôn trượt nên luôn
        // "missing" dù tệp còn nguyên trên đĩa (xác nhận bằng cách so cỡ byte: 7/7 tệp trong uploads/ khớp đúng
        // size của 7 tin outgoing đang 404). Thử cả hai gốc hợp lệ; tdlib/files/ giữ nguyên yêu cầu "ít nhất 2 cấp
        // thư mục" (tệp DB ở gốc không phục vụ); uploads/ là tệp phẳng (chat-....ext ngay trong thư mục) nên không
        // áp yêu cầu đó.
        let (sub, relative) = match path.strip_prefix(root.join(TD_FILES)) {
            Ok(relative) => {
                if relative.components().count() < 2 { return Err(missing()); }
                (TD_FILES, relative.to_path_buf())
            }
            Err(_) => match path.strip_prefix(root.join(UPLOADS)) {
                Ok(relative) => (UPLOADS, relative.to_path_buf()),
                Err(_) => return Err(missing()),
            },
        };
        let mime = mime_guess::from_path(&relative).first_raw().unwrap_or("text/plain");
        let content_type = if mime.starts_with("text/") { format!("{mime}; charset=utf-8") } else { mime.into() };
        let bytes = read_bounded(root, Path::new(sub).join(relative), missing).await?;
        Ok(Binary {
            bytes, content_type, cache_control: "private, max-age=86400", disposition: None,
        })
    }
}

async fn read_bounded(root: std::path::PathBuf, relative: std::path::PathBuf,
    missing: fn() -> GwError) -> Result<Vec<u8>, GwError> {
    tokio::task::spawn_blocking(move || {
        let too_large = || GwError::new(413, "FILE_TOO_LARGE", "Tệp vượt 64 MB.");
        let file = open_beneath(&root, &relative).map_err(|_| missing())?;
        let meta = file.metadata().map_err(|_| missing())?;
        if !meta.is_file() { return Err(missing()); }
        if meta.len() > MAX_FILE_BYTES { return Err(too_large()); }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|_| missing())?;
        if bytes.len() as u64 > MAX_FILE_BYTES { return Err(too_large()); }
        Ok(bytes)
    }).await.map_err(|_| missing())?
}

fn open_beneath(root: &Path, path: &Path) -> io::Result<File> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    let dir_flags = flags | OFlags::DIRECTORY;
    let mut dir = open(root, dir_flags, Mode::empty())?;
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else { break };
        if components.peek().is_none() {
            return Ok(openat(&dir, name, flags, Mode::empty())?.into());
        }
        dir = openat(&dir, name, dir_flags, Mode::empty())?;
    }
    Err(io::ErrorKind::InvalidInput.into())
}
