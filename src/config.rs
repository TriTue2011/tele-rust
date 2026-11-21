//! Đọc ở biên; lỗi chỉ nêu tên biến và điều kiện, không in giá trị.
use regex::Regex;

pub(crate) fn alias(get: &impl Fn(&str) -> Option<String>, rust: &str, python: &str) -> Option<String> {
    get(rust).or_else(|| get(python))
}

pub(crate) fn rx_video(text: &str) -> Result<(i16, i16, u8), String> {
    let invalid = || "TELEGRAM_RX_VIDEO phải có dạng WxH@F, rộng 160–3840, cao 120–2160, fps 5–60.".to_owned();
    let re = Regex::new(r"^\s*(\d{3,4})x(\d{3,4})@(\d{1,2})\s*$").unwrap();
    let cap = re.captures(text).ok_or_else(invalid)?;
    let w: i16 = cap[1].parse().map_err(|_| invalid())?;
    let h: i16 = cap[2].parse().map_err(|_| invalid())?;
    let f: u8 = cap[3].parse().map_err(|_| invalid())?;
    if !(160..=3840).contains(&w) || !(120..=2160).contains(&h) || !(5..=60).contains(&f) { return Err(invalid()); }
    Ok((w, h, f))
}

pub(crate) fn tts(voice: &str, rate: &str) -> Result<(String, String), String> {
    tts_from_lookup(voice, rate, |key| std::env::var(key).ok())
}

fn tts_from_lookup(voice: &str, rate: &str, get: impl Fn(&str) -> Option<String>) -> Result<(String, String), String> {
    let voice = if voice.is_empty() { get("TTS_VOICE").unwrap_or_else(|| "vi-VN-HoaiMyNeural".into()) } else { voice.into() };
    let rate = if rate.is_empty() { get("TTS_RATE").unwrap_or_else(|| "+0%".into()) } else { rate.into() };
    validate_tts(&voice, &rate)?;
    Ok((voice, rate))
}

fn validate_tts(voice: &str, rate: &str) -> Result<(), String> {
    if voice.is_empty() || voice.len() > 128 || !voice.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("TTS_VOICE phải là tên giọng gồm chữ, số và dấu gạch ngang (1–128 ký tự).".into());
    }
    if !Regex::new(r"^[+-]\d{1,3}%$").unwrap().is_match(rate) {
        return Err("TTS_RATE phải có dạng +N% hoặc -N%, từ -100% đến +100%.".into());
    }
    let n = rate[..rate.len()-1].parse::<i16>().map_err(|_| "TTS_RATE không hợp lệ.".to_owned())?;
    if !(-100..=100).contains(&n) { return Err("TTS_RATE phải từ -100% đến +100%.".into()); }
    Ok(())
}

pub fn validate() -> Result<(), String> {
    match std::env::var("GATEWAY_MOCK").unwrap_or_else(|_| "0".into()).trim() {
        "0" => (),
        "1" => return Err("GATEWAY_MOCK=1: bản Rust chưa hỗ trợ chế độ giả lập; đặt GATEWAY_MOCK=0 để chạy.".into()),
        _ => return Err("GATEWAY_MOCK phải là 0 hoặc 1.".into()),
    }
    crate::calls::trace::Config::from_env()?;
    crate::logging::level(|key| std::env::var(key).ok())?;
    rx_video(&std::env::var("TELEGRAM_RX_VIDEO").unwrap_or_else(|_| "640x360@15".into()))?;
    tts("", "")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rx_negotiation_and_tts_values_validate_at_boundary() {
        assert_eq!(rx_video("1280x720@30").unwrap(), (1280, 720, 30));
        assert_eq!(rx_video(" 640x360@15 ").unwrap(), (640, 360, 15));
        for bad in ["bad", "640x360", "99999x720@30", "640x360@0", "640x360@61"] { assert!(rx_video(bad).unwrap_err().contains("TELEGRAM_RX_VIDEO")); }
        assert!(validate_tts("vi-VN-HoaiMyNeural", "+0%").is_ok());
        assert!(validate_tts("en-US-AriaNeural", "-20%").is_ok());
        for bad in ["", "voice\nsecret", "--file=/etc/passwd"] { assert!(validate_tts(bad, "+0%").unwrap_err().contains("TTS_VOICE")); }
        for bad in ["1", "fast", "+101%", "-101%", "+0%\n"] { assert!(validate_tts("vi-VN-HoaiMyNeural", bad).unwrap_err().contains("TTS_RATE")); }
        let env = |k: &str| Some(if k == "TTS_VOICE" { "en-US-AriaNeural" } else { "-20%" }.into());
        assert_eq!(tts_from_lookup("", "", env).unwrap(), ("en-US-AriaNeural".into(), "-20%".into()));
        assert_eq!(tts_from_lookup("vi-VN-NamMinhNeural", "+15%", env).unwrap(), ("vi-VN-NamMinhNeural".into(), "+15%".into()));
        assert_eq!(tts_from_lookup("", "", |_| None).unwrap(), ("vi-VN-HoaiMyNeural".into(), "+0%".into()));
    }
}
