//! Cảm xúc ĐƯỢC PHÉP của một tin và HÌNH NỀN hội thoại — đồng bộ với Telegram qua TDLib (điện thoại và web thấy giống nhau).
//!
//! * `getMessageAvailableReactions` ⇒ danh sách cảm xúc mỗi cuộc trò chuyện cho phép (hàng nhanh + «gần đây» + «phổ biến»), không gắn cứng 👍.
//! * `getInstalledBackgrounds` / `setChatBackground` / `deleteChatBackground`: bộ sưu tập hình nền của Telegram, ảnh tự gán, màu/chuyển màu,
//!   đổi «cho cả hai bên» (`only_for_self=false`) và độ tối ban đêm (`dark_theme_dimming`).
use serde_json::{json, Value};

use super::{array, int, truth, ChatService, GwError, Reply};

pub(crate) const MAX_WALLPAPER_BYTES: usize = 3 * 1024 * 1024;

/// Tên tệp trong gói ảnh ngoài: chữ/số/_/-, đuôi png/jpg/jpeg/webp (không đường dẫn). Khớp `pack_name_ok` của zalo-rs.
pub(crate) fn pack_name_ok(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    !stem.is_empty() && stem.len() <= 40 && stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') && matches!(ext, "png" | "jpg" | "jpeg" | "webp")
}

fn emoji_of(kind: &Value) -> Option<String> {
    (kind["@type"] == "reactionTypeEmoji").then(|| kind["emoji"].as_str().unwrap_or_default().to_owned()).filter(|e| !e.is_empty())
}

/// Chỉ lấy emoji thường (bỏ emoji tuỳ biến của Premium và cảm xúc cần Premium: tài khoản không Premium sẽ bị Telegram từ chối).
fn emoji_list(list: &Value) -> Vec<Value> {
    array(list).iter().filter(|r| !truth(&r["needs_premium"])).filter_map(|r| emoji_of(&r["type"])).map(|e| json!(e)).collect()
}

fn rgb(color: &Value) -> String { format!("#{:06x}", color.as_i64().unwrap_or(0) & 0xFF_FFFF) }

fn fill_shape(fill: &Value) -> Value {
    match fill["@type"].as_str().unwrap_or("") {
        "backgroundFillSolid" => json!({"kind":"solid","colors":[rgb(&fill["color"])]}),
        "backgroundFillGradient" => json!({"kind":"gradient","colors":[rgb(&fill["top_color"]), rgb(&fill["bottom_color"])],"angle":fill["rotation_angle"].as_i64().unwrap_or(0)}),
        "backgroundFillFreeformGradient" => json!({"kind":"freeform","colors":array(&fill["colors"]).iter().map(rgb).collect::<Vec<_>>()}),
        _ => json!({"kind":"solid","colors":[]}),
    }
}

/// Chuẩn hoá một hình nền TDLib: mã (chuỗi — int64 vượt số an toàn của JS), loại, màu, tệp ảnh/hình thu nhỏ (qua /api/file/<id>).
pub(super) fn background_shape(b: &Value) -> Value {
    let kind = &b["type"];
    let mut out = json!({"id": b["id"].as_str().map(str::to_owned).unwrap_or_else(|| b["id"].to_string()),
        "name": b["name"].as_str().unwrap_or_default(), "dark": truth(&b["is_dark"]), "default": truth(&b["is_default"])});
    match kind["@type"].as_str().unwrap_or("") {
        "backgroundTypeWallpaper" => { out["kind"] = json!("wallpaper"); out["blurred"] = json!(truth(&kind["is_blurred"])); out["moving"] = json!(truth(&kind["is_moving"])); }
        "backgroundTypePattern" => {
            out["kind"] = json!("pattern"); out["intensity"] = json!(kind["intensity"].as_i64().unwrap_or(0)); out["inverted"] = json!(truth(&kind["is_inverted"]));
            out["fill"] = fill_shape(&kind["fill"]);
        }
        "backgroundTypeFill" => { out["kind"] = json!("fill"); out["fill"] = fill_shape(&kind["fill"]); }
        "backgroundTypeChatTheme" => { out["kind"] = json!("theme"); out["theme"] = json!(kind["theme_name"].as_str().unwrap_or_default()); }
        _ => out["kind"] = json!("unknown"),
    }
    let doc = &b["document"];
    if doc["@type"] == "document" {
        if let Some(id) = doc["thumbnail"]["file"]["id"].as_i64().filter(|n| *n > 0) { out["thumb_file"] = json!(id); }
        if let Some(id) = doc["document"]["id"].as_i64().filter(|n| *n > 0) { out["file"] = json!(id); }
    }
    out
}

impl ChatService {
    pub(super) async fn available_reactions(&self, chat: i64, message: i64) -> Reply {
        let id = self.final_id(message).await;
        let r = self.request(json!({"@type":"getMessageAvailableReactions","chat_id":chat,"message_id":id,"row_size":8})).await?;
        Ok(json!({"top": emoji_list(&r["top_reactions"]), "recent": emoji_list(&r["recent_reactions"]), "popular": emoji_list(&r["popular_reactions"]),
            "allow_custom_emoji": truth(&r["allow_custom_emoji"])}))
    }

    /// Bộ sưu tập hình nền đã cài của tài khoản (ban ngày hoặc ban đêm).
    /// GÓI ẢNH NGOÀI dùng chung với Zalo (ví dụ 20 ảnh lát `assets/background/bg_*.png` của APK Zalo, nhập bằng scripts/nhap_hinh_nen_apk.py vào `<data>/wallpaper-pack/`).
    /// Ảnh là tài sản của VNG nên KHÔNG nằm trong kho mã. Không có `name` ⇒ danh sách; có `name` ⇒ ảnh dạng base64 (trình duyệt vẽ lát rồi tải lên như ảnh tự chọn).
    pub(super) async fn wallpaper_pack(&self, name: Option<String>) -> Reply {
        use base64::Engine as _;
        let dir = self.data_dir.join("wallpaper-pack");
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            let mut files: Vec<String> = std::fs::read_dir(&dir).into_iter().flatten().flatten()
                .filter_map(|e| { let n = e.file_name().to_string_lossy().into_owned(); (pack_name_ok(&n) && e.metadata().ok().is_some_and(|m| m.is_file() && m.len() <= 1_048_576)).then_some(n) }).collect();
            files.sort(); files.truncate(200);
            return Ok(json!({"files": files}));
        };
        if !pack_name_ok(&name) { return Err(GwError::bad_request("Tên ảnh trong gói không hợp lệ.")); }
        let path = dir.join(&name);
        let bytes = tokio::task::spawn_blocking(move || std::fs::read(path)).await.ok().and_then(Result::ok)
            .ok_or_else(|| GwError::new(404, "PACK_NOT_FOUND", "Không có ảnh này trong gói."))?;
        if bytes.len() > 1_048_576 { return Err(GwError::bad_request("Ảnh trong gói quá lớn.")); }
        let mime = match name.rsplit('.').next() { Some("png") => "image/png", Some("webp") => "image/webp", _ => "image/jpeg" };
        Ok(json!({"name": name, "mime": mime, "data_b64": base64::engine::general_purpose::STANDARD.encode(bytes)}))
    }

    pub(super) async fn installed_backgrounds(&self, dark: bool) -> Reply {
        let r = self.request(json!({"@type":"getInstalledBackgrounds","for_dark_theme":dark})).await?;
        Ok(json!({"backgrounds": array(&r["backgrounds"]).iter().map(background_shape).collect::<Vec<_>>()}))
    }

    /// Hình nền hiện tại của hội thoại: `null` nếu không có.
    pub(super) async fn chat_background(&self, chat: i64) -> Reply {
        let ch = self.request(json!({"@type":"getChat","chat_id":chat})).await?;
        self.state.remember_chat(&ch);
        let bg = &ch["background"];
        if !bg.is_object() { return Ok(json!({"background": null})); }
        Ok(json!({"background": {"dimming": bg["dark_theme_dimming"].as_i64().unwrap_or(0), "background": background_shape(&bg["background"])}}))
    }

    /// Đặt hình nền hội thoại. Thân: `{background_id}` (từ bộ sưu tập) | `{color}` / `{gradient:{from,to,angle}}` | `{file_path}` (ảnh đã tải lên gateway);
    /// kèm `dimming` 0–100 (độ tối ban đêm), `for_both` (đổi luôn phía người kia — chỉ hội thoại 1-1), `blurred`, `moving`.
    pub(super) async fn set_chat_background(&self, chat: i64, body: &Value) -> Reply {
        let dimming = if body["dimming"].is_null() { 0 } else { int(&body["dimming"], "dimming")? };
        if !(0..=100).contains(&dimming) { return Err(GwError::bad_request("dimming phải từ 0 đến 100.")); }
        let hex = |v: &Value, name: &str| -> Result<i64, GwError> {
            let s = v.as_str().unwrap_or_default().trim_start_matches('#');
            if s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()) { Ok(i64::from_str_radix(s, 16).unwrap()) } else { Err(GwError::bad_request(format!("{name} phải là màu #rrggbb."))) }
        };
        let (background, kind) = if let Some(id) = body["background_id"].as_str().filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'-')) {
            (json!({"@type":"inputBackgroundRemote","background_id":id}), Value::Null)       // loại mặc định của hình nền từ xa
        } else if !body["color"].is_null() {
            (Value::Null, json!({"@type":"backgroundTypeFill","fill":{"@type":"backgroundFillSolid","color":hex(&body["color"], "color")?}}))
        } else if body["gradient"].is_object() {
            let g = &body["gradient"];
            (Value::Null, json!({"@type":"backgroundTypeFill","fill":{"@type":"backgroundFillGradient","top_color":hex(&g["from"], "from")?,
                "bottom_color":hex(&g["to"], "to")?,"rotation_angle":g["angle"].as_i64().unwrap_or(0).rem_euclid(360)}}))
        } else if let Some(path) = body["file_path"].as_str().filter(|p| !p.is_empty()) {
            let path = self.wallpaper_path(path)?;
            (json!({"@type":"inputBackgroundLocal","background":{"@type":"inputFileLocal","path":path}}),
                json!({"@type":"backgroundTypeWallpaper","is_blurred":truth(&body["blurred"]),"is_moving":truth(&body["moving"])}))
        } else {
            return Err(GwError::bad_request("Cần background_id, color, gradient hoặc file_path."));
        };
        self.request(json!({"@type":"setChatBackground","chat_id":chat,"background":background,"type":kind,"dark_theme_dimming":dimming,
            "only_for_self":!truth(&body["for_both"])})).await?;
        Ok(json!({"ok":true}))
    }

    pub(super) async fn delete_chat_background(&self, chat: i64, body: &Value) -> Reply {
        self.request(json!({"@type":"deleteChatBackground","chat_id":chat,"restore_previous":truth(&body["restore_previous"])})).await?;
        Ok(json!({"ok":true}))
    }

    /// Lưu ảnh nền tải lên (JPEG/PNG thật, ≤ 3 MB) vào `wallpapers/<băm sha256 32 hex>.<đuôi>` — cùng ảnh ⇒ cùng tên, không trùng. Trả tên tệp.
    pub(super) async fn save_wallpaper(&self, body: &Value) -> Reply {
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.decode(body["data_b64"].as_str().unwrap_or_default())
            .map_err(|_| GwError::bad_request("Dữ liệu ảnh không hợp lệ."))?;
        if data.is_empty() { return Err(GwError::bad_request("Ảnh rỗng.")); }
        if data.len() > MAX_WALLPAPER_BYTES { return Err(GwError::bad_request("Ảnh nền tối đa 3 MB.")); }
        let ext = if data.starts_with(&[0xFF, 0xD8, 0xFF]) { "jpg" } else if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) { "png" }
            else { return Err(GwError::bad_request("Chỉ nhận ảnh JPEG hoặc PNG.")); };
        let hash: String = ring::digest::digest(&ring::digest::SHA256, &data).as_ref().iter().take(16).map(|b| format!("{b:02x}")).collect();
        let name = format!("{hash}.{ext}");
        let dir = self.data_dir.join("wallpapers");
        let target = dir.join(&name);
        let io = |_| GwError::new(500, "WALLPAPER_STORAGE", "Không lưu được ảnh nền.");
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
            if !target.exists() {
                use std::io::Write;
                let temp = dir.join(format!(".tmp-{}", target.file_name().unwrap().to_string_lossy()));
                let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
                f.write_all(&data)?; f.sync_all()?; std::fs::rename(temp, target)?;
            }
            Ok(())
        }).await.map_err(|_| GwError::new(500, "WALLPAPER_STORAGE", "Không lưu được ảnh nền."))?.map_err(io)?;
        Ok(json!({"ok":true,"file":name}))
    }

    /// Ảnh nền tự gán chỉ được lấy từ thư mục `wallpapers/` của gateway (không cho đường dẫn tuỳ ý).
    fn wallpaper_path(&self, name: &str) -> Result<String, GwError> {
        let ok = name.len() == 36 && name[..32].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) && matches!(&name[32..], ".jpg" | ".png");
        if !ok { return Err(GwError::bad_request("Tên ảnh nền không hợp lệ.")); }
        let path = self.data_dir.join("wallpapers").join(name);
        if !path.is_file() { return Err(GwError::bad_request("Ảnh nền chưa được tải lên.")); }
        path.to_str().map(str::to_owned).ok_or_else(|| GwError::bad_request("Đường dẫn ảnh nền không hợp lệ."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_cover_every_background_kind_and_keep_ids_as_strings() {
        let wall = background_shape(&json!({"id":"5678901234567890123","name":"Núi","is_dark":true,"type":{"@type":"backgroundTypeWallpaper","is_blurred":true,"is_moving":false},
            "document":{"@type":"document","thumbnail":{"file":{"id":41}},"document":{"id":42}}}));
        assert_eq!((wall["id"].as_str(), wall["kind"].as_str(), wall["blurred"].as_bool(), wall["thumb_file"].as_i64(), wall["file"].as_i64()),
            (Some("5678901234567890123"), Some("wallpaper"), Some(true), Some(41), Some(42)));
        let grad = background_shape(&json!({"id":"1","type":{"@type":"backgroundTypeFill","fill":{"@type":"backgroundFillGradient","top_color":16711680,"bottom_color":255,"rotation_angle":45}}}));
        assert_eq!(grad["fill"], json!({"kind":"gradient","colors":["#ff0000","#0000ff"],"angle":45}));
        let solid = background_shape(&json!({"id":"2","type":{"@type":"backgroundTypeFill","fill":{"@type":"backgroundFillSolid","color":2105376}}}));
        assert_eq!(solid["fill"]["colors"], json!(["#202020"]));
        let free = background_shape(&json!({"id":"3","type":{"@type":"backgroundTypeFill","fill":{"@type":"backgroundFillFreeformGradient","colors":[1,2,3,4]}}}));
        assert_eq!(free["fill"]["colors"].as_array().unwrap().len(), 4);
        let pat = background_shape(&json!({"id":"4","type":{"@type":"backgroundTypePattern","intensity":60,"is_inverted":true,"fill":{"@type":"backgroundFillSolid","color":0}}}));
        assert_eq!((pat["kind"].as_str(), pat["intensity"].as_i64(), pat["inverted"].as_bool()), (Some("pattern"), Some(60), Some(true)));
        assert_eq!(background_shape(&json!({"id":"5","type":{"@type":"backgroundTypeChatTheme","theme_name":"🐥"}}))["theme"], "🐥");
        assert_eq!(background_shape(&json!({"id":9,"type":{"@type":"lạ"}}))["id"], "9", "mã số nguyên cũng ra chuỗi");
        assert_eq!(background_shape(&json!({"id":"6","type":{"@type":"lạ"}}))["kind"], "unknown");
    }

    #[test]
    fn emoji_list_skips_premium_and_custom_emoji() {
        let list = json!([{"type":{"@type":"reactionTypeEmoji","emoji":"❤️"},"needs_premium":false},{"type":{"@type":"reactionTypeEmoji","emoji":"🔥"}},
            {"type":{"@type":"reactionTypeEmoji","emoji":"🦄"},"needs_premium":true},{"type":{"@type":"reactionTypeCustomEmoji","custom_emoji_id":"1"}},{"type":{"@type":"reactionTypeEmoji","emoji":""}}]);
        assert_eq!(emoji_list(&list), vec![json!("❤️"), json!("🔥")]);
        assert!(emoji_list(&Value::Null).is_empty());
    }
}
