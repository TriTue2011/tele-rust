//! Lỗi cấu hình phải xảy ra trước khi nạp TDLib hoặc có thể kết nối Telegram.
use std::process::Command;

#[test]
fn invalid_env_refuses_startup_in_vietnamese_without_echoing_values() {
    for (key, value) in [("GATEWAY_MOCK", "1"), ("GATEWAY_MOCK", "bad-secret-value"),
        ("LOG_LEVEL", "bad-secret-value"), ("TELEGRAM_DEBUG", "bad-secret-value"),
        ("TELEGRAM_RX_VIDEO", "bad-secret-value"), ("TTS_VOICE", "bad-secret-value\n"),
        ("TTS_RATE", "bad-secret-value"), ("TELEGRAM_CALLTRACE_KEEP", "-1"),
        ("TELEGRAM_CALLTRACE_KEEP_BAD", "bad-secret-value"), ("TELEGRAM_CALLTRACE_MAX_KB", "0"),
        ("TELEGRAM_WEBRTC_LOG", "bad-secret-value"), ("TELEGRAM_WEBRTC_LOG_MB", "0"),
        ("TELEGRAM_WEBRTC_LOG_LOC", "[")] {
        let out = Command::new(env!("CARGO_BIN_EXE_tggw")).env_clear()
            .env("TGGW_DATA", "target/env-tests").env("TDJSON_LIB", "missing-tdlib-must-never-load")
            .env(key, value).arg("serve").output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{key}");
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(error.contains(key) && error.contains("LỖI"), "{key}: {error}");
        assert!(!error.contains("bad-secret-value") && !error.contains("missing-tdlib-must-never-load"), "{key}: {error}");
    }
}
