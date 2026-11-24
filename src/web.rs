use axum::{http::header,response::{Html,IntoResponse,Response}};

fn page(html:&'static str)->Response {
    let mut response=Html(html).into_response();
    for (name,value) in [
        (header::CONTENT_SECURITY_POLICY,"default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        (header::X_CONTENT_TYPE_OPTIONS,"nosniff"),
        (header::REFERRER_POLICY,"no-referrer"),
        (header::CACHE_CONTROL,"no-store"),
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
            assert!(response.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("connect-src 'self'"));
        }
    }
}
