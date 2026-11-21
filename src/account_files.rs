//! Tải tệp từ đúng phiên đã chọn, giữ giới hạn đọc và chống đi theo symlink.
use std::{fs::File,io::{self,Read},path::{Component,Path,PathBuf},sync::Arc};
use rustix::fs::{open,openat,Mode,OFlags};
use serde_json::json;
use crate::contract::{Binary,GwError,TdLink};

pub(crate) async fn download(td:Arc<dyn TdLink>,session:PathBuf,id:i64)->Result<Binary,GwError> {
    let missing=||GwError::not_found("Tệp chưa tải xong.");
    if !(1..=i64::from(i32::MAX)).contains(&id) {return Err(missing());}
    let file=td.request(json!({"@type":"downloadFile","file_id":id,"priority":16,"offset":0,"limit":0,"synchronous":true})).await.map_err(|mut error| {
        if error.code.is_none() && error.message.starts_with("TDLib không trả lời ") {
            return GwError::new(504,"TELEGRAM_TIMEOUT","Telegram không trả lời kịp.");
        }
        if error.code.is_none() && error.message.starts_with("mất kết nối TDLib") {
            return GwError::new(503,"ACCOUNT_OFFLINE","Tài khoản đang mất kết nối tới Telegram.");
        }
        let mapped=GwError::from_tdlib(error.clone());
        if error.code.is_some() && mapped.retry_after_seconds.is_none() {
            error.message.clear();
            GwError::from_tdlib(error)
        } else {mapped}
    })?;
    if file["local"]["is_downloading_completed"]!=true {return Err(missing());}
    let root=std::path::absolute(session.join("files")).map_err(|_|missing())?;
    let path=file["local"]["path"].as_str().ok_or_else(missing)?;
    let path=std::path::absolute(path).map_err(|_|missing())?;
    let relative=path.strip_prefix(&root).map_err(|_|missing())?.to_path_buf();
    if relative.components().count()<2 {return Err(missing());}
    let mime=mime_guess::from_path(&relative).first_raw().unwrap_or("text/plain");
    let content_type=if mime.starts_with("text/"){format!("{mime}; charset=utf-8")}else{mime.into()};
    let bytes=tokio::task::spawn_blocking(move || {
        let file=open_beneath(&root,&relative).map_err(|_|missing())?;
        if !file.metadata().map_err(|_|missing())?.is_file() {return Err(missing());}
        const LIMIT:u64=64*1024*1024;
        let mut bytes=Vec::new();file.take(LIMIT+1).read_to_end(&mut bytes).map_err(|_|missing())?;
        if bytes.len() as u64>LIMIT {return Err(GwError::new(413,"FILE_TOO_LARGE","Tệp vượt 64 MB."));}
        Ok(bytes)
    }).await.map_err(|_|missing())??;
    Ok(Binary{bytes,content_type,cache_control:"private, max-age=86400",disposition:None})
}

fn open_beneath(root:&Path,relative:&Path)->io::Result<File> {
    let flags=OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::CLOEXEC|OFlags::NONBLOCK;
    let mut dir=open(root,flags|OFlags::DIRECTORY,Mode::empty())?;
    let mut parts=relative.components().peekable();
    while let Some(Component::Normal(name))=parts.next() {
        if parts.peek().is_none() {return Ok(openat(&dir,name,flags,Mode::empty())?.into());}
        dir=openat(&dir,name,flags|OFlags::DIRECTORY,Mode::empty())?;
    }
    Err(io::ErrorKind::InvalidInput.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{BoxFut,TdError};
    use serde_json::Value;

    struct Fake(Result<Value,TdError>);
    impl TdLink for Fake {
        fn request(&self,_:Value)->BoxFut<'_,Result<Value,TdError>> {Box::pin(async {self.0.clone()})}
        fn updates(&self)->tokio::sync::broadcast::Receiver<Value> {tokio::sync::broadcast::channel(1).1}
    }
    #[tokio::test]
    async fn selected_session_cannot_download_another_account_or_database_or_symlink() {
        let root=PathBuf::from(format!("target/account-files-{}",std::process::id()));
        let a=root.join("a");let b=root.join("b");
        for session in [&a,&b] {std::fs::create_dir_all(session.join("files/documents")).unwrap();}
        std::fs::write(a.join("files/documents/test.txt"),b"account a").unwrap();
        std::fs::write(b.join("files/documents/test.txt"),b"account b").unwrap();
        std::fs::write(a.join("files/td.binlog"),b"database").unwrap();
        std::os::unix::fs::symlink(std::path::absolute(b.join("files/documents/test.txt")).unwrap(),a.join("files/documents/link.txt")).unwrap();
        for path in [b.join("files/documents/test.txt"),a.join("files/td.binlog"),a.join("files/documents/link.txt")] {
            let td=Arc::new(Fake(Ok(json!({"local":{"path":std::path::absolute(path).unwrap(),"is_downloading_completed":true}}))));
            assert_eq!(download(td,a.clone(),1).await.unwrap_err().status,404);
        }
        let td=Arc::new(Fake(Ok(json!({"local":{"path":std::path::absolute(a.join("files/documents/test.txt")).unwrap(),"is_downloading_completed":true}}))));
        let file=download(td,a.clone(),1).await.unwrap();
        assert_eq!(file.bytes,b"account a");assert_eq!(file.content_type,"text/plain; charset=utf-8");
        let td=Arc::new(Fake(Err(TdError::telegram(400,"/private/session/path"))));
        let error=download(td,a,1).await.unwrap_err();
        assert_eq!(error.status,400);assert!(!error.message.contains("/private/session/path"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
