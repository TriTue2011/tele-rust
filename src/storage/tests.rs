use std::{fs::{self, File, FileTimes}, path::PathBuf, sync::{Arc, Mutex}, time::Duration};

use super::*;
use crate::contract::{BoxFut, TdError};

struct Cache {
    path: PathBuf,
    now: Mutex<SystemTime>,
    downloads: Mutex<usize>,
}

impl TdLink for Cache {
    fn request(&self, request: Value) -> BoxFut<'_, Result<Value, TdError>> {
        Box::pin(async move {
            match request["@type"].as_str().unwrap() {
                "downloadFile" => {
                    *self.downloads.lock().unwrap() += 1;
                    Ok(json!({"@type":"file","local":{"is_downloading_completed":true,"path":self.path}}))
                }
                "getStorageStatistics" => Ok(json!({"@type":"storageStatistics","size":8,"count":1,
                    "by_chat":[{"chat_id":7,"size":8,"count":1,"by_file_type":[]}]})),
                "optimizeStorage" => {
                    let meta = fs::metadata(&self.path).unwrap();
                    let timestamp = meta.accessed().unwrap().max(meta.modified().unwrap());
                    let age = self.now.lock().unwrap().duration_since(timestamp).unwrap();
                    assert_eq!(request["chat_ids"], json!([7]));
                    if age > Duration::from_secs(request["ttl"].as_u64().unwrap()) {
                        fs::remove_file(&self.path).unwrap();
                        Ok(json!({"@type":"storageStatistics","size":8,"count":1,"by_chat":[]}))
                    } else {
                        Ok(json!({"@type":"storageStatistics","size":0,"count":0,"by_chat":[]}))
                    }
                }
                method => panic!("unexpected request {method}"),
            }
        })
    }

    fn updates(&self) -> tokio::sync::broadcast::Receiver<Value> { tokio::sync::broadcast::channel(1).1 }
}

#[tokio::test]
async fn stream_access_refreshes_expiry_and_skips_auto_download() {
    let session = std::env::temp_dir().join(format!("tggw-storage-stream-{}", std::process::id()));
    let dir = session.join("files/photos");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("example.jpg");
    fs::write(&path, b"test pic").unwrap();
    let past = SystemTime::now() - Duration::from_secs(900);
    File::open(&path).unwrap().set_times(FileTimes::new().set_modified(past).set_accessed(past)).unwrap();
    let cache = Arc::new(Cache { path: path.clone(), now: Mutex::new(SystemTime::now()), downloads: Mutex::new(0) });
    let mut policy = storage_settings::load(&session).unwrap();
    policy["mode"] = json!("stream");
    policy["auto_download"]["private"]["photo"]["on"] = json!(true);
    storage_settings::save(&session, &policy).unwrap();
    auto_download(cache.as_ref(), &session, &json!({"chat_id":7,"content":{"@type":"messagePhoto","photo":{
        "sizes":[{"photo":{"id":8,"size":8}}]}}})).await;
    assert_eq!(*cache.downloads.lock().unwrap(), 0);

    let file = crate::account_files::download(cache.clone(), session.clone(), 8, true).await.unwrap();
    assert_eq!(file.cache_control, "no-store");
    assert_eq!(file.bytes, b"test pic");
    let last_access = fs::metadata(&path).unwrap().accessed().unwrap();
    assert!(last_access > past + Duration::from_secs(600));
    *cache.now.lock().unwrap() = last_access + Duration::from_secs(599);
    let mut report = Report::new();
    run(cache.as_ref(), &policy, &mut report).await.unwrap();
    assert!(path.exists(), "lượt xem lại phải làm mới hạn");
    *cache.now.lock().unwrap() = last_access + Duration::from_secs(601);
    run(cache.as_ref(), &policy, &mut report).await.unwrap();
    assert!(!path.exists(), "tệp hết hạn phải được dọn");
    assert_eq!(report.json()["freed_bytes"], 8);
    fs::remove_dir_all(session).unwrap();
}
