//! Danh mục HA chỉ là dữ liệu để chọn thiết bị; không tự mở URL hoặc chạy tiến trình.
use std::{collections::HashSet, fs, path::Path};
use serde_json::{json, Value};
use crate::contract::GwError;

const MAX_DEVICES: usize = 64;

pub(super) fn empty() -> Value { json!({"mic":[],"speaker":[],"camera":[]}) }

fn valid_id(id: &str, kind: &str) -> bool {
    if id.is_empty() || id.len() > 2048 || id.chars().any(|c| c.is_control() || c.is_whitespace()) { return false; }
    if id.contains("://") {
        if kind == "speaker" { return super::valid_speaker_url(id); }
        let Ok(url) = reqwest::Url::parse(id) else { return false; };
        return matches!(url.scheme(), "http" | "https" | "rtsp" | "rtsps" | "rtmp") && url.host_str().is_some()
            && !id.contains(['\'', '"', '`', '$', '<', '>', '\\']);
    }
    // ID HA/opaque: không cho đường dẫn, dấu lệnh hoặc ký tự điều khiển.
    id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        && id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
}

pub(super) fn filter(body: &Value) -> Result<Value, GwError> {
    if !body.is_object() { return Err(GwError::bad_request("Cần danh mục {mic:[],speaker:[],camera:[]}.")); }
    let mut result = empty();
    for kind in ["mic", "speaker", "camera"] {
        let Some(items) = body.get(kind) else { continue; };
        let items = items.as_array().ok_or_else(|| GwError::bad_request("Mỗi nhóm thiết bị phải là mảng."))?;
        let mut seen = HashSet::new();
        let out = result[kind].as_array_mut().unwrap();
        for item in items {
            let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str()) else { continue; };
            let (id, name) = (id.trim(), name.trim());
            if !valid_id(id, kind) || name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) || !seen.insert(id.to_owned()) { continue; }
            out.push(json!({"id":id,"name":name}));
            if out.len() == MAX_DEVICES { break; }
        }
    }
    Ok(result)
}

pub(super) fn load(dir: &Path) -> Result<Value, GwError> {
    match fs::read(dir.join("ha_devices.json")) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|_| super::io_error()).and_then(|v| filter(&v)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(empty()),
        Err(_) => Err(super::io_error()),
    }
}

pub(super) fn save(dir: &Path, body: &Value) -> Result<Value, GwError> {
    let catalog = filter(body)?;
    fs::create_dir_all(dir).map_err(|_| super::io_error())?;
    crate::accounts::atomic_json(&dir.join("ha_devices.json"), &catalog).map_err(|_| super::io_error())?;
    Ok(catalog)
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
        assert_eq!(result["mic"][1]["id"],"http://camera.example/audio");
        assert_eq!(result["speaker"].as_array().unwrap().len(),1);
        assert_eq!(filter(&json!({})).unwrap(),empty());
        assert!(filter(&json!({"mic":"bad"})).is_err());
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
}
