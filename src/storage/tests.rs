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

struct FakeFile {
    chat_id: i64,
    file_type: &'static str,
    bytes: u64,
    age_s: u64,
}

#[derive(Default)]
struct FakeStore {
    files: Mutex<Vec<FakeFile>>,
    chats: Mutex<BTreeMap<i64, Value>>,
    sent: Mutex<Vec<Value>>,
    downloads: Mutex<Vec<i64>>,
}

impl TdLink for FakeStore {
    fn request(&self, request: Value) -> BoxFut<'_, Result<Value, TdError>> {
        Box::pin(async move {
            match request["@type"].as_str().unwrap() {
                "getChat" => {
                    let chat_id = request["chat_id"].as_i64().unwrap();
                    let kind = self.chats.lock().unwrap().get(&chat_id).cloned()
                        .unwrap_or_else(|| json!({"@type":"chatTypePrivate"}));
                    Ok(json!({"@type":"chat","type":kind}))
                }
                "downloadFile" => {
                    self.downloads.lock().unwrap().push(request["file_id"].as_i64().unwrap());
                    Ok(json!({"@type":"file","local":{"is_downloading_completed":true,"path":""}}))
                }
                "getStorageStatistics" => {
                    let files = self.files.lock().unwrap();
                    let mut total_bytes = 0_u64;
                    let mut total_files = 0_u64;
                    let mut by_chat: BTreeMap<i64, (u64, u64, BTreeMap<&'static str, (u64, u64)>)> = BTreeMap::new();
                    for file in files.iter() {
                        total_bytes += file.bytes;
                        total_files += 1;
                        let entry = by_chat.entry(file.chat_id).or_default();
                        entry.0 += file.bytes;
                        entry.1 += 1;
                        let kind = entry.2.entry(file.file_type).or_default();
                        kind.0 += file.bytes;
                        kind.1 += 1;
                    }
                    let chats = by_chat.into_iter().map(|(chat_id, (size, count, types))| {
                        let by_file_type = types.into_iter().map(|(ft, (b, c))| {
                            json!({"file_type":{"@type":ft},"size":b,"count":c})
                        }).collect::<Vec<_>>();
                        json!({"chat_id":chat_id,"size":size,"count":count,"by_file_type":by_file_type})
                    }).collect::<Vec<_>>();
                    Ok(json!({"@type":"storageStatistics","size":total_bytes,"count":total_files,"by_chat":chats}))
                }
                "optimizeStorage" => {
                    self.sent.lock().unwrap().push(request.clone());
                    let chat_ids: Vec<i64> = request["chat_ids"].as_array().unwrap().iter().filter_map(Value::as_i64).collect();
                    let exclude: Vec<i64> = request["exclude_chat_ids"].as_array().unwrap().iter().filter_map(Value::as_i64).collect();
                    let types: Vec<String> = request["file_types"].as_array().unwrap().iter()
                        .filter_map(|t| t["@type"].as_str().map(str::to_owned)).collect();
                    let ttl = request["ttl"].as_u64().unwrap();
                    let target_size = request["size"].as_u64().unwrap();
                    let target_count = request["count"].as_u64().unwrap();
                    let mut files = self.files.lock().unwrap();
                    let mut total_bytes: u64 = files.iter().map(|f| f.bytes).sum();
                    let mut total_files: u64 = files.len() as u64;
                    let eligible = |f: &FakeFile| {
                        (chat_ids.is_empty() || chat_ids.contains(&f.chat_id))
                            && !exclude.contains(&f.chat_id)
                            && types.iter().any(|t| t == f.file_type)
                    };
                    let mut indices: Vec<usize> = files.iter().enumerate()
                        .filter(|(_, f)| eligible(f)).map(|(i, _)| i).collect();
                    indices.sort_by_key(|&i| std::cmp::Reverse(files[i].age_s));
                    let mut removed = vec![false; files.len()];
                    let mut deleted_bytes = 0_u64;
                    let mut deleted_files = 0_u64;
                    for &i in &indices {
                        if files[i].age_s > ttl {
                            removed[i] = true;
                            total_bytes -= files[i].bytes;
                            total_files -= 1;
                            deleted_bytes += files[i].bytes;
                            deleted_files += 1;
                        }
                    }
                    for &i in &indices {
                        if removed[i] { continue; }
                        if total_bytes <= target_size && total_files <= target_count { break; }
                        removed[i] = true;
                        total_bytes -= files[i].bytes;
                        total_files -= 1;
                        deleted_bytes += files[i].bytes;
                        deleted_files += 1;
                    }
                    let mut idx = 0;
                    files.retain(|_| { let keep = !removed[idx]; idx += 1; keep });
                    Ok(json!({"@type":"storageStatistics","size":deleted_bytes,"count":deleted_files,"by_chat":[]}))
                }
                method => panic!("unexpected request {method}"),
            }
        })
    }

    fn updates(&self) -> tokio::sync::broadcast::Receiver<Value> { tokio::sync::broadcast::channel(1).1 }
}

#[tokio::test]
async fn clear_all_removes_6000_files_across_multiple_turns_with_per_turn_cap() {
    let files = (0..6000).map(|i| FakeFile {
        chat_id: 1, file_type: "fileTypePhoto", bytes: 10, age_s: 100 + i,
    }).collect();
    let td = FakeStore { files: Mutex::new(files), ..Default::default() };

    let mut turn1 = Report::new();
    clear(&td, Clear::All, &mut turn1).await.unwrap();
    let r1 = turn1.json();
    assert_eq!(r1["freed_files"], 5000);
    assert_eq!(r1["freed_bytes"], 50_000);
    assert_eq!(r1["remaining_files"], 1000);
    assert_eq!(r1["limited"], true);
    assert_eq!(td.files.lock().unwrap().len(), 1000);

    let mut turn2 = Report::new();
    clear(&td, Clear::All, &mut turn2).await.unwrap();
    let r2 = turn2.json();
    assert_eq!(r2["freed_files"], 1000);
    assert_eq!(r2["freed_bytes"], 10_000);
    assert_eq!(r2["remaining_files"], 0);
    assert_eq!(r2["limited"], false);
    assert!(td.files.lock().unwrap().is_empty());
}

#[tokio::test]
async fn clear_chat_removes_oversized_bytes_across_multiple_turns() {
    let gib = 1024 * 1024 * 1024_u64;
    let mut files: Vec<FakeFile> = (0..8).map(|i| FakeFile {
        chat_id: 10, file_type: "fileTypeVideo", bytes: gib, age_s: 100 + i,
    }).collect();
    files.extend((0..2).map(|i| FakeFile {
        chat_id: 11, file_type: "fileTypePhoto", bytes: gib, age_s: 200 + i,
    }));
    let td = FakeStore { files: Mutex::new(files), ..Default::default() };

    let mut turn1 = Report::new();
    clear(&td, Clear::Chat(10), &mut turn1).await.unwrap();
    assert_eq!(turn1.json()["freed_bytes"], 5 * gib);
    assert_eq!(turn1.json()["limited"], true);

    let mut turn2 = Report::new();
    clear(&td, Clear::Chat(10), &mut turn2).await.unwrap();
    assert_eq!(turn2.json()["freed_bytes"], 3 * gib);
    assert_eq!(turn2.json()["limited"], false);
    let left = td.files.lock().unwrap();
    assert_eq!(left.len(), 2);
    assert!(left.iter().all(|f| f.chat_id == 11));
}

#[tokio::test]
async fn max_cache_bytes_trims_oversized_cache_and_preserves_forever_exceptions() {
    let gib = 1024 * 1024 * 1024_u64;
    let mut files: Vec<FakeFile> = (0..8).map(|i| FakeFile {
        chat_id: 10, file_type: "fileTypeVideo", bytes: gib, age_s: 10 + i,
    }).collect();
    // Ngoại lệ mãi mãi có tệp cũ nhất: nếu bỏ exclude_chat_ids sẽ bị xoá trước.
    files.push(FakeFile { chat_id: 43, file_type: "fileTypeDocument", bytes: gib, age_s: 999_999 });
    let td = FakeStore { files: Mutex::new(files), ..Default::default() };
    let mut policy = storage_settings::safe_default();
    policy["max_cache_bytes"] = json!(2 * gib);
    policy["exceptions"] = json!([{"chat_id": 43, "keep": null}]);

    let mut turn1 = Report::new();
    run(&td, &policy, &mut turn1).await.unwrap();
    assert_eq!(turn1.json()["freed_bytes"], 5 * gib);
    assert_eq!(turn1.json()["limited"], true);
    assert_eq!(td.sent.lock().unwrap().last().unwrap()["exclude_chat_ids"], json!([43]));

    let mut turn2 = Report::new();
    run(&td, &policy, &mut turn2).await.unwrap();
    assert_eq!(turn2.json()["freed_bytes"], 2 * gib);
    assert_eq!(turn2.json()["limited"], false);
    let left = td.files.lock().unwrap();
    assert_eq!(left.iter().map(|f| f.bytes).sum::<u64>(), 2 * gib);
    assert_eq!(left.iter().filter(|f| f.chat_id == 43).count(), 1);
}

#[tokio::test]
async fn stream_mode_preserves_forever_exception_and_passes_exclude_chat_ids() {
    let td = FakeStore {
        files: Mutex::new(vec![
            FakeFile { chat_id: 7, file_type: "fileTypePhoto", bytes: 100, age_s: 700 },
            FakeFile { chat_id: 43, file_type: "fileTypePhoto", bytes: 200, age_s: 700 },
        ]),
        ..Default::default()
    };
    let mut policy = storage_settings::safe_default();
    policy["mode"] = json!("stream");
    policy["stream_ttl_s"] = json!(600);
    policy["exceptions"] = json!([{"chat_id": 43, "keep": null}]);

    let mut report = Report::new();
    run(&td, &policy, &mut report).await.unwrap();
    let sent = td.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["chat_ids"], json!([7]));
    assert_eq!(sent[0]["exclude_chat_ids"], json!([43]));
    assert_eq!(sent[0]["ttl"], 600);
    let left = td.files.lock().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].chat_id, 43);
}

#[tokio::test]
async fn auto_download_in_local_mode_respects_rules_limits_and_corrupted_settings() {
    let session = std::env::temp_dir().join(format!("tggw-storage-autodl-{}", std::process::id()));
    fs::create_dir_all(&session).unwrap();
    let mut policy = storage_settings::safe_default();
    policy["mode"] = json!("local");
    policy["auto_download"]["private"]["photo"] = json!({"on": true, "max_bytes": 1024});
    storage_settings::save(&session, &policy).unwrap();

    let td = FakeStore::default();
    td.chats.lock().unwrap().insert(7, json!({"@type":"chatTypePrivate"}));
    td.chats.lock().unwrap().insert(8, json!({"@type":"chatTypeBasicGroup"}));

    let photo = |id: i64, size: u64, outgoing: bool, done: bool| json!({
        "chat_id": 7, "is_outgoing": outgoing,
        "content": {"@type":"messagePhoto","photo":{"sizes":[{"photo":{"id":id,"size":size,"local":{"is_downloading_completed":done}}}]}}
    });
    auto_download(&td, &session, &photo(10, 512, false, false)).await;
    auto_download(&td, &session, &photo(11, 2048, false, false)).await;
    auto_download(&td, &session, &photo(12, 512, true, false)).await;
    auto_download(&td, &session, &photo(13, 512, false, true)).await;
    auto_download(&td, &session, &json!({
        "chat_id": 7, "self_destruct_type": {"@type":"messageSelfDestructTypeTimer"},
        "content": {"@type":"messagePhoto","photo":{"sizes":[{"photo":{"id":14,"size":512}}]}}
    })).await;
    auto_download(&td, &session, &json!({
        "chat_id": 8, "content": {"@type":"messageVideo","video":{"video":{"id":15,"size":512}}}
    })).await;
    assert_eq!(*td.downloads.lock().unwrap(), vec![10]);

    fs::write(session.join(storage_settings::FILE), b"{corrupt").unwrap();
    auto_download(&td, &session, &photo(16, 512, false, false)).await;
    assert_eq!(*td.downloads.lock().unwrap(), vec![10]);
    fs::remove_dir_all(session).unwrap();
}
