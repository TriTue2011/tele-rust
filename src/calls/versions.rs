//! Cùng thứ tự chọn phiên bản với versions.py / NTgCalls best_match.
pub fn best_match<'a>(peer: &'a [String], ours: &[String]) -> Option<&'a str> {
    let key = |s: &str| s.split('.').map(str::parse::<i64>).collect::<Result<Vec<_>, _>>().unwrap_or_default();
    peer.iter().filter(|v| ours.contains(v)).max_by_key(|v| key(v)).map(String::as_str)
}

pub fn describe(peer: &[String], ours: &[String]) -> (Option<String>, String) {
    if peer.is_empty() { return (None, "bên kia không khai phiên bản".into()); }
    if let Some(version) = best_match(peer, ours) {
        let family = match version { "8.0.0" | "9.0.0" => "V2 Legacy", "12.0.0" | "13.0.0" => "V3 SCTP", _ => "?" };
        return (Some(version.into()), format!("{version} (báo hiệu {family})"));
    }
    let old = peer.iter().all(|v| ["2.4.4", "2.7.7", "4.0.0", "5.0.0", "6.0.0", "7.0.0"].contains(&v.as_str()));
    (None, format!("KHÔNG có phiên bản chung (bên kia {peer:?}, ta {ours:?}){}", if old { " — bản tgcalls cũ, cần cập nhật Telegram" } else { "" }))
}
