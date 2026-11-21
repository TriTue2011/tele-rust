//! Gateway Telegram bản Rust — cột mốc 0: mở libtdjson và đọc phiên bản TDLib.
//!
//! Chạy: `TDJSON_LIB=/đường/tới/libtdjson.so tggw version|login|protocol|serve`

use std::{io::{self, IsTerminal, Write}, process::Command, time::Duration};

use telegram_gateway_rs::contract::TdLink;
use telegram_gateway_rs::td;

#[tokio::main]
async fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "version".into());
    let data_dir = std::env::var("TGGW_DATA").or_else(|_| std::env::var("DATA_DIR")).unwrap_or_else(|_| "/data".into());
    if cmd == "passwd" {
        if let Err(error) = run_passwd(&data_dir) {
            eprintln!("LỖI đặt lại mật khẩu: {error}");
            std::process::exit(2);
        }
        return;
    }
    if let Err(e) = telegram_gateway_rs::config::validate() {
        eprintln!("LỖI cấu hình: {e}");
        std::process::exit(2);
    }
    if let Err(e) = telegram_gateway_rs::logging::init(data_dir.into()) {
        eprintln!("LỖI cấu hình nhật ký: {e}");
        std::process::exit(2);
    }
    let path = std::env::var("TDJSON_LIB").unwrap_or_else(|_| "libtdjson.so".into());
    if cmd == "protocol" {
        return run_protocol();
    }
    if cmd == "login" {
        if let Err(e) = run_login(&path).await {
            log::error!("LỖI đăng nhập: {e}");
            std::process::exit(3);
        }
        return;
    }
    if cmd == "serve" {
        if let Err(e) = telegram_gateway_rs::app::App::start().await {
            log::error!("LỖI: {e}");
            std::process::exit(2);
        }
        return;
    }
    let td = match td::TdJson::open(&path) {
        Ok(t) => t,
        Err(e) => {
            log::error!("LỖI: {e}");
            std::process::exit(2);
        }
    };
    match cmd.as_str() {
        "version" => {
            // Tắt nhật ký TDLib ra stderr (mặc định khá ồn) trước khi hỏi.
            td.execute(&serde_json::json!({"@type": "setLogVerbosityLevel", "new_verbosity_level": 0}));
            match td.version() {
                Some(v) => println!("TDLib {v}"),
                None => {
                    log::error!("LỖI: TDLib không trả phiên bản");
                    std::process::exit(3);
                }
            }
        }
        other => {
            log::error!("lệnh lạ: {other} (có: version, login, protocol, serve, passwd)");
            std::process::exit(64);
        }
    }
}

fn run_passwd(data_dir: &str) -> Result<(), String> {
    let password = match std::env::var("TG_UI_PASSWORD") {
        Ok(password) => password,
        Err(_) => {
            eprint!("Mật khẩu mới: ");
            io::stderr().flush().map_err(|_| "Không đọc được mật khẩu từ terminal.".to_owned())?;
            let hidden = io::stdin().is_terminal();
            if hidden && !Command::new("stty").arg("-echo").status().map_err(|_| "Không thể tắt echo terminal.".to_owned())?.success() {
                return Err("Không thể tắt echo terminal.".into());
            }
            let mut password = String::new();
            let read = io::stdin().read_line(&mut password);
            let restored = if hidden { Command::new("stty").arg("echo").status().map_err(|_| "Không khôi phục được echo terminal.".to_owned())?.success() } else { true };
            if hidden { eprintln!(); }
            read.map_err(|_| "Không đọc được mật khẩu từ terminal.".to_owned())?;
            if !restored { return Err("Không khôi phục được echo terminal.".into()); }
            while password.ends_with(['\n', '\r']) { password.pop(); }
            password
        }
    };
    telegram_gateway_rs::ui_auth::UiAuth::reset_password(std::path::Path::new(data_dir), &password)?;
    println!("Đã đặt lại mật khẩu đăng nhập web.");
    Ok(())
}

async fn run_login(path: &str) -> Result<(), String> {
    let cfg = td::login::LoginConfig::from_env()?;
    let client = td::TdClient::open(path, &cfg.session_dir())?;
    let result = async {
        td::login::run(&client, &cfg).await?;
        let me = client.request(serde_json::json!({"@type": "getMe"})).await.map_err(|e| e.message)?;
        let name = [me.get("first_name").and_then(|v| v.as_str()).unwrap_or_default(),
                    me.get("last_name").and_then(|v| v.as_str()).unwrap_or_default()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        println!("Đăng nhập thành công: {name}");
        Ok(())
    }.await;
    // Kể cả đăng nhập/getMe thất bại vẫn chờ TDLib ghi DB và nhả khoá.
    client.close(Duration::from_secs(5)).await;
    result
}

fn run_protocol() {
    match ntgcalls::NTgCalls::get_protocol() {
        // Kiểm liên kết ntgcalls: phiên bản NTgCalls + các phiên bản tgcalls hỗ trợ (bản Python khai 8.0.0, 9.0.0, 12.0.0, 13.0.0).
        Ok(p) => println!("NTgCalls {} | tgcalls {:?} | udp_p2p={} udp_reflector={} layer {}..{}",
                          ntgcalls::version(), p.library_versions, p.udp_p2p, p.udp_reflector, p.min_layer, p.max_layer),
        Err(e) => {
            log::error!("LỖI ntgcalls: {e:?}");
            std::process::exit(3);
        }
    }
}
