//! Gateway Telegram bản Rust — cột mốc 0: mở libtdjson và đọc phiên bản TDLib.
//!
//! Chạy: `TDJSON_LIB=/đường/tới/libtdjson.so tggw version|login|protocol`
mod contract;
mod td;

use std::time::Duration;

use crate::contract::TdLink;

#[tokio::main]
async fn main() {
    let path = std::env::var("TDJSON_LIB").unwrap_or_else(|_| "libtdjson.so".into());
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "version".into());
    if cmd == "protocol" {
        return run_protocol();
    }
    if cmd == "login" {
        if let Err(e) = run_login(&path).await {
            eprintln!("LỖI đăng nhập: {e}");
            std::process::exit(3);
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
            eprintln!("lệnh lạ: {other} (có: version, login, protocol)");
            std::process::exit(64);
        }
    }
}

async fn run_login(path: &str) -> Result<(), String> {
    let cfg = td::login::LoginConfig::from_env()?;
    let client = td::TdClient::open(path)?;
    let mut auth = client.auth_watch();
    let login = td::login::run(&client, &cfg);
    tokio::pin!(login);
    loop {
        tokio::select! {
            result = &mut login => {
                result?;
                break;
            }
            result = auth.changed() => {
                result.map_err(|_| "mất kết nối TDLib".to_owned())?;
                let state = auth.borrow_and_update().clone();
                if !state.is_empty() {
                    println!("Trạng thái đăng nhập: {state}");
                }
            }
        }
    }

    let me = client.request(serde_json::json!({"@type": "getMe"})).await?;
    let name = [me.get("first_name").and_then(|v| v.as_str()).unwrap_or_default(),
                me.get("last_name").and_then(|v| v.as_str()).unwrap_or_default()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let username = me.get("username").and_then(|v| v.as_str()).unwrap_or_default();
    if name.is_empty() {
        println!("Đăng nhập thành công: @{username}");
    } else if username.is_empty() {
        println!("Đăng nhập thành công: {name}");
    } else {
        println!("Đăng nhập thành công: {name} (@{username})");
    }
    client.close(Duration::from_secs(5)).await;
    Ok(())
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
