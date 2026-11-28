use std::{io, net::{IpAddr, Ipv4Addr}, sync::Arc, time::Duration};

use reqwest::{dns::{Addrs, Name, Resolve, Resolving}, header::{HeaderValue, CONTENT_TYPE}, redirect::Policy, Client, Url};
use serde_json::Value;
use tokio::sync::broadcast;

pub(crate) struct Webhook {
    client: Client,
    url: Url,
    token: HeaderValue,
}

impl Webhook {
    pub(crate) fn from_env() -> Option<Self> {
        let url = std::env::var("HA_WEBHOOK_URL").unwrap_or_default();
        if url.is_empty() { return None; }
        let token = std::env::var("HA_WEBHOOK_TOKEN").unwrap_or_default();
        let webhook = Self::new(&url, &token);
        if webhook.is_none() { log::warn!("Cấu hình webhook HA không hợp lệ. Sự kiện vẫn có trên API poll."); }
        webhook
    }

    pub(super) fn new(url: &str, token: &str) -> Option<Self> {
        let url = Url::parse(url).ok()?;
        let host = url.host_str()?;
        if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
            return None;
        }
        // IP viết trực tiếp trong URL không qua bộ phân giải của HTTP client.
        if host.trim_start_matches('[').trim_end_matches(']').parse().is_ok_and(blocked) {
            return None;
        }
        let mut token = HeaderValue::from_str(token).ok()?;
        token.set_sensitive(true);
        let client = Client::builder().redirect(Policy::none()).no_proxy().retry(reqwest::retry::never())
            .dns_resolver(Arc::new(FilteredDns)).timeout(Duration::from_secs(10)).build().ok()?;
        Some(Self { client, url, token })
    }

    pub(super) async fn follow(self, mut source: broadcast::Receiver<Value>) {
        loop {
            match source.recv().await {
                Ok(event) => self.deliver(event).await,
                Err(broadcast::error::RecvError::Lagged(_)) => log::warn!("Webhook HA chậm. Các sự kiện còn trong spool có thể lấy qua poll."),
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    async fn deliver(&self, event: Value) {
        let body = event.to_string();
        for attempt in 0..3 {
            let result = self.client.post(self.url.clone())
                .header(CONTENT_TYPE, "application/json").header("X-Gateway-Token", self.token.clone())
                .body(body.clone()).send().await;
            match result {
                Ok(response) if response.status().is_success() => return,
                Ok(response) if response.status().is_redirection() => break,
                _ => (),
            }
            if attempt < 2 { tokio::time::sleep(Duration::from_secs(1 << attempt)).await; }
        }
        log::warn!("Không gửi được sự kiện đến webhook HA. Có thể lấy qua poll.");
    }
}

fn blocked_v4(ip: Ipv4Addr) -> bool {
    ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast()
}

fn blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => blocked_v4(ip),
        IpAddr::V6(ip) => {
            let s = ip.segments();
            let embedded_v4 = ip.to_ipv4().or_else(|| {
                if (s[0] == 0x0064 && s[1] == 0xff9b && s[2..6] == [0_u16; 4])
                    || (s[0..4] == [0_u16; 4] && s[4] == 0xffff && s[5] == 0)
                {
                    let b = ip.octets();
                    Some(Ipv4Addr::new(b[12], b[13], b[14], b[15]))
                } else {
                    None
                }
            });
            let is_aws_imds_v6 = s == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254];
            ip.is_unicast_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || is_aws_imds_v6
                || embedded_v4.is_some_and(blocked_v4)
        }
    }
}

struct FilteredDns;

impl Resolve for FilteredDns {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            // Dùng chính các địa chỉ đã lọc để nối socket, kể cả khi thử lại.
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?
                .filter(|addr| !blocked(addr.ip())).collect();
            if addresses.is_empty() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Đích webhook bị chặn.").into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}
