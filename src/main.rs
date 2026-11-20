//! Gateway Telegram bản Rust — cột mốc 0: mở libtdjson và đọc phiên bản TDLib.
//!
//! Chạy: `TDJSON_LIB=/đường/tới/libtdjson.so tggw version`
mod td;

fn main() {
    let path = std::env::var("TDJSON_LIB").unwrap_or_else(|_| "libtdjson.so".into());
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "version".into());
    let td = match td::TdJson::open(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("LỖI: {e}");
            std::process::exit(2);
        }
    };
    match cmd.as_str() {
        "version" => {
            // Tắt nhật ký TDLib ra stderr (mặc định khá ồn) trước khi hỏi.
            td.execute(&serde_json::json!({"@type": "setLogVerbosityLevel", "new_verbosity_level": 1}));
            match td.version() {
                Some(v) => println!("TDLib {v}"),
                None => {
                    eprintln!("LỖI: TDLib không trả phiên bản");
                    std::process::exit(3);
                }
            }
        }
        other => {
            eprintln!("lệnh lạ: {other} (có: version)");
            std::process::exit(64);
        }
    }
}
