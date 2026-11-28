//! Danh mục HA chỉ là dữ liệu để chọn thiết bị; không tự mở URL hoặc chạy tiến trình.
//! Mục nào HA đẩy lên dạng URL (mic/speaker/camera có mật khẩu RTSP, mic R1 ?k=…) thì KHÔNG trả nguyên URL cho trình
//! duyệt qua GET /api/live/devices — mọi người đăng nhập chung WebUI sẽ đọc được, không riêng ai cấu hình thiết bị đó.
//! `filter_with_urls` thay URL bằng mã ẩn danh (băm SHA-256 của URL), URL thật chỉ lưu trong `ha_devices_urls.json`
//! (0600, cùng cấp `ha_devices.json`) — `resolve()` tra lại URL thật khi THỰC SỰ phát lệnh (sources.rs), không phải
//! lúc liệt kê. Đường "Nhập URL" thủ công (người dùng tự gõ trong trình duyệt) không qua đây, không bị ảnh hưởng.
use std::{collections::{HashMap, HashSet}, fs, path::Path};
use serde_json::{json, Value};
use crate::contract::GwError;

const MAX_DEVICES: usize = 64;
const URLS_FILE: &str = "ha_devices_urls.json";

pub(super) fn empty() -> Value { json!({"mic":[],"speaker":[],"camera":[]}) }

fn valid_id(id: &str, kind: &str) -> bool {
    if id.is_empty() || id.len() > 2048 || id.chars().any(|c| c.is_control() || c.is_whitespace()) { return false; }
    if id.contains("://") {
        if kind == "speaker" { return super::valid_speaker_url(id); }
        return super::sources::valid_media_url(id) && !id.contains(['\'', '"', '`', '$', '<', '>', '\\']);
    }
    // ID HA/opaque: không cho đường dẫn, dấu lệnh hoặc ký tự điều khiển.
    id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        && id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
}

// Băm ổn định: cùng URL ⇒ cùng mã ẩn danh mỗi lần HA đẩy lại danh mục (picker không đổi lựa chọn đã chọn trước đó).
fn anon_id(url: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, url.as_bytes()).as_ref().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn filter_with_urls(body: &Value) -> Result<(Value, HashMap<String, String>), GwError> {
    if !body.is_object() { return Err(GwError::bad_request("Cần danh mục {mic:[],speaker:[],camera:[]}.")); }
    let mut result = empty();
    let mut urls = HashMap::new();
    for kind in ["mic", "speaker", "camera"] {
        let Some(items) = body.get(kind) else { continue; };
        let items = items.as_array().ok_or_else(|| GwError::bad_request("Mỗi nhóm thiết bị phải là mảng."))?;
        let mut seen = HashSet::new();
        let out = result[kind].as_array_mut().unwrap();
        for item in items {
            let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str()) else { continue; };
            let (id, name) = (id.trim(), name.trim());
            if !valid_id(id, kind) || name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) || !seen.insert(id.to_owned()) { continue; }
            let public_id = if id.contains("://") { let a = anon_id(id); urls.insert(a.clone(), id.to_owned()); a } else { id.to_owned() };
            out.push(json!({"id":public_id,"name":name}));
            if out.len() == MAX_DEVICES { break; }
        }
    }
    Ok((result, urls))
}

pub(super) fn filter(body: &Value) -> Result<Value, GwError> { Ok(filter_with_urls(body)?.0) }

pub(super) fn load(dir: &Path) -> Result<Value, GwError> {
    match fs::read(dir.join("ha_devices.json")) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|_| super::io_error()).and_then(|v| filter(&v)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(empty()),
        Err(_) => Err(super::io_error()),
    }
}

pub(super) fn save(dir: &Path, body: &Value) -> Result<Value, GwError> {
    let (catalog, urls) = filter_with_urls(body)?;
    fs::create_dir_all(dir).map_err(|_| super::io_error())?;
    let urls_path = dir.join(URLS_FILE);
    if !urls.is_empty() {
        crate::accounts::atomic_json(&urls_path, &json!(urls)).map_err(|_| super::io_error())?;
    }
    crate::accounts::atomic_json(&dir.join("ha_devices.json"), &catalog).map_err(|_| super::io_error())?;
    if urls.is_empty() { let _ = fs::remove_file(&urls_path); }
    Ok(catalog)
}

/// Tra URL thật từ mã ẩn danh — chỉ gọi lúc THỰC SỰ phát lệnh (sources.rs), không phải lúc liệt kê cho trình duyệt.
/// `id` không khớp mục nào (id HA thường, hoặc giá trị người dùng tự gõ ở "Nhập URL") ⇒ None, gọi nơi dùng tự giữ nguyên giá trị gốc.
pub(super) fn resolve(dir: &Path, id: &str) -> Option<String> {
    let raw = fs::read(dir.join(URLS_FILE)).ok()?;
    let map: HashMap<String, String> = serde_json::from_slice(&raw).ok()?;
    map.get(id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_ids_urls_duplicates_names_and_caps_each_group() {
        let mut mic = vec![json!({"id":" sensor.mic ","name":" Mic "}), json!({"id":"sensor.mic","name":"trùng"}),
            json!({"id":"file:///secret","name":"x"}),json!({"id":"http://","name":"x"}),json!({"id":"http://cam/$(id)","name":"x"}),
            json!({"id":"../file","name":"x"}),json!({"id":"x\ny","name":"x"}),json!({"id":"valid","name":"x\ny"}),
            json!({"id":"http://camera.example/audio","name":"HTTP"})];
        for i in 0..100 { mic.push(json!({"id":format!("mic.{i}"),"name":"Thiết bị"})); }
        let result = filter(&json!({"mic":mic,"speaker":[{"id":"https://evil.example/x","name":"Loa"},{"id":"media_player.room","name":"Phòng"}],"camera":[]})).unwrap();
        assert_eq!(result["mic"].as_array().unwrap().len(),64);
        assert_eq!(result["mic"][0],json!({"id":"sensor.mic","name":"Mic"}));
        // id dạng URL KHÔNG trả nguyên văn ra ngoài (ai đăng nhập chung WebUI cũng đọc được qua GET devices nếu còn lộ) — thay bằng mã ẩn danh.
        let anon = result["mic"][1]["id"].as_str().unwrap();
        assert_ne!(anon,"http://camera.example/audio"); assert!(anon.len()==32 && anon.bytes().all(|b|b.is_ascii_hexdigit()));
        assert_eq!(result["speaker"].as_array().unwrap().len(),1);
        assert_eq!(filter(&json!({})).unwrap(),empty());
        assert!(filter(&json!({"mic":"bad"})).is_err());
    }

    #[test]
    fn urls_anonymized_resolve_back_and_file_only_exists_when_needed() {
        let dir=std::path::PathBuf::from(format!("target/browser-catalog-urls-{}",std::process::id()));
        let with_url=json!({"mic":[{"id":"rtsp://user:pass@cam.local/stream","name":"Mic cổng"}],"speaker":[],"camera":[]});
        let saved=save(&dir,&with_url).unwrap();
        let anon=saved["mic"][0]["id"].as_str().unwrap().to_owned();
        assert_ne!(anon,"rtsp://user:pass@cam.local/stream","URL thật không được lộ ra danh mục công khai");
        assert_eq!(resolve(&dir,&anon).as_deref(),Some("rtsp://user:pass@cam.local/stream"),"tra lại đúng URL thật bằng mã ẩn danh");
        assert_eq!(resolve(&dir,"id-khong-ton-tai"),None,"id lạ (vd. người dùng tự gõ ở Nhập URL) không khớp gì");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(dir.join(URLS_FILE)).unwrap().permissions().mode()&0o777,0o600);
        // Đẩy lại danh mục CÙNG URL ⇒ CÙNG mã ẩn danh (ổn định qua nhiều lần HA đẩy lại, không đổi lựa chọn người dùng đã chọn).
        let saved2=save(&dir,&with_url).unwrap();
        assert_eq!(saved2["mic"][0]["id"],json!(anon));
        // Đẩy lại KHÔNG còn URL nào ⇒ tệp URL thật phải mất, không để sót lại tra được URL cũ.
        save(&dir,&json!({"mic":[{"id":"sensor.mic","name":"Mic"}],"speaker":[],"camera":[]})).unwrap();
        assert!(!dir.join(URLS_FILE).exists());
        assert_eq!(resolve(&dir,&anon),None,"xoá danh mục URL thì không tra lại được URL cũ nữa");
        fs::remove_file(dir.join("ha_devices.json")).unwrap();fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn catalog_is_atomic_private_and_restores_filtered_values() {
        use std::os::unix::fs::PermissionsExt;
        let dir=std::path::PathBuf::from(format!("target/browser-catalog-{}",std::process::id()));
        assert_eq!(load(&dir).unwrap(),empty());
        let body=json!({"mic":[{"id":"sensor.mic","name":"Mic"}],"speaker":[],"camera":[]});
        let saved=save(&dir,&body).unwrap();
        assert_eq!(load(&dir).unwrap(),saved);
        assert_eq!(fs::metadata(dir.join("ha_devices.json")).unwrap().permissions().mode()&0o777,0o600);
        let invalid=json!({"mic":false});
        assert!(save(&dir,&invalid).is_err());assert_eq!(load(&dir).unwrap(),saved);
        assert_eq!(fs::read_dir(&dir).unwrap().count(),1);
        fs::remove_file(dir.join("ha_devices.json")).unwrap();fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn rejects_link_local_metadata_and_unspecified_urls() {
        let body = json!({
            "mic": [
                {"id": "http://169.254.169.254/audio", "name": "Metadata"},
                {"id": "http://[::ffff:169.254.169.254]/audio", "name": "MappedMetadata"},
                {"id": "http://0.0.0.0/audio", "name": "Unspecified"},
                {"id": "http://[::]/audio", "name": "UnspecifiedV6"},
                {"id": "http://[fe80::1]/audio", "name": "LinkLocalV6"},
                {"id": "http://camera.example/audio", "name": "Valid"}
            ],
            "speaker": [],
            "camera": []
        });
        let result = filter(&body).unwrap();
        assert_eq!(result["mic"].as_array().unwrap().len(), 1);
        assert_eq!(result["mic"][0]["name"], "Valid");
    }
}
