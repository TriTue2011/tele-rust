use std::{io, net::IpAddr, sync::Arc, time::Duration};

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
        if webhook.is_none() { eprintln!("Cấu hình webhook HA không hợp lệ. Sự kiện vẫn có trên API poll."); }
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
                Err(broadcast::error::RecvError::Lagged(_)) => eprintln!("Webhook HA chậm. Các sự kiện còn trong spool có thể lấy qua poll."),
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
        eprintln!("Không gửi được sự kiện đến webhook HA. Có thể lấy qua poll.");
    }
}

fn blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_unicast_link_local() || ip.to_ipv4().is_some_and(|ip| ip.is_link_local()),
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
