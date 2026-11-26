use axum::{http::header,response::{Html,IntoResponse,Response}};

// Nhúng tab thanh bên HA cần cho phép ĐÚNG GỐC (origin) của HA nhúng iframe; bản độc lập (Docker/Compose,
// không ai nhúng) giữ "frame-ancestors 'none'". HA (panel_iframe, HA Container — không có Supervisor/Ingress
// nên KHÔNG cùng gốc với Tele) và Tele thường ở hai cổng khác nhau trên CÙNG một host ⇒ 'self' KHÔNG đủ (khác
// gốc theo chuẩn CSP: khác cổng = khác origin) — phải liệt kê đúng gốc HA. TGGW_HA_INGRESS nhận THẲNG gốc đó
// (vd "http://172.16.10.200:8123"), không phải cờ bật/tắt — đọc lại cho đúng sau khi hiểu nhầm ban đầu.
fn frame_ancestors()->String {
    match std::env::var("TGGW_HA_INGRESS") {
        Ok(origin) if !origin.is_empty() => format!("frame-ancestors {origin}"),
        _ => "frame-ancestors 'none'".to_string(),
    }
}
fn page(html:&'static str)->Response {
    let mut response=Html(html).into_response();
    let csp=format!("default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; {}; form-action 'none'",frame_ancestors());
    for (name,value) in [
        (header::CONTENT_SECURITY_POLICY,csp),
        (header::X_CONTENT_TYPE_OPTIONS,"nosniff".to_string()),
        (header::REFERRER_POLICY,"no-referrer".to_string()),
        (header::CACHE_CONTROL,"no-store".to_string()),
    ] {response.headers_mut().insert(name,value.parse().unwrap());}
    response
}
pub(crate) async fn app()->Response { page(include_str!("../web/app.html")) }
pub(crate) async fn accounts()->Response { page(include_str!("../web/index.html")) }
pub(crate) async fn qrcode()->Response {
    ([(header::CONTENT_TYPE,"text/javascript; charset=utf-8"), (header::X_CONTENT_TYPE_OPTIONS,"nosniff"),
        (header::CACHE_CONTROL,"no-cache")], include_str!("../web/qrcode.min.js")).into_response()
}

fn script(body:&'static str)->Response {
    ([(header::CONTENT_TYPE,"text/javascript; charset=utf-8"), (header::X_CONTENT_TYPE_OPTIONS,"nosniff"), (header::CACHE_CONTROL,"no-cache")], body).into_response()
}
pub(crate) async fn emoji_js()->Response { script(include_str!("../web/emoji.js")) }
pub(crate) async fn emoji_data_js()->Response { script(include_str!("../web/emoji-data.js")) }
pub(crate) async fn wallpaper_art_js()->Response { script(include_str!("../web/wallpaper-art.js")) }
pub(crate) async fn favicon()->Response {
    ([(header::CONTENT_TYPE,"image/png"), (header::X_CONTENT_TYPE_OPTIONS,"nosniff"),
        (header::CACHE_CONTROL,"public, max-age=86400")], include_bytes!("../web/icon.png").as_slice()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn embedded_pages_have_security_headers() {
        for response in [app().await,accounts().await] {
            assert_eq!(response.status(),200);
            assert_eq!(response.headers()[header::X_CONTENT_TYPE_OPTIONS],"nosniff");
            assert_eq!(response.headers()[header::REFERRER_POLICY],"no-referrer");
            let csp=response.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
            assert!(csp.contains("connect-src 'self'"));
            assert!(csp.contains("frame-ancestors 'none'"),"mặc định (không chạy add-on) phải chặn nhúng iframe: {csp}");
        }
    }

    #[tokio::test]
    async fn favicon_serves_embedded_png_with_nosniff() {
        let response = favicon().await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(response.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn ha_ingress_env_sets_exact_origin_only_for_this_run() {
        let old=std::env::var_os("TGGW_HA_INGRESS");
        std::env::remove_var("TGGW_HA_INGRESS");
        assert_eq!(frame_ancestors(),"frame-ancestors 'none'","chưa đặt biến môi trường thì vẫn chặn iframe như cũ");
        std::env::set_var("TGGW_HA_INGRESS","'self'");
        assert_eq!(frame_ancestors(),"frame-ancestors 'self'","Supervisor Ingress thật (cùng gốc HA) dùng từ khoá 'self'");
        std::env::set_var("TGGW_HA_INGRESS","http://192.0.2.10:8123");
        assert_eq!(frame_ancestors(),"frame-ancestors http://192.0.2.10:8123","HA Container (panel_iframe, khác gốc) dùng đúng gốc HA");
        std::env::set_var("TGGW_HA_INGRESS","");
        assert_eq!(frame_ancestors(),"frame-ancestors 'none'","chuỗi rỗng vẫn coi như tắt");
        match old { Some(v)=>std::env::set_var("TGGW_HA_INGRESS",v), None=>std::env::remove_var("TGGW_HA_INGRESS") }
    }
}
