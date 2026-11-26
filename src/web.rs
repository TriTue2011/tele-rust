use axum::{http::header,response::{Html,IntoResponse,Response}};

// Add-on HA (Ingress) cần nhúng được trong iframe của HA; bản độc lập (Docker/Compose) giữ "frame-ancestors 'none'"
// vì không ai nhúng nó cả — chỉ bớt bảo mật đúng chỗ cần, không đổi mặc định cho mọi cách chạy.
fn frame_ancestors()->&'static str {
    // Ingress của HA proxy xuyên suốt qua chính máy chủ HA: trình duyệt thấy trang này cùng gốc (origin) với HA,
    // nên 'self' là đủ — không cần liệt kê domain HA cụ thể (mỗi nhà có domain/IP khác nhau, không đoán được).
    if std::env::var("TGGW_HA_INGRESS").is_ok_and(|v| v=="1"||v.eq_ignore_ascii_case("true")) { "frame-ancestors 'self'" } else { "frame-ancestors 'none'" }
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

    #[test]
    fn ha_ingress_env_relaxes_frame_ancestors_only_for_this_run() {
        let old=std::env::var_os("TGGW_HA_INGRESS");
        std::env::remove_var("TGGW_HA_INGRESS");
        assert_eq!(frame_ancestors(),"frame-ancestors 'none'","chưa đặt biến môi trường thì vẫn chặn iframe như cũ");
        std::env::set_var("TGGW_HA_INGRESS","1");
        assert_eq!(frame_ancestors(),"frame-ancestors 'self'","bật cờ add-on thì phải cho nhúng cùng gốc với HA");
        std::env::set_var("TGGW_HA_INGRESS","0");
        assert_eq!(frame_ancestors(),"frame-ancestors 'none'","đặt giá trị khác 1/true thì vẫn coi như tắt");
        match old { Some(v)=>std::env::set_var("TGGW_HA_INGRESS",v), None=>std::env::remove_var("TGGW_HA_INGRESS") }
    }
}
