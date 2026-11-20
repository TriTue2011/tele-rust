//! Cầu sự kiện Telegram → spool, REST và Home Assistant.
mod spool;
mod webhook;
#[cfg(test)]
mod tests;

use std::{path::Path, sync::{Arc, Mutex}, time::{Duration, SystemTime, UNIX_EPOCH}};

use serde_json::{json, Value};
use tokio::{sync::{broadcast, mpsc, oneshot}, task::JoinHandle};

use crate::contract::GwError;
use spool::Spool;
pub(crate) use webhook::Webhook;

#[derive(Clone)]
pub(crate) struct EventSink {
    account: Arc<str>,
    queue: mpsc::Sender<Value>,
}

impl EventSink {
    pub(crate) fn emit(&self, kind: &str, data: Value) {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        let event = json!({"type":kind,"account":self.account.as_ref(),"timestamp":timestamp,"data":data});
        if let Err(mpsc::error::TrySendError::Full(_)) = self.queue.try_send(event) {
            eprintln!("Hàng đợi sự kiện đầy.");
        }
    }

    pub(crate) async fn follow_calls(self, mut source: broadcast::Receiver<Value>) {
        loop {
            match source.recv().await {
                Ok(event) => self.call(&event),
                Err(broadcast::error::RecvError::Lagged(_)) => eprintln!("Mất cập nhật sự kiện cuộc gọi."),
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    fn call(&self, event: &Value) {
        let call = &event["call"];
        let id = &call["id"];
        match event["kind"].as_str() {
            Some("started") => self.emit("call_started", json!({"call_id":id,"peer":event["peer"],"mode":"live","mock":false})),
            Some("incoming") => self.emit("call_incoming", json!({"call_id":id,"peer":mask_peer_id(&call["user_id"]),"mode":"live","mock":false})),
            Some("active") if call["outgoing"] == false => self.emit("call_started", json!({"call_id":id,"peer":mask_peer_id(&call["user_id"]),"mode":"live","mock":false})),
            Some("ended") if event["reason"].is_string() => self.emit("call_ended", json!({
                "call_id":id,"reason":event["reason"],"duration_seconds":call["seconds"]})),
            _ => (),
        }
    }
}

pub(crate) fn mask_peer(peer: &str) -> String {
    let chars: Vec<_> = peer.chars().collect();
    match chars.as_slice() {
        [] => String::new(),
        [a, b, .., y, z] if chars.len() > 4 => format!("{a}{b}••••{y}{z}"),
        _ => "••••".into(),
    }
}

pub(crate) fn mask_peer_id(id: &Value) -> String {
    id.as_i64().map_or_else(String::new, |id| mask_peer(&id.to_string()))
}

#[derive(Clone)]
pub(crate) struct EventStore(Arc<Mutex<Spool>>);

impl EventStore {
    pub(crate) async fn poll(&self, since: i64) -> Result<Value, GwError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || store.0.lock().unwrap().poll(since.max(0) as u64))
            .await.map_err(|_| GwError::new(500, "EVENT_STORAGE", "Không đọc được sự kiện."))
    }
}

pub(crate) struct EventBridge {
    pub(crate) store: EventStore,
    pub(crate) sink: EventSink,
    stop: Option<oneshot::Sender<()>>,
    worker: JoinHandle<()>,
    webhook: Option<JoinHandle<()>>,
}

impl EventBridge {
    pub(crate) async fn start(data_dir: &Path, account: &str, webhook: Option<Webhook>) -> Result<Self, String> {
        let path = data_dir.join("events");
        let spool = tokio::task::spawn_blocking(move || Spool::open(path)).await
            .ok().and_then(Result::ok).ok_or("Không mở được thư mục sự kiện.")?;
        let store = EventStore(Arc::new(Mutex::new(spool)));
        let (tx, mut queue) = mpsc::channel(1024);
        let (stop, mut stopping) = oneshot::channel();
        let (published, source) = broadcast::channel(256);
        let webhook = webhook.map(|webhook| tokio::spawn(webhook.follow(source)));
        let writer = store.clone();
        let worker = tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    _ = &mut stopping, if !queue.is_closed() => { queue.close(); continue; }
                    event = queue.recv() => event,
                };
                let Some(event) = event else { break };
                let writer = writer.clone();
                let written = tokio::task::spawn_blocking(move || writer.0.lock().unwrap().append(event)).await;
                match written {
                    Ok(Ok(event)) => { let _ = published.send(event); }
                    _ => eprintln!("Không ghi được sự kiện."),
                }
            }
        });
        Ok(Self { store, sink: EventSink { account: account.into(), queue: tx }, stop: Some(stop), worker, webhook })
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() { let _ = stop.send(()); }
        if tokio::time::timeout(Duration::from_secs(5), &mut self.worker).await.is_err() {
            self.worker.abort();
        }
    }
}

impl Drop for EventBridge {
    fn drop(&mut self) {
        if let Some(webhook) = &self.webhook { webhook.abort(); }
    }
}
