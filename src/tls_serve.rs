//! HTTPS tự ký TRÊN CÙNG CỔNG với HTTP (chủ máy 23/11: mic/webcam USB của trình duyệt chỉ chạy ở ngữ cảnh an toàn — HTTPS hoặc localhost).
//! Byte đầu của kết nối là 0x16 (TLS ClientHello) ⇒ bắt tay TLS; còn lại ⇒ HTTP thường như trước. Chứng chỉ tự sinh một lần, lưu ở
//! `<data>/tls/` (DER, quyền 0600); trình duyệt hỏi tin cậy MỘT lần rồi `getUserMedia` dùng được. Tắt bằng `GATEWAY_HTTPS=0`.
use std::{future::Future, net::SocketAddr, path::{Path, PathBuf}, sync::Arc};

use axum::{extract::ConnectInfo, Extension, Router};
use hyper_util::{rt::{TokioExecutor, TokioIo}, server::conn::auto::Builder, service::TowerToHyperService};
use rustls::{pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer}, ServerConfig};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

const TLS_HELLO: u8 = 0x16;

pub(crate) fn tls_dir() -> PathBuf {
    let data = std::env::var("TGGW_DATA").or_else(|_| std::env::var("DATA_DIR")).unwrap_or_else(|_| "/data".into());
    Path::new(&data).join("tls")
}

/// Tên/IP đưa vào chứng chỉ: localhost + `GATEWAY_TLS_NAMES` (phẩy) + địa chỉ nghe nếu cụ thể.
fn alt_names(addr: SocketAddr) -> Vec<String> {
    let mut names: Vec<String> = ["localhost", "127.0.0.1", "::1"].into_iter().map(String::from).collect();
    for n in std::env::var("GATEWAY_TLS_NAMES").unwrap_or_default().split(',') {
        let n = n.trim();
        if !n.is_empty() && !names.iter().any(|x| x == n) { names.push(n.to_owned()); }
    }
    if !addr.ip().is_unspecified() && !names.iter().any(|x| *x == addr.ip().to_string()) { names.push(addr.ip().to_string()); }
    names
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(bytes)
}

/// Đọc chứng chỉ đã lưu hoặc sinh mới (khoá ECDSA P-256, hạn 10 năm).
fn load_or_create(dir: &Path, names: Vec<String>) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (cert_path, key_path) = (dir.join("cert.der"), dir.join("key.der"));
    if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
        if !cert.is_empty() && !key.is_empty() { return Ok((cert, key)); }
    }
    let mut params = rcgen::CertificateParams::new(names).map_err(|e| format!("tham số chứng chỉ: {e}"))?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "telegram-gateway-rs (tự ký)");
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2044, 1, 1);
    let key = rcgen::KeyPair::generate().map_err(|e| format!("sinh khoá: {e}"))?;
    let cert = params.self_signed(&key).map_err(|e| format!("ký chứng chỉ: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("tạo {}: {e}", dir.display()))?;
    let (cert_der, key_der) = (cert.der().to_vec(), key.serialize_der());
    write_private(&cert_path, &cert_der).and_then(|()| write_private(&key_path, &key_der)).map_err(|e| format!("lưu chứng chỉ: {e}"))?;
    Ok((cert_der, key_der))
}

pub(crate) fn acceptor(dir: &Path, addr: SocketAddr) -> Result<TlsAcceptor, String> {
    let (cert, key) = load_or_create(dir, alt_names(addr))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().map_err(|e| format!("TLS: {e}"))?
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(cert)], PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)))
        .map_err(|e| format!("TLS: {e}"))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub(crate) fn enabled() -> bool {
    !matches!(std::env::var("GATEWAY_HTTPS").unwrap_or_default().trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off")
}

/// Vòng nhận kết nối: HTTP thường hoặc TLS tuỳ byte đầu. Dừng khi `shutdown` xong.
pub(crate) async fn serve_dual<F>(listener: TcpListener, app: Router, acceptor: TlsAcceptor, shutdown: F) -> Result<(), String>
where F: Future<Output = ()> + Send + 'static {
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => match accepted { Ok(v) => v, Err(e) => { log::warn!("accept lỗi: {e}"); continue; } },
        };
        let service = TowerToHyperService::new(app.clone().layer(Extension(ConnectInfo(peer))));
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let mut first = [0u8; 1];
            match tokio::time::timeout(std::time::Duration::from_secs(10), stream.peek(&mut first)).await {
                Ok(Ok(1)) => {}
                _ => return,
            }
            let builder = Builder::new(TokioExecutor::new());
            if first[0] == TLS_HELLO {
                if let Ok(tls) = acceptor.accept(stream).await { let _ = builder.serve_connection_with_upgrades(TokioIo::new(tls), service).await; }
            } else {
                let _ = builder.serve_connection_with_upgrades(TokioIo::new(stream), service).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HTTP thường và HTTPS (chứng chỉ tự ký) trả lời trên CÙNG một cổng; chứng chỉ được lưu và dùng lại ở lần sau.
    #[tokio::test]
    async fn http_and_https_share_one_port_and_the_certificate_is_reused() {
        let dir = std::env::temp_dir().join(format!("tggw-tls-test-{}", std::process::id()));
        let app = Router::new().route("/ping", axum::routing::get(|| async { "pong" }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = acceptor(&dir, addr).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_dual(listener, app, acceptor, async move { let _ = stopped.await; }));
        let client = reqwest::Client::builder().danger_accept_invalid_certs(true).build().unwrap();
        for scheme in ["http", "https"] {
            let body = client.get(format!("{scheme}://{addr}/ping")).send().await.unwrap().text().await.unwrap();
            assert_eq!(body, "pong", "{scheme}");
        }
        let first = std::fs::read(dir.join("cert.der")).unwrap();
        assert!(acceptor_ok(&dir, addr) && std::fs::read(dir.join("cert.der")).unwrap() == first, "chứng chỉ phải được dùng lại");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.join("key.der")).unwrap().permissions().mode() & 0o777, 0o600, "khoá riêng quyền 0600");
        let _ = stop.send(());
        server.await.unwrap().unwrap();
    }

    fn acceptor_ok(dir: &Path, addr: SocketAddr) -> bool { acceptor(dir, addr).is_ok() }
}
