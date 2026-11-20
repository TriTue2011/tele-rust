use std::{io, net::Ipv4Addr, sync::Arc, time::Duration};

use prost::Message;
use rustls::{client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime}, DigitallySignedStruct, SignatureScheme};
use serde_json::{json, Value};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt}, net::{TcpStream, UdpSocket}, sync::oneshot, time};

use super::{local_cast_ip, Target};

const CAST_PORT: u16 = 8009;
const APP_ID: &str = "CC1AD845";
const RECEIVER: &str = "receiver-0";
const CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const CONTROL: &str = "urn:x-cast:com.google.cast.receiver";
const MEDIA: &str = "urn:x-cast:com.google.cast.media";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

pub(super) async fn run(target: Target, rest_port: u16, path: String, mut stop: oneshot::Receiver<()>) -> io::Result<()> {
    let startup = async {
        let ip = match target { Target::Ip(ip) => ip, Target::Id(id) => discover(&id).await? };
        let socket = TcpStream::connect((ip, CAST_PORT)).await?;
        let addr = socket.local_addr()?.ip();
        let config = rustls::ClientConfig::builder().dangerous()
            .with_custom_certificate_verifier(Arc::new(LocalCertificate)).with_no_client_auth();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(ServerName::IpAddress(std::net::IpAddr::V4(ip).into()), socket).await?;
        Ok::<_, io::Error>((format!("http://{addr}:{rest_port}{path}"), tls))
    };
    let (url, socket) = tokio::select! {
        _ = &mut stop => return Ok(()),
        result = time::timeout(REQUEST_TIMEOUT, startup) => result??,
    };
    control(socket, &url, stop).await
}

async fn control<S: AsyncRead + AsyncWrite + Unpin>(socket: S, url: &str, mut stop: oneshot::Receiver<()>) -> io::Result<()> {
    let (mut input, mut output) = tokio::io::split(socket);
    let launch = async {
        send(&mut output, RECEIVER, CONNECTION, json!({"type":"CONNECT"})).await?;
        send(&mut output, RECEIVER, CONTROL, json!({"type":"LAUNCH","appId":APP_ID,"requestId":1})).await?;
        loop {
            let (message, payload) = read(&mut input).await?;
            pong(&mut output, &message, &payload).await?;
            if message.namespace != CONTROL || payload["type"] != "RECEIVER_STATUS" { continue; }
            let Some(app) = payload["status"]["applications"].as_array().and_then(|a| a.iter().find(|a| a["appId"] == APP_ID)) else { continue };
            let transport = app["transportId"].as_str().filter(|s| !s.is_empty()).ok_or_else(invalid)?;
            let session = app["sessionId"].as_str().filter(|s| !s.is_empty()).ok_or_else(invalid)?;
            send(&mut output, transport, CONNECTION, json!({"type":"CONNECT"})).await?;
            send(&mut output, transport, MEDIA, json!({"type":"LOAD","requestId":2,"autoplay":true,
                "media":{"contentId":url,"streamType":"BUFFERED","contentType":"audio/mpeg",
                    "metadata":{"metadataType":0,"title":"Cuộc gọi"}}})).await?;
            return Ok::<_, io::Error>(session.to_owned());
        }
    };
    let session = tokio::select! {
        _ = &mut stop => return Ok(()),
        result = time::timeout(REQUEST_TIMEOUT, launch) => result??,
    };

    let mut ping = time::interval(Duration::from_secs(5));
    loop {
        let receive = read(&mut input);
        tokio::pin!(receive);
        loop {
            tokio::select! {
                _ = &mut stop => {
                    let message = json!({"type":"STOP","requestId":3,"sessionId":session});
                    return time::timeout(Duration::from_millis(500), send(&mut output, RECEIVER, CONTROL, message)).await?;
                }
                result = &mut receive => {
                    let (message, payload) = result?;
                    pong(&mut output, &message, &payload).await?;
                    break;
                }
                _ = ping.tick() => send(&mut output, RECEIVER, HEARTBEAT, json!({"type":"PING"})).await?,
            }
        }
    }
}

#[derive(Clone, Message)]
struct CastMessage {
    #[prost(int32, required, tag="1")]
    protocol: i32,
    #[prost(string, required, tag="2")]
    source: String,
    #[prost(string, required, tag="3")]
    dest: String,
    #[prost(string, required, tag="4")]
    namespace: String,
    #[prost(int32, required, tag="5")]
    payload_type: i32,
    #[prost(string, optional, tag="6")]
    payload: Option<String>,
}

async fn send(output: &mut (impl AsyncWrite + Unpin), dest: &str, namespace: &str, payload: Value) -> io::Result<()> {
    let message = CastMessage { protocol: 0, source: "sender-0".into(), dest: dest.into(),
        namespace: namespace.into(), payload_type: 0, payload: Some(payload.to_string()) };
    time::timeout(Duration::from_secs(2), write(output, message)).await?
}

async fn write(output: &mut (impl AsyncWrite + Unpin), message: CastMessage) -> io::Result<()> {
    let bytes = message.encode_to_vec();
    output.write_u32(bytes.len() as u32).await?;
    output.write_all(&bytes).await?;
    output.flush().await
}

async fn read(input: &mut (impl AsyncRead + Unpin)) -> io::Result<(CastMessage, Value)> {
    let len = input.read_u32().await?;
    if len == 0 || len > 64 * 1024 { return Err(invalid()); }
    let mut bytes = vec![0; len as usize];
    input.read_exact(&mut bytes).await?;
    let message = CastMessage::decode(bytes.as_slice()).map_err(|_| invalid())?;
    let payload = message.payload.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
    Ok((message, payload))
}

async fn pong(output: &mut (impl AsyncWrite + Unpin), message: &CastMessage, payload: &Value) -> io::Result<()> {
    if message.namespace == HEARTBEAT && payload["type"] == "PING" {
        send(output, &message.source, HEARTBEAT, json!({"type":"PONG"})).await?;
    }
    Ok(())
}

fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "Phản hồi Cast không hợp lệ.") }

async fn discover(id: &str) -> io::Result<Ipv4Addr> {
    const QUERY: &[u8] = b"\x00\x00\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x0b_googlecast\x04_tcp\x05local\0\0\x0c\0\x01";
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.set_multicast_ttl_v4(2)?;
    let mut tick = time::interval(Duration::from_secs(1));
    let mut packet = [0; 9000];
    let target = format!("id={id}");
    time::timeout(Duration::from_secs(3), async {
        loop {
            tokio::select! {
                _ = tick.tick() => { socket.send_to(QUERY, (Ipv4Addr::new(224,0,0,251), 5353)).await?; }
                result = socket.recv_from(&mut packet) => {
                    let (size, peer) = result?;
                    let std::net::SocketAddr::V4(peer) = peer else { continue };
                    if local_cast_ip(*peer.ip()) && has_cast_id(&packet[..size], &target) { return Ok(*peer.ip()); }
                }
            }
        }
    }).await?
}

fn has_cast_id(packet: &[u8], target: &str) -> bool {
    // Chỉ nhận trường TXT của một phản hồi mDNS, không dò chuỗi tùy ý trong gói.
    let Some(header) = packet.get(..12) else { return false };
    if header[2] & 0x80 == 0 { return false; }
    let number = |index| u16::from_be_bytes([header[index], header[index + 1]]) as usize;
    let mut rest = &packet[12..];
    for _ in 0..number(4) {
        let Some(tail) = after_name(rest).and_then(|t| t.get(4..)) else { return false };
        rest = tail;
    }
    for _ in 0..number(6) + number(8) + number(10) {
        let Some(tail) = after_name(rest) else { return false };
        let Some(rr) = tail.get(..10) else { return false };
        let len = u16::from_be_bytes([rr[8], rr[9]]) as usize;
        let Some(data) = tail.get(10..10+len) else { return false };
        rest = &tail[10+len..];
        if rr[..2] != [0, 16] { continue; }
        let mut data = data;
        while let Some((&len, tail)) = data.split_first() {
            let Some(item) = tail.get(..len as usize) else { break };
            if item.eq_ignore_ascii_case(target.as_bytes()) { return true; }
            data = &tail[len as usize..];
        }
    }
    false
}

fn after_name(mut field: &[u8]) -> Option<&[u8]> {
    while let Some((&n, rest)) = field.split_first() {
        match n {
            0 => return Some(rest),
            1..=63 => field = rest.get(n as usize..)?,
            192..=255 => return rest.get(1..),
            _ => return None,
        }
    }
    None
}

// Google Cast dùng chứng chỉ tự ký. Chỉ dùng xác minh này với IP LAN đã kiểm tra.
// Chữ ký TLS vẫn phải khớp khóa trong chứng chỉ mà loa trình bày.
#[derive(Debug)]
struct LocalCertificate;

impl ServerCertVerifier for LocalCertificate {
    fn verify_server_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>,
        _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> { Ok(ServerCertVerified::assertion()) }

    fn verify_tls12_signature(&self, bytes: &[u8], cert: &CertificateDer<'_>, signed: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(bytes, cert, signed, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }

    fn verify_tls13_signature(&self, bytes: &[u8], cert: &CertificateDer<'_>, signed: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(bytes, cert, signed, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_receiver_launches_buffered_mp3_and_stops_session() {
        let (client, mut receiver) = tokio::io::duplex(4096);
        let (stop, stopped) = oneshot::channel();
        let client = tokio::spawn(control(client, "http://test/cast/audio.mp3", stopped));
        let (_, connect) = read(&mut receiver).await.unwrap();
        assert_eq!(connect["type"], "CONNECT");
        let (_, launch) = read(&mut receiver).await.unwrap();
        assert_eq!(launch["appId"], APP_ID);
        let status = json!({"type":"RECEIVER_STATUS","status":{"applications":[
            {"appId":APP_ID,"transportId":"media-1","sessionId":"test-session"}]}});
        write(&mut receiver, CastMessage { protocol:0, source:RECEIVER.into(), dest:"sender-0".into(),
            namespace:CONTROL.into(), payload_type:0, payload:Some(status.to_string()) }).await.unwrap();
        let (connect, _) = read(&mut receiver).await.unwrap();
        assert_eq!(connect.dest, "media-1");
        let (_, load) = read(&mut receiver).await.unwrap();
        assert_eq!(load["media"]["streamType"], "BUFFERED");
        assert_eq!(load["media"]["contentId"], "http://test/cast/audio.mp3");
        stop.send(()).unwrap();
        let (_, mut message) = read(&mut receiver).await.unwrap();
        if message["type"] == "PING" { message = read(&mut receiver).await.unwrap().1; }
        assert_eq!(message, json!({"type":"STOP","requestId":3,"sessionId":"test-session"}));
        client.await.unwrap().unwrap();
    }
}
