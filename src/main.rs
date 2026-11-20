//! Gateway Telegram bản Rust — cột mốc 0: mở libtdjson và đọc phiên bản TDLib.
//!
//! Chạy: `TDJSON_LIB=/đường/tới/libtdjson.so tggw version`
mod contract;
mod rest;
mod td;

fn main() {
    let path = std::env::var("TDJSON_LIB").unwrap_or_else(|_| "libtdjson.so".into());
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "version".into());
    if cmd == "protocol" {
        return run_protocol();
    }
    if cmd == "serve" {
        let stub = std::sync::Arc::new(rest::ChuaCo);
        if let Err(e) = rest::serve_from_env(stub.clone(), stub) {
            eprintln!("LỖI: {e}");
            std::process::exit(2);
        }
        return;
    }
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
            eprintln!("lệnh lạ: {other} (có: version, protocol, serve)");
            std::process::exit(64);
        }
    }
}

fn run_protocol() {
    match ntgcalls::NTgCalls::get_protocol() {
        // Kiểm liên kết ntgcalls: phiên bản NTgCalls + các phiên bản tgcalls hỗ trợ (bản Python khai 8.0.0, 9.0.0, 12.0.0, 13.0.0).
        Ok(p) => println!("NTgCalls {} | tgcalls {:?} | udp_p2p={} udp_reflector={} layer {}..{}",
                          ntgcalls::version(), p.library_versions, p.udp_p2p, p.udp_reflector, p.min_layer, p.max_layer),
        Err(e) => {
            eprintln!("LỖI ntgcalls: {e:?}");
            std::process::exit(3);
        }
    }
}
