//! Chính sách lưu trữ theo thư mục phiên TDLib.
use std::{fs, io, path::Path};

use serde_json::{json, Value};

use crate::{accounts::atomic_json, contract::GwError};

pub(crate) const FILE: &str = "storage_settings.json";
pub(crate) const CHAT_TYPES: [&str; 3] = ["private", "groups", "channels"];
pub(crate) const MEDIA_TYPES: [&str; 6] = ["photo", "video", "file", "voice", "video_note", "gif"];
const MAX_INT53: u64 = (1 << 53) - 1;

fn defaults(upgrade: bool) -> Value {
    let mut keep = json!({});
    let mut auto_download = json!({});
    for chat in CHAT_TYPES {
        keep[chat] = if upgrade { Value::Null } else { json!(604800) };
        for media in MEDIA_TYPES {
            auto_download[chat][media] = json!({"on":false,"max_bytes":10 * 1024 * 1024});
        }
    }
    json!({
        "mode":"local", "keep":keep, "exceptions":[], "auto_download":auto_download,
        "max_cache_bytes":if upgrade { Value::Null } else { json!(2_u64 * 1024 * 1024 * 1024) },
        "stream_ttl_s":600
    })
}

pub(crate) fn load(session: &Path) -> io::Result<Value> {
    match fs::read(session.join(FILE)) {
        Ok(bytes) => {
            let value = serde_json::from_slice(&bytes)?;
            validate(&value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.message))?;
            Ok(value)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let upgrade = match fs::read_dir(session.join("files")) {
                Ok(mut entries) => entries.next().transpose()?.is_some(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            };
            Ok(defaults(upgrade))
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn safe_default() -> Value {
    defaults(true)
}

pub(crate) fn parse_update(mut body: Value) -> Result<(Value, bool), GwError> {
    let invalid = || GwError::new(422, "INVALID_STORAGE_SETTINGS", "Cấu hình lưu trữ không hợp lệ.");
    let Some(map) = body.as_object_mut() else {
        return Err(invalid());
    };
    let confirmed = match map.remove("confirm") {
        None => false,
        Some(Value::Bool(v)) => v,
        Some(_) => return Err(invalid()),
    };
    validate(&body)?;
    Ok((body, confirmed))
}

fn tighter(old: Option<u64>, new: Option<u64>) -> bool {
    match (old, new) {
        (None, Some(_)) => true,
        (Some(a), Some(b)) => b < a,
        _ => false,
    }
}

fn exception_map(settings: &Value) -> std::collections::BTreeMap<i64, Option<u64>> {
    settings["exceptions"].as_array().into_iter().flatten()
        .filter_map(|row| row["chat_id"].as_i64().map(|id| (id, row["keep"].as_u64())))
        .collect()
}

pub(crate) fn is_tightening(current: &Value, next: &Value) -> bool {
    if current["mode"] == "local" && next["mode"] == "stream" {
        return true;
    }
    let cur_exc = exception_map(current);
    let next_exc = exception_map(next);
    let cur_stream_ttl = current["stream_ttl_s"].as_u64();
    let next_stream_ttl = next["stream_ttl_s"].as_u64();
    if next["mode"] == "stream" {
        if tighter(cur_stream_ttl, next_stream_ttl) {
            return true;
        }
        return cur_exc.keys().chain(next_exc.keys()).any(|id| {
            let old = cur_exc.get(id).copied().unwrap_or(cur_stream_ttl);
            let new = next_exc.get(id).copied().unwrap_or(next_stream_ttl);
            tighter(old, new)
        });
    }
    if tighter(current["max_cache_bytes"].as_u64(), next["max_cache_bytes"].as_u64()) {
        return true;
    }
    let next_has_limit = next["max_cache_bytes"].as_u64().is_some()
        || CHAT_TYPES.iter().any(|kind| next["keep"][kind].as_u64().is_some());
    if current["mode"] == "stream" {
        if CHAT_TYPES.iter().any(|kind| tighter(cur_stream_ttl, next["keep"][kind].as_u64())) {
            return true;
        }
        return cur_exc.keys().chain(next_exc.keys()).any(|id| {
            let old = cur_exc.get(id).copied().unwrap_or(cur_stream_ttl);
            match next_exc.get(id).copied() {
                Some(new) => tighter(old, new),
                None => match old {
                    None => next_has_limit,
                    Some(_) => CHAT_TYPES.iter().any(|kind| tighter(old, next["keep"][kind].as_u64())),
                },
            }
        });
    }
    if CHAT_TYPES.iter().any(|kind| tighter(current["keep"][kind].as_u64(), next["keep"][kind].as_u64())) {
        return true;
    }
    cur_exc.keys().chain(next_exc.keys()).any(|id| {
        match (cur_exc.get(id).copied(), next_exc.get(id).copied()) {
            (Some(old), Some(new)) => tighter(old, new),
            (Some(old), None) => match old {
                None => next_has_limit,
                Some(_) => CHAT_TYPES.iter().any(|kind| tighter(old, next["keep"][kind].as_u64())),
            },
            (None, Some(new)) => {
                new.is_some() && CHAT_TYPES.iter().any(|kind| tighter(current["keep"][kind].as_u64(), new))
            }
            (None, None) => false,
        }
    })
}

pub(crate) fn validate(settings: &Value) -> Result<(), GwError> {
    let keep = |v: &Value| v.is_null() || matches!(v.as_u64(), Some(86400 | 604800 | 2678400));
    let bytes = |v: &Value| v.as_u64().is_some_and(|n| n <= MAX_INT53);
    let fields = |v: &Value, names: &[&str]| {
        v.as_object().is_some_and(|m| m.len() == names.len() && names.iter().all(|k| m.contains_key(*k)))
    };
    let mut seen = std::collections::HashSet::new();
    let valid = fields(settings, &["mode", "keep", "exceptions", "max_cache_bytes", "auto_download", "stream_ttl_s"])
        && matches!(settings["mode"].as_str(), Some("local" | "stream"))
        && fields(&settings["keep"], &CHAT_TYPES)
        && CHAT_TYPES.iter().all(|chat| keep(&settings["keep"][chat]))
        && settings["exceptions"].as_array().is_some_and(|exceptions| exceptions.iter().all(|row| {
            fields(row, &["chat_id", "keep"]) && keep(&row["keep"])
                && row["chat_id"].as_i64().is_some_and(|n| n != 0 && n.unsigned_abs() <= MAX_INT53 && seen.insert(n))
        }))
        && (settings["max_cache_bytes"].is_null() || bytes(&settings["max_cache_bytes"]))
        && settings["stream_ttl_s"].as_u64().is_some_and(|n| (1..=i32::MAX as u64).contains(&n))
        && fields(&settings["auto_download"], &CHAT_TYPES)
        && CHAT_TYPES.iter().all(|chat| {
            let rules = &settings["auto_download"][chat];
            fields(rules, &MEDIA_TYPES) && MEDIA_TYPES.iter().all(|media| {
                let rule = &rules[media];
                fields(rule, &["on", "max_bytes"]) && rule["on"].is_boolean() && bytes(&rule["max_bytes"])
            })
        });
    if valid { Ok(()) } else { Err(GwError::new(422, "INVALID_STORAGE_SETTINGS", "Cấu hình lưu trữ không hợp lệ.")) }
}

pub(crate) fn save(session: &Path, settings: &Value) -> io::Result<()> {
    atomic_json(&session.join(FILE), settings)?;
    fs::File::open(session)?.sync_all()
}

pub(crate) fn init(session: &Path) -> io::Result<()> {
    if !session.join(FILE).try_exists()? {
        save(session, &load(session)?)?;
    }
    Ok(())
}
