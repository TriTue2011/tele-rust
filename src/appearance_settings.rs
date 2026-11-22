//! Cấu hình giao diện trò chuyện theo thư mục phiên TDLib.
use std::{fs, io, path::Path};

use serde_json::{json, Value};

use crate::{accounts::atomic_json, contract::GwError};

pub(crate) const FILE: &str = "appearance_settings.json";
pub(crate) const FONTS: [&str; 4] = ["system", "sans", "serif", "mono"];
pub(crate) const THEMES: [&str; 10] = [
    "system", "light", "dark", "midnight", "telegram",
    "green", "purple", "orange", "pink", "high-contrast",
];
pub(crate) const BUBBLES: [&str; 3] = ["classic", "rounded", "compact"];
pub(crate) const ACCENTS: [&str; 6] = ["blue", "green", "purple", "orange", "pink", "teal"];
pub(crate) const LINE_HEIGHTS: [&str; 3] = ["compact", "normal", "relaxed"];
pub(crate) const CHAT_BGS: [&str; 3] = ["plain", "pattern", "gradient"];
pub(crate) const CODE_THEMES: [&str; 3] = ["auto", "light", "dark"];
pub(crate) const MIN_FONT_SIZE: u64 = 12;
pub(crate) const MAX_FONT_SIZE: u64 = 20;

pub(crate) fn defaults() -> Value {
    json!({
        "font_family": "system",
        "font_size": 15,
        "theme": "system",
        "bubble_style": "classic",
        "accent": "blue",
    })
}

pub(crate) fn validate(settings: &Value) -> Result<(), GwError> {
    let required = ["font_family", "font_size", "theme", "bubble_style", "accent"];
    let optional = ["line_height", "chat_bg", "code_theme"];
    let valid_fields = settings.as_object().is_some_and(|m| {
        required.iter().all(|k| m.contains_key(*k))
            && m.keys().all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
    });
    let valid_line_height = settings.get("line_height").map_or(true, |v| {
        v.as_str().is_some_and(|s| LINE_HEIGHTS.contains(&s))
    });
    let valid_chat_bg = settings.get("chat_bg").map_or(true, |v| {
        v.as_str().is_some_and(|s| CHAT_BGS.contains(&s))
    });
    let valid_code_theme = settings.get("code_theme").map_or(true, |v| {
        v.as_str().is_some_and(|s| CODE_THEMES.contains(&s))
    });
    let valid = valid_fields
        && valid_line_height
        && valid_chat_bg
        && valid_code_theme
        && settings["font_family"].as_str().is_some_and(|v| FONTS.contains(&v))
        && settings["font_size"].as_u64().is_some_and(|n| (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&n))
        && settings["theme"].as_str().is_some_and(|v| THEMES.contains(&v))
        && settings["bubble_style"].as_str().is_some_and(|v| BUBBLES.contains(&v))
        && settings["accent"].as_str().is_some_and(|v| ACCENTS.contains(&v));
    if valid {
        Ok(())
    } else {
        Err(GwError::new(422, "INVALID_APPEARANCE_SETTINGS", "Cấu hình giao diện không hợp lệ."))
    }
}

pub(crate) fn load(session: &Path) -> io::Result<Value> {
    match fs::read(session.join(FILE)) {
        Ok(bytes) => {
            let value = serde_json::from_slice(&bytes)?;
            validate(&value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.message))?;
            Ok(value)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(defaults()),
        Err(error) => Err(error),
    }
}

pub(crate) fn load_or_default(session: &Path) -> Value {
    match load(session) {
        Ok(value) => value,
        Err(err) => {
            log::warn!("Cấu hình giao diện không đọc được ({:?}); dùng mặc định.", err.kind());
            defaults()
        }
    }
}

pub(crate) fn save(session: &Path, settings: &Value) -> io::Result<()> {
    atomic_json(&session.join(FILE), settings)?;
    fs::File::open(session)?.sync_all()
}

pub(crate) fn io_error() -> GwError {
    GwError::new(500, "APPEARANCE_SETTINGS_IO", "Không đọc hoặc ghi được cấu hình giao diện.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn defaults_and_validation_enforce_allowed_values() {
        let def = defaults();
        assert!(validate(&def).is_ok());

        let valid = json!({
            "font_family": "mono",
            "font_size": 18,
            "theme": "dark",
            "bubble_style": "rounded",
            "accent": "teal",
        });
        assert!(validate(&valid).is_ok());

        for bad in [
            json!({}),
            json!({"font_family":"custom","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue"}),
            json!({"font_family":"system","font_size":11,"theme":"system","bubble_style":"classic","accent":"blue"}),
            json!({"font_family":"system","font_size":21,"theme":"system","bubble_style":"classic","accent":"blue"}),
            json!({"font_family":"system","font_size":15.5,"theme":"system","bubble_style":"classic","accent":"blue"}),
            json!({"font_family":"system","font_size":"15","theme":"system","bubble_style":"classic","accent":"blue"}),
            json!({"font_family":"system","font_size":-15,"theme":"system","bubble_style":"classic","accent":"blue"}),
            json!({"font_family":"system","font_size":15,"theme":"neon","bubble_style":"classic","accent":"blue"}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"pill","accent":"blue"}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"red"}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","extra":1}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","line_height":"huge"}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","line_height":1.5}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","line_height":true}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","chat_bg":"photo"}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","chat_bg":123}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","code_theme":"solarized"}),
            json!({"font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","code_theme":false}),
        ] {
            let err = validate(&bad).unwrap_err();
            assert_eq!((err.status, err.code.as_str()), (422, "INVALID_APPEARANCE_SETTINGS"));
        }

        for lh in LINE_HEIGHTS {
            assert!(validate(&json!({
                "font_family":"system","font_size":15,"theme":"system","bubble_style":"classic","accent":"blue","line_height":lh
            })).is_ok());
        }
        for theme in THEMES {
            for chat_bg in CHAT_BGS {
                for code_theme in CODE_THEMES {
                    assert!(validate(&json!({
                        "font_family":"system","font_size":15,"theme":theme,"bubble_style":"classic","accent":"blue",
                        "chat_bg":chat_bg,"code_theme":code_theme
                    })).is_ok());
                }
            }
        }
    }

    #[test]
    fn save_writes_0600_and_corrupt_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("tggw-appearance-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(load(&dir).unwrap(), defaults());

        let custom = json!({
            "font_family": "serif",
            "font_size": 20,
            "theme": "light",
            "bubble_style": "compact",
            "accent": "green",
        });
        save(&dir, &custom).unwrap();
        let file = dir.join(FILE);
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(load(&dir).unwrap(), custom);

        fs::write(&file, b"{broken").unwrap();
        assert!(load(&dir).is_err());
        assert_eq!(load_or_default(&dir), defaults());

        fs::write(&file, b"{\"font_family\":\"system\",\"font_size\":99,\"theme\":\"system\",\"bubble_style\":\"classic\",\"accent\":\"blue\"}").unwrap();
        assert!(load(&dir).is_err());
        assert_eq!(load_or_default(&dir), defaults());

        fs::remove_dir_all(&dir).unwrap();
    }
}
