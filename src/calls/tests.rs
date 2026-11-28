use super::*;
use crate::contract::TdError;
use std::sync::Mutex;
use ntgcalls::{Frame, FrameData, MediaSource, VideoRotation};
use media::MediaFuture;

const R1: &str = "http://192.0.2.10:8123/api/r1_speaker/intercom/01TESTENTRYID000000000000A?k=ab-cd_EF%20x";
const DUPLEX: &str = "https://ha.local/api/dahua_talk/duplex/ABC123?k=tok";

fn strings(values: &[&str]) -> Vec<String> { values.iter().map(|s| s.to_string()).collect() }

struct FakeTd {
    requests: Arc<Mutex<Vec<Value>>>,
    updates: broadcast::Sender<Value>,
    early: Mutex<Option<Value>>,
    fail: Mutex<Option<String>>,
}

impl TdLink for FakeTd {
    fn request(&self, req: Value) -> BoxFut<'_, Result<Value, TdError>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(req.clone());
            if let Some(kind) = self.fail.lock().unwrap().as_ref() {
                if req["@type"].as_str() == Some(kind) { return Err(TdError::transport(format!("lỗi có khóa {R1}"))); }
            }
            if req["@type"] == "createCall" {
                if let Some(update) = self.early.lock().unwrap().take() { let _ = self.updates.send(update); }
                Ok(json!({"@type":"callId","call_id":7}))
            } else if req["@type"] == "searchPublicChat" {
                Ok(json!({"type":{"@type":"chatTypePrivate","user_id":42}}))
            } else if req["@type"] == "searchUserByPhoneNumber" {
                Ok(json!({"@type":"user","id":99}))
            } else { Ok(json!({"@type":"ok"})) }
        })
    }
    fn updates(&self) -> broadcast::Receiver<Value> { self.updates.subscribe() }
}

#[derive(Default)]
struct FakeState {
    ops: Vec<MediaOp>,
    callback: Option<mpsc::UnboundedSender<Command>>,
    frames: Option<mpsc::Sender<MediaEvent>>,
    audio: Option<mpsc::Sender<MediaEvent>>,
    fail: Option<&'static str>,
}

struct FakeMedia(Arc<Mutex<FakeState>>);

fn op_name(op: &MediaOp) -> &'static str {
    match op { MediaOp::Create(_) => "create", MediaOp::Sources(_,StreamMode::Capture,_) => "capture", MediaOp::Sources(_,StreamMode::Playback,_) => "playback",
        MediaOp::Skip(..) => "skip", MediaOp::Connect(..) => "connect", MediaOp::Signaling(..) => "signaling", MediaOp::Stop(_) => "stop" }
}

impl Media for FakeMedia {
    fn protocol(&self) -> Result<Protocol, String> {
        Ok(Protocol { min_layer:92,max_layer:92,udp_p2p:true,udp_reflector:true,library_versions:strings(&["8.0.0","9.0.0","12.0.0","13.0.0"]) })
    }
    fn callbacks(&mut self, sender: mpsc::UnboundedSender<Command>, frames: mpsc::Sender<MediaEvent>, audio: mpsc::Sender<MediaEvent>) {
        let mut s = self.0.lock().unwrap();
        s.callback = Some(sender);
        s.frames = Some(frames);
        s.audio = Some(audio);
    }
    fn execute(&mut self, op: MediaOp) -> MediaFuture<'_> {
        Box::pin(async move {
            let mut s = self.0.lock().unwrap();
            let fail = s.fail == Some(op_name(&op));
            s.ops.push(op);
            if fail { Err(format!("lỗi chứa URL {R1}")) } else { Ok(()) }
        })
    }
}

struct Fixture { manager: Option<CallManager>, td: Arc<FakeTd>, media: Arc<Mutex<FakeState>>, dir: PathBuf }

impl Fixture {
    fn new(p2p: bool) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let dir = PathBuf::from(format!("target/calls-tests/{}-{}",std::process::id(),SEQ.fetch_add(1,Ordering::Relaxed)));
        let (updates,_) = broadcast::channel(256);
        let td = Arc::new(FakeTd { requests:Arc::new(Mutex::new(Vec::new())),updates,early:Mutex::new(None),fail:Mutex::new(None) });
        let media=Arc::new(Mutex::new(FakeState::default()));
        let manager=CallManager::spawn(td.clone(),dir.clone(),Box::new(FakeMedia(media.clone())),p2p,None).unwrap();
        Self { manager:Some(manager),td,media,dir }
    }
    fn manager(&self) -> &CallManager { self.manager.as_ref().unwrap() }
    fn requests(&self, kind: &str) -> Vec<Value> { self.td.requests.lock().unwrap().iter().filter(|r|r["@type"]==kind).cloned().collect() }
    fn ops(&self) -> Vec<MediaOp> { self.media.lock().unwrap().ops.clone() }
    async fn incoming(&self, id:i64,user:i64,video:bool) -> Value {
        self.manager().handle_update(pending(id,user,false,video)).await.unwrap();
        let all=self.manager().calls().await.unwrap();
        all["calls"].as_array().unwrap().iter().find(|c|c["user_id"]==user).unwrap().clone()
    }
    async fn outgoing(&self, video:bool) -> Value { self.manager().call_out(json!({"user_id":42,"video":video})).await.unwrap() }
    async fn ready(&self,id:i64,user:i64,outgoing:bool,versions:&[&str]) {
        self.manager().handle_update(ready(id,user,outgoing,versions)).await.unwrap();
    }
    async fn info(&self) -> Value { self.manager().calls().await.unwrap()["calls"][0].clone() }
    async fn emit(&self,event:MediaEvent) {
        self.media.lock().unwrap().callback.as_ref().unwrap().send(Command::Media(event)).unwrap();
        self.manager().calls().await.unwrap(); // FIFO trên cùng command channel.
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Đợi thread đóng media trước khi dọn dữ liệu của riêng test.
        let (tx,rx)=std::sync::mpsc::channel();
        let mut sent = false;
        if let Some(manager)=self.manager.take() {
            manager.handle.shutting_down.store(true, Ordering::Release);
            sent = manager.handle.sender.send(Command::Shutdown(Some(tx), Instant::now()+Duration::from_secs(3))).is_ok();
            let (callback, frames, audio) = {
                let mut s = self.media.lock().unwrap();
                (s.callback.take(), s.frames.take(), s.audio.take())
            };
            drop((callback, frames, audio)); drop(manager);
        }
        if sent { let _ = rx.recv_timeout(Duration::from_secs(3)); }
        for name in ["live_settings.json","live_settings.json.tmp","ha_devices.json"] { let _=fs::remove_file(self.dir.join(name)); }
        if let Ok(files)=fs::read_dir(self.dir.join("live")) { for file in files.flatten() { let _=fs::remove_file(file.path()); } }
        let _=fs::remove_dir(self.dir.join("live")); let _=fs::remove_dir(&self.dir);
        if let Ok(files)=fs::read_dir(self.dir.join("calltrace")) { for file in files.flatten() { let _=fs::remove_file(file.path()); } }
        let _=fs::remove_dir(self.dir.join("calltrace")); let _=fs::remove_dir(&self.dir);
    }
}

fn pending(id:i64,user:i64,outgoing:bool,video:bool)->Value {
    json!({"@type":"updateCall","call":{"id":id,"user_id":user,"is_outgoing":outgoing,"is_video":video,"state":{"@type":"callStatePending","is_created":true,"is_received":!outgoing}}})
}

fn ready(id:i64,user:i64,outgoing:bool,versions:&[&str])->Value {
    json!({"@type":"updateCall","call":{"id":id,"user_id":user,"is_outgoing":outgoing,"state":{"@type":"callStateReady","emojis":["🚙","🍚"],"protocol":{"library_versions":versions},"encryption_key":STANDARD.encode([1u8;256]),"servers":[
        {"id":"1234567890123456789","ip_address":"149.154.1.1","ipv6_address":"::1","port":443,"type":{"@type":"callServerTypeTelegramReflector","peer_tag":STANDARD.encode([3u8;16]),"is_tcp":true}},
        {"id":"22","ip_address":"149.154.1.2","port":3478,"type":{"@type":"callServerTypeWebrtc","supports_turn":true,"supports_stun":true,"username":"u","password":"p"}}
    ],"allow_p2p":true,"custom_parameters":"{\"x\":1}"}}})
}

fn local_actor(f: &Fixture) -> (Actor, mpsc::UnboundedReceiver<Command>) {
    let media = FakeMedia(f.media.clone());
    let protocol = media.protocol().unwrap();
    let (sender,commands) = mpsc::unbounded_channel();
    let (events,_) = broadcast::channel(256);
    (Actor { td:f.td.clone(), media:Box::new(media), protocol, p2p:false, data_dir:f.dir.clone(), defaults:Devices::default(),
        calls:vec![], catalog:catalog::empty(), handled:HashSet::new(), handled_order:VecDeque::new(), events, sender, profile:(640,360,15), cast_port:None }, commands)
}

#[test]
fn mic_camera_commands_quote_sources_and_formats() {
    let evil="http://h/x'; $(id); #";
    let quoted=sources::quote(evil);
    assert!(mic_command(evil,"auto").contains(&format!("-i {quoted} -vn")));
    assert!(camera_command(evil,(640,360,15)).contains(&format!("-i {quoted} -an")));
    assert_eq!(quoted,"'http://h/x'\"'\"'; $(id); #'");
    assert!(mic_command("test","auto").contains("sine=frequency=440"));
    assert!(mic_command("silence","auto").contains("anullsrc"));
    assert!(!mic_command("silence","auto").contains("sine"));
    assert!(mic_command("http://r1/mic","s16le16k").contains("-f s16le -ar 16000 -ac 1 -i"));
    for url in ["rtsp://u:p@cam/s","rtsps://cam/s"] {
        assert!(camera_command(url,(640,360,15)).contains("-rtsp_transport tcp"));
        assert!(mic_command(url,"auto").contains("-rtsp_transport tcp"));
    }
    for local in ["/tmp/say.wav", "./local/clip.mp3"] {
        let cmd = mic_command(local, "auto");
        assert!(cmd.contains(" -re -i "), "{cmd}");
        assert!(cmd.contains(&format!("-i {} -vn", sources::quote(local))), "{cmd}");
        assert!(!cmd.contains("nobuffer") && !cmd.contains("probesize") && !cmd.contains("analyzeduration"), "{cmd}");
    }
    for cmd in [mic_command("/tmp/say.pcm", "s16le48k"), mic_command("/tmp/say.pcm", "auto")] {
        assert!(cmd.contains(" -re -f s16le -ar 48000 -ac 1 -i /tmp/say.pcm "), "{cmd}");
        assert!(!cmd.contains("nobuffer") && !cmd.contains("probesize") && !cmd.contains("analyzeduration") && !cmd.contains(" -vn "), "{cmd}");
    }
    assert!(camera_command("black",(640,360,15)).contains("color=c=black"));
    assert!(camera_command("test",(640,360,15)).contains("rawvideo -pix_fmt yuv420p pipe:1"));
    assert_eq!(video_profile(" Low "),(480,270,10));
    assert_eq!(video_profile("?"),(640,360,15));
}

#[test]
fn speaker_regex_and_fixed_ffmpeg_commands() {
    for good in [R1,DUPLEX] { assert!(valid_speaker_url(good)); }
    for bad in ["","ftp://h/api/r1_speaker/intercom/E?k=1","http://h/api/other/intercom/E?k=1","http://h/api/r1_speaker/intercom/E",
        "http://h/api/r1_speaker/intercom/E?k=1;id","http://h/api/r1_speaker/intercom/E?k=1 &x","http://h/api/r1_speaker/mic/E?k=1",
        "http://h x/api/r1_speaker/intercom/E?k=1","http://h/api/r1_speaker/intercom/E?k=$(id)","http://h/api/r1_speaker/intercom/../E?k=1",
        "http://h/api/r1_speaker/intercom/E?k=1\nfoo","http://h/api/r1_speaker/intercom/E?k=1\n"] {
        assert!(!valid_speaker_url(bad),"{bad:?}"); assert!(speaker_command(bad).is_err());
    }
    let a=speaker_command(R1).unwrap();
    assert!(a.ends_with(&sources::quote(R1)) && a.contains("pcm_alaw -f alaw") && a.contains("-method POST"));
    assert!(a.contains("-f s16le -ar 48000 -ac 1 -i pipe:0"));
    let d=speaker_command(DUPLEX).unwrap();
    assert!(d.contains("-ar 16000 -c:a pcm_s16le -f s16le") && !d.contains("pcm_alaw"));
    for domain in ["r1_speaker","dahua_talk"] { for kind in ["intercom","duplex"] {
        let url=format!("http://ha.local/api/{domain}/{kind}/E-1_x?k=k%20e%2Fy%3B%26");
        assert!(valid_speaker_url(&url));
    }}
}

#[test]
fn cast_only_accepts_internal_ipv4_or_lowercase_id() {
    for good in ["cast://192.168.1.50","cast://10.0.0.7","cast://192.168.1.60","cast://id/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"] {
        assert!(is_cast_spec(good) && valid_speaker_url(good));
    }
    for bad in ["cast://8.8.8.8","cast://127.0.0.1","cast://169.254.1.1","cast://0.0.0.0","cast://loa.local","cast://192.168.1.50:8009",
        "cast://id/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","cast://id/abc","http://192.168.1.50/x","cast://192.168.1.500","","cast://192.168.1.50/../x"] {
        assert!(!is_cast_spec(bad),"{bad}");
    }
}

#[test]
fn playback_audio_always_uses_microphone_never_speaker() {
    for (kind,cmd,src) in [("external","",MediaSource::External),("file","",MediaSource::File),("shell","",MediaSource::Shell),("push",R1,MediaSource::Shell)] {
        let desc=playback_description(kind,cmd,"data/live/in.pcm",true).unwrap();
        assert!(desc.speaker.is_none() && desc.screen.is_none());
        let audio=desc.microphone.unwrap();
        assert_eq!(audio.media_source,src); assert_eq!(audio.sample_rate,48_000); assert_eq!(audio.channel_count,1); assert!(!audio.keep_open);
        assert_eq!(desc.camera.unwrap().media_source,MediaSource::External);
        if kind=="push" { assert_eq!(audio.input,speaker_command(R1).unwrap()); }
    }
}

#[test]
fn versions_match_numerically_and_describe_families() {
    let ours=strings(&["8.0.0","9.0.0","12.0.0","13.0.0"]);
    for (peer,want) in [(vec!["8.0.0"],Some("8.0.0")),(vec!["2.4.4","8.0.0","13.0.0","9.0.0"],Some("13.0.0")),
        (vec!["12.0.0","9.0.0"],Some("12.0.0")),(vec!["10.0.0","9.0.0"],Some("9.0.0")),(vec!["2.4.4","5.0.0"],None),
        (vec![],None),(vec!["x.y","8.0.0"],Some("8.0.0"))] { assert_eq!(versions::best_match(&strings(&peer),&ours),want); }
    assert_eq!(versions::describe(&strings(&["8.0.0"]),&ours).1,"8.0.0 (báo hiệu V2 Legacy)");
    assert!(versions::describe(&strings(&["13.0.0","8.0.0"]),&ours).1.contains("V3 SCTP"));
    assert!(versions::describe(&strings(&["2.4.4","5.0.0"]),&ours).1.contains("cần cập nhật Telegram"));
    assert!(!versions::describe(&strings(&["99.0.0"]),&ours).1.contains("cập nhật"));
    assert_eq!(versions::describe(&[],&ours),(None,"bên kia không khai phiên bản".into()));
}

#[test]
fn p2p_flag_defaults_off() {
    assert!(!p2p_enabled(None));
    for yes in ["1","true","ON"," yes "] { assert!(p2p_enabled(Some(yes))); }
    for no in ["","0","no","false","unknown"] { assert!(!p2p_enabled(Some(no))); }
}

#[tokio::test]
async fn incoming_accept_ready_orders_media_and_maps_tdlib() {
    let f=Fixture::new(false);
    let mut events=f.manager().events();
    let c=f.incoming(11,9,true).await;
    assert_eq!(events.recv().await.unwrap()["kind"],"incoming");
    assert_eq!(c["state"],"ringing"); assert_eq!(c["outgoing"],false);
    f.manager().settings(json!({"mic":"http://r1/mic?k=SECRET","mic_format":"s16le16k","camera":"rtsp://cam/s","speaker":R1})).await.unwrap();
    f.manager().accept(json!({"id":c["id"]})).await.unwrap();
    assert!(f.ops().is_empty());
    let accept=&f.requests("acceptCall")[0];
    assert_eq!(accept["call_id"],11); assert_eq!(accept["protocol"]["udp_p2p"],false);
    assert_eq!(accept["protocol"]["library_versions"],json!(["8.0.0","9.0.0","12.0.0","13.0.0"]));
    f.ready(11,9,false,&["8.0.0"]).await;
    let ops=f.ops();
    assert_eq!(ops.iter().map(op_name).collect::<Vec<_>>(),["create","capture","playback","skip","connect"]);
    assert!(matches!(ops[0],MediaOp::Create(9)));
    if let MediaOp::Sources(9,StreamMode::Capture,desc)=&ops[1] {
        assert!(desc.microphone.as_ref().unwrap().input.contains("-f s16le -ar 16000 -ac 1")); assert!(desc.camera.is_some());
    } else { panic!("capture"); }
    if let MediaOp::Sources(9,StreamMode::Playback,desc)=&ops[2] { assert!(desc.speaker.is_none()); assert_eq!(desc.microphone.as_ref().unwrap().input,speaker_command(R1).unwrap()); } else { panic!("playback"); }
    assert!(matches!(&ops[3],MediaOp::Skip(9,key,false) if key==&vec![1u8;256]));
    if let MediaOp::Connect(9,r)=&ops[4] {
        assert!(!r.p2p); assert_eq!(r.versions,strings(&["8.0.0"])); assert_eq!(r.custom.as_deref(),Some("{\"x\":1}"));
        assert_eq!(r.servers[0].id,1234567890123456789); assert_eq!(r.servers[0].peer_tag,vec![3u8;16]); assert!(r.servers[0].tcp);
        assert!(r.servers[1].stun && r.servers[1].turn); assert_eq!(r.servers[1].username,"u"); assert_eq!(r.servers[1].password,"p");
    } else { panic!("connect"); }
    f.ready(11,9,false,&["8.0.0"]).await;
    assert_eq!(f.ops().len(),5);
    f.emit(MediaEvent::Connection(9,ConnectionState::Connected)).await;
    assert_eq!(f.info().await["state"],"active"); assert_eq!(events.recv().await.unwrap()["kind"],"active");
    let info=f.info().await.to_string(); assert!(!info.contains("SECRET") && !info.contains("ab-cd") && !info.contains("http://"));
}

#[tokio::test]
async fn p2p_applies_to_both_protocol_and_connect_and_peer_can_forbid_it() {
    for enabled in [false,true] { for allowed in [false,true] {
        let f=Fixture::new(enabled); f.outgoing(false).await;
        assert_eq!(f.requests("createCall")[0]["protocol"]["udp_p2p"],enabled);
        let mut r=ready(7,42,true,&["13.0.0"]); r["call"]["state"]["allow_p2p"]=json!(allowed);
        f.manager().handle_update(r).await.unwrap();
        assert!(matches!(f.ops().last().unwrap(),MediaOp::Connect(_,r) if r.p2p==(enabled && allowed)));
    }}
}

#[tokio::test]
async fn incompatible_versions_discard_and_end_without_media() {
    let f=Fixture::new(false); let c=f.incoming(9,5,false).await;
    f.manager().accept(json!({"id":c["id"]})).await.unwrap();
    f.ready(9,5,false,&["2.4.4","5.0.0"]).await;
    let info=f.info().await;
    assert_eq!(info["state"],"ended"); assert!(info["error"].as_str().unwrap().contains("không tương thích"));
    assert!(info["error"].as_str().unwrap().contains("cần cập nhật Telegram"));
    assert_eq!(f.requests("discardCall").len(),1); assert_eq!(f.requests("discardCall")[0]["call_id"],9); assert!(f.ops().is_empty());
}

#[tokio::test]
async fn missing_peer_versions_still_connects() {
    let f=Fixture::new(false); f.outgoing(false).await; f.ready(7,42,true,&[]).await;
    assert_eq!(f.info().await["state"],"connecting"); assert!(f.requests("discardCall").is_empty());
    assert!(f.ops().iter().any(|op|matches!(op,MediaOp::Connect(..))));
}

#[tokio::test]
async fn signaling_base64_round_trip_and_counters() {
    let f=Fixture::new(false); f.outgoing(false).await;
    f.manager().handle_update(json!({"@type":"updateNewCallSignalingData","call_id":7,"data":"AAH/"})).await.unwrap();
    assert!(f.ops().is_empty());
    f.ready(7,42,true,&["9.0.0"]).await;
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Signaling(42,bytes) if bytes==&vec![0,1,255]));
    f.emit(MediaEvent::Signaling(42,vec![1,2,255])).await;
    assert_eq!(f.requests("sendCallSignalingData")[0],json!({"@type":"sendCallSignalingData","call_id":7,"data":"AQL/"}));
    assert_eq!(f.info().await["sig"],json!({"tx_n":1,"tx_b":3,"rx_n":1,"rx_b":3}));
    assert_eq!(f.manager().handle_update(json!({"@type":"updateNewCallSignalingData","call_id":7,"data":"!!"})).await.unwrap_err().status,400);
}

#[tokio::test]
async fn early_outgoing_update_is_adopted_and_broadcast_subscription_works() {
    let f=Fixture::new(false);
    *f.td.early.lock().unwrap()=Some(pending(7,42,true,false));
    let call=f.outgoing(false).await;
    let mut events=f.manager().events();
    f.td.updates.send(pending(8,99,false,true)).unwrap();
    let event=tokio::time::timeout(Duration::from_secs(2),events.recv()).await.unwrap().unwrap();
    assert_eq!(event["kind"],"incoming"); assert_eq!(event["call"]["user_id"],99);
    let all=f.manager().calls().await.unwrap();
    assert_eq!(all["calls"].as_array().unwrap().len(),2); assert_eq!(all["calls"][0]["id"],call["id"]);
    f.ready(7,42,true,&["8.0.0"]).await;
    assert_eq!(f.ops().iter().filter(|o|matches!(o,MediaOp::Create(42))).count(),1);
}

#[tokio::test]
async fn hangup_is_idempotent_and_discards_with_video_duration_and_cleans_file() {
    let f=Fixture::new(false); let c=f.outgoing(true).await; f.ready(7,42,true,&["13.0.0"]).await;
    f.emit(MediaEvent::Connection(42,ConnectionState::Connected)).await;
    let path=f.dir.join("live").join(format!("{}_in.pcm",c["id"].as_str().unwrap()));
    fs::write(&path,vec![7u8;1000]).unwrap();
    fs::write(format!("{}.wav",path.to_string_lossy()),[1,2]).unwrap();
    let info=f.manager().hangup(json!({"id":c["id"]})).await.unwrap();
    f.manager().hangup(json!({"id":c["id"]})).await.unwrap();
    assert_eq!(info["state"],"ended"); assert_eq!(info["playback"]["bytes"],1000); assert!(!path.exists()); assert!(!PathBuf::from(format!("{}.wav",path.to_string_lossy())).exists());
    let discard=f.requests("discardCall"); assert_eq!(discard.len(),1); assert_eq!(discard[0]["is_video"],true); assert!(discard[0]["duration"].is_number());
    assert_eq!(f.ops().iter().filter(|op|matches!(op,MediaOp::Stop(42))).count(),1);
    let secs=info["seconds"].clone(); tokio::time::sleep(Duration::from_millis(120)).await; assert_eq!(f.info().await["seconds"],secs);
}

#[tokio::test]
async fn old_or_processed_incoming_calls_cannot_be_accepted_again() {
    let f=Fixture::new(false); let c=f.incoming(11,9,false).await;
    f.manager().accept(json!({"id":c["id"]})).await.unwrap();
    assert_eq!(f.manager().accept(json!({"id":c["id"]})).await.unwrap_err().status,404);
    f.manager().hangup(json!({"id":c["id"]})).await.unwrap();
    f.manager().handle_update(pending(11,9,false,false)).await.unwrap();
    assert_eq!(f.manager().accept(json!({"id":c["id"]})).await.unwrap_err().status,404);
    f.manager().handle_update(json!({"@type":"updateCall","call":{"id":77,"user_id":12,"is_outgoing":false,"state":{"@type":"callStateDiscarded"}}})).await.unwrap();
    f.manager().handle_update(pending(77,12,false,false)).await.unwrap();
    f.manager().handle_update(ready(78,13,false,&["8.0.0"])).await.unwrap();
    f.manager().handle_update(pending(78,13,false,false)).await.unwrap();
    assert_eq!(f.manager().calls().await.unwrap()["calls"].as_array().unwrap().len(),1);
    assert_eq!(f.requests("acceptCall").len(),1);
}

#[tokio::test]
async fn ready_on_another_device_ends_incoming_without_starting_media() {
    let f=Fixture::new(false); let c=f.incoming(11,9,false).await;
    f.ready(11,9,false,&["8.0.0"]).await;
    assert_eq!(f.info().await["state"],"ended"); assert!(f.ops().is_empty());
    assert!(f.manager().accept(json!({"id":c["id"]})).await.is_err());
}

#[tokio::test]
async fn exchanging_keys_on_another_device_and_queued_discard_prevent_accept() {
    for state in ["callStateExchangingKeys","callStateDiscarded"] {
        let f=Fixture::new(false); let c=f.incoming(11,9,false).await;
        f.td.updates.send(json!({"@type":"updateCall","call":{"id":11,"state":{"@type":state}}})).unwrap();
        assert_eq!(f.manager().accept(json!({"id":c["id"]})).await.unwrap_err().status,404);
        f.manager().handle_update(pending(11,9,false,false)).await.unwrap();
        assert_eq!(f.info().await["state"],"ended"); assert!(f.requests("acceptCall").is_empty()); assert!(f.ops().is_empty());
    }
}

#[tokio::test]
async fn settings_validate_mask_preserve_secrets_and_persist_0600() {
    use std::os::unix::fs::PermissionsExt;
    let f=Fixture::new(false);
    assert_eq!(f.manager().settings(Value::Null).await.unwrap(),Devices::default().json(true));
    assert_eq!(f.manager().settings(json!({"mic":"ftp://x"})).await.unwrap_err().status,400);
    assert_eq!(f.manager().settings(json!({"speaker":"http://evil/x"})).await.unwrap_err().status,400);
    f.manager().settings(json!({"mic":"http://r1/mic?k=SECRETMIC","camera":"rtsp://u:secret@cam/s","mic_format":"s16le16k","speaker":R1})).await.unwrap();
    let masked=f.manager().settings(Value::Null).await.unwrap();
    assert_eq!(masked["camera"],"rtsp://u:***@cam/s"); assert!(masked["speaker"].as_str().unwrap().ends_with("?k=***"));
    assert!(!masked.to_string().contains("SECRETMIC") && !masked.to_string().contains("ab-cd"));
    f.manager().settings(masked).await.unwrap();
    let path=f.dir.join("live_settings.json");
    let saved:Value=serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["camera"],"rtsp://u:secret@cam/s"); assert_eq!(saved["speaker"],R1); assert_eq!(fs::metadata(path).unwrap().permissions().mode()&0o777,0o600);
    let second=CallManager::spawn(f.td.clone(),f.dir.clone(),Box::new(FakeMedia(Arc::new(Mutex::new(FakeState::default())))),false,None).unwrap();
    assert_eq!(second.settings(Value::Null).await.unwrap()["mic_format"],"s16le16k");
    f.manager().settings(json!({"speaker":""})).await.unwrap(); assert_eq!(f.manager().settings(Value::Null).await.unwrap()["speaker"],"");
}

#[tokio::test]
async fn push_validates_url_and_cannot_take_cmd_override_or_leak_it() {
    let f=Fixture::new(false); let c=f.manager().call_out(json!({"user_id":42,"speaker":R1})).await.unwrap();
    assert_eq!(c["playback"]["kind"],"push");
    f.ready(7,42,true,&["8.0.0"]).await;
    f.manager().playback(json!({"id":c["id"],"kind":"push","url":DUPLEX,"cmd":"touch SHOULD_NOT_RUN"})).await.unwrap();
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Playback,d) if d.microphone.as_ref().unwrap().input==speaker_command(DUPLEX).unwrap()));
    assert_eq!(f.manager().playback(json!({"id":c["id"],"kind":"push","url":"http://evil/x"})).await.unwrap_err().status,400);
    assert_eq!(f.manager().call_out(json!({"user_id":99,"speaker":"http://evil/x"})).await.unwrap_err().status,400);
    let muted = f.manager().playback(json!({"id":c["id"],"kind":"external"})).await.unwrap();
    assert_eq!(muted["playback"]["kind"], "external");
    assert_eq!(muted["playback"]["speaker_configured"], true);
    let on = f.manager().playback(json!({"id":c["id"],"kind":"push"})).await.unwrap();
    assert_eq!(on["playback"]["kind"], "push");
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Playback,d) if d.microphone.as_ref().unwrap().input==speaker_command(DUPLEX).unwrap()));
    assert!(!f.info().await.to_string().contains("ab-cd"));
}

#[tokio::test]
async fn outgoing_overrides_use_defaults_for_empty_capture_but_can_clear_speaker() {
    let f = Fixture::new(false);
    f.manager().settings(json!({"mic":"silence","camera":"black","speaker":R1})).await.unwrap();
    let masked = f.manager().settings(Value::Null).await.unwrap()["speaker"].clone();
    assert_eq!(f.manager().call_out(json!({"user_id":42,"speaker":masked})).await.unwrap_err().status, 400);
    assert!(f.requests("createCall").is_empty());

    let info = f.manager().call_out(json!({"peer":42,"video":true,"mic":"","camera":"","speaker":""})).await.unwrap();
    assert_eq!(info["user_id"], 42);
    assert_eq!(info["mic_muted"], true);
    assert_eq!(info["playback"]["kind"], "file");
    assert_eq!(f.manager().media(json!({"id":info["id"],"mic_muted":false})).await.unwrap_err().status,400);
    f.ready(7, 42, true, &["8.0.0"]).await;
    assert!(matches!(&f.ops()[1], MediaOp::Sources(_, StreamMode::Capture, d)
        if d.camera.as_ref().unwrap().input.contains("color=c=black")));
}

#[tokio::test]
async fn cast_returns_explicit_501_before_tdlib_or_media_side_effects() {
    let f=Fixture::new(false);
    assert_eq!(f.manager().call_out(json!({"user_id":42,"speaker":"cast://192.168.1.50"})).await.unwrap_err().status,501);
    assert!(f.requests("createCall").is_empty());
    let c=f.incoming(11,9,false).await;
    f.manager().settings(json!({"speaker":"cast://id/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"})).await.unwrap();
    let error=f.manager().accept(json!({"id":c["id"]})).await.unwrap_err();
    assert_eq!(error.status,501); assert!(error.message.contains("cổng HTTP")); assert!(f.requests("acceptCall").is_empty());
}

// Hồi quy lớp «gộp hai tính năng cùng sửa một hàm»: tắt/bật mic (hoan3) phải đi qua cùng đường mô tả nguồn với mic trình duyệt (hoan4),
// nếu không mic "browser" bị đổi thành im lặng mà từng tính năng riêng vẫn đạt test.
#[tokio::test]
async fn browser_mic_survives_mute_unmute_and_only_accepts_pcm_while_selected() {
    use crate::contract::CallControl;
    let f=Fixture::new(false); let c=f.outgoing(false).await;
    let id=c["id"].as_str().unwrap().to_owned();
    f.ready(7,42,true,&["8.0.0"]).await;
    f.emit(MediaEvent::Connection(42,ConnectionState::Connected)).await;
    assert_eq!(f.info().await["state"],"active");
    let loopback=|f:&Fixture|matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Capture,d)
        if d.microphone.as_ref().unwrap().input.contains("http://127.0.0.1:") && !d.microphone.as_ref().unwrap().input.contains("anullsrc"));
    let silent=|f:&Fixture|matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Capture,d) if d.microphone.as_ref().unwrap().input.contains("anullsrc"));
    let info=f.manager().media(json!({"id":id,"mic":"browser"})).await.unwrap();
    assert_eq!(info["mic_muted"],false); assert!(loopback(&f),"mic browser phải nối vào nguồn loopback");
    f.manager().audio_in(&id,vec![0;640]).await.unwrap();
    let muted=f.manager().media(json!({"id":id,"mic_muted":true})).await.unwrap();
    assert_eq!(muted["mic_muted"],true); assert!(silent(&f),"tắt mic phải ra im lặng");
    assert_eq!(f.manager().audio_in(&id,vec![0;640]).await.unwrap_err().status,409,"đang tắt mic thì không nhận PCM từ trình duyệt");
    let unmuted=f.manager().media(json!({"id":id,"mic_muted":false})).await.unwrap();
    assert_eq!(unmuted["mic_muted"],false); assert!(loopback(&f),"bật lại mic phải nối lại loopback, không phải im lặng");
    f.manager().audio_in(&id,vec![0;640]).await.unwrap();
    f.manager().media(json!({"id":id,"mic":"silence"})).await.unwrap();
    assert_eq!(f.manager().audio_in(&id,vec![0;640]).await.unwrap_err().status,409);
}

#[tokio::test]
async fn capture_can_change_before_or_during_call_and_info_has_python_keys() {
    let f=Fixture::new(false); let c=f.outgoing(true).await;
    assert_eq!(f.manager().media(json!({"id":c["id"]})).await.unwrap_err().status,400);
    assert_eq!(f.manager().media(json!({"id":c["id"],"camera":"silence"})).await.unwrap_err().status,400);
    f.manager().media(json!({"id":c["id"],"mic":"silence","camera_off":true})).await.unwrap(); assert!(f.ops().is_empty());
    f.ready(7,42,true,&["8.0.0"]).await;
    assert!(matches!(&f.ops()[1],MediaOp::Sources(_,StreamMode::Capture,d) if d.camera.is_none() && d.microphone.as_ref().unwrap().input.contains("anullsrc")));
    let info=f.manager().media(json!({"id":c["id"],"mic":"http://192.0.2.10/mic?k=key","mic_format":"s16le16k","camera_off":false})).await.unwrap();
    assert_eq!(info["mic_muted"],false); assert_eq!(info["cam_off"],false);
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Capture,d) if d.camera.is_some()));
    let muted=f.manager().media(json!({"id":c["id"],"mic_muted":true})).await.unwrap();
    assert_eq!(muted["mic_muted"],true);
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Capture,d) if d.microphone.as_ref().unwrap().input.contains("anullsrc")));
    let unmuted=f.manager().media(json!({"id":c["id"],"mic_muted":false})).await.unwrap();
    assert_eq!(unmuted["mic_muted"],false);
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Capture,d) if d.microphone.as_ref().unwrap().input == mic_command("http://192.0.2.10/mic?k=key", "s16le16k")));
    let mut keys=info.as_object().unwrap().keys().map(String::as_str).collect::<Vec<_>>(); keys.sort();
    let mut expected=vec!["id","user_id","outgoing","video","state","emojis","error","seconds","peer_video","view","diag","sig","cpu_max","mic_muted","saying","cam_off","frames_in","frame_bytes","peak","playback","events"]; expected.sort(); assert_eq!(keys,expected);
}

#[tokio::test]
async fn mic_toggle_on_incoming_restores_source_chosen_when_accepting() {
    let f = Fixture::new(false);
    let call = f.incoming(11, 9, false).await;
    let source = "http://192.0.2.10/pcm";
    let manager = f.manager();
    manager.settings(json!({"mic":source,"mic_format":"s16le16k"})).await.unwrap();
    manager.accept(json!({"id":call["id"]})).await.unwrap();
    f.ready(11, 9, false, &["8.0.0"]).await;
    manager.media(json!({"id":call["id"],"mic_muted":true})).await.unwrap();
    let result = manager.media(json!({"id":call["id"],"mic_muted":false})).await.unwrap();
    assert_eq!(result["mic_muted"], false);
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Capture,d) if d.microphone.as_ref().unwrap().input == mic_command(source, "s16le16k")));
}

#[tokio::test]
async fn audio_frames_and_video_frames_have_separate_counters() {
    let f=Fixture::new(false); f.outgoing(false).await; f.ready(7,42,true,&["8.0.0"]).await;
    let manager = f.manager();
    let info = f.info().await;
    let id = info["id"].as_str().unwrap();
    assert_eq!(manager.snapshot("unknown").await.unwrap_err().status, 404);
    assert_eq!(manager.snapshot(id).await.unwrap_err().code, "NO_VIDEO");
    let mut frames = manager.video_frames(id).await.unwrap();
    let metadata=FrameData{absolute_capture_timestamp_ms:0,rotation:VideoRotation::VideoRotation0,width:8,height:4};
    f.emit(MediaEvent::Frames(42,StreamMode::Playback,StreamDevice::Microphone,vec![Frame{ssrc:0,data:vec![0,128,255,127],frame_data:metadata.clone()}])).await;
    f.emit(MediaEvent::Frames(42,StreamMode::Playback,StreamDevice::Camera,vec![Frame{ssrc:0,data:vec![0;48],frame_data:metadata}])).await;
    let info=f.info().await; assert_eq!(info["frames_in"],1); assert_eq!(info["frame_bytes"],4); assert_eq!(info["peak"],32768); assert_eq!(info["view"]["frames"],1); assert_eq!(info["view"]["size"],"8x4");
    f.emit(MediaEvent::PeerVideo(42,true)).await;
    assert_eq!(f.info().await["peer_video"],true);
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Sources(_,StreamMode::Playback,d) if d.camera.is_some()));
    let jpeg = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match manager.snapshot(id).await {
                Ok(jpeg) => break jpeg,
                Err(error) => assert_eq!(error.code, "NO_VIDEO"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert_eq!(jpeg.content_type, "image/jpeg");
    assert!(jpeg.bytes.starts_with(&[0xff, 0xd8]) && jpeg.bytes.ends_with(&[0xff, 0xd9]));
    frames.changed().await.unwrap();
    assert_eq!(frames.borrow_and_update().as_deref(), Some(jpeg.bytes.as_slice()));
    manager.hangup(json!({"id":id})).await.unwrap();
    assert_eq!(manager.snapshot(id).await.unwrap_err().status, 404);
    assert!(frames.has_changed().is_err());
    assert!(frames.borrow().is_none());
}

#[tokio::test]
async fn failures_end_cleanly_and_never_publish_backend_urls() {
    let f=Fixture::new(false); f.outgoing(false).await;
    f.media.lock().unwrap().fail=Some("connect");
    assert_eq!(f.manager().handle_update(ready(7,42,true,&["8.0.0"])).await.unwrap_err().status,502);
    let info=f.info().await; assert_eq!(info["state"],"ended"); assert!(!info.to_string().contains("ab-cd"));
    let id = info["id"].as_str().unwrap();
    let trace = fs::read_to_string(f.dir.join("calltrace").join(format!("{id}.log"))).unwrap();
    assert!(trace.contains("su-kien") && trace.contains("tong-ket") && trace.contains("danh-dau-loi"));
    assert!(f.dir.join("calltrace").join(format!("{id}.loi")).exists());
    assert!(matches!(f.ops().last().unwrap(),MediaOp::Stop(42))); assert_eq!(f.requests("discardCall").len(),1);
}

#[tokio::test]
async fn connection_failure_and_tdlib_discard_stop_once() {
    let f=Fixture::new(false); f.outgoing(false).await; f.ready(7,42,true,&["8.0.0"]).await;
    f.emit(MediaEvent::Connection(42,ConnectionState::Timeout)).await;
    f.manager().handle_update(json!({"@type":"updateCall","call":{"id":7,"state":{"@type":"callStateDiscarded"}}})).await.unwrap();
    assert_eq!(f.info().await["state"],"ended"); assert_eq!(f.ops().iter().filter(|o|matches!(o,MediaOp::Stop(_))).count(),1);
}

#[tokio::test]
async fn peer_resolution_busy_and_tdlib_failure() {
    let f=Fixture::new(false); let c=f.manager().call_out(json!({"peer":"@ben_bap"})).await.unwrap(); assert_eq!(c["user_id"],42);
    assert_eq!(f.requests("searchPublicChat")[0]["username"],"ben_bap");
    assert_eq!(f.manager().call_out(json!({"user_id":42})).await.unwrap_err().code,"CALL_BUSY");
    f.manager().hangup(json!({"id":c["id"]})).await.unwrap();
    *f.td.fail.lock().unwrap()=Some("createCall".into());
    let e=f.manager().call_out(json!({"peer":"+84 912 345 678"})).await.unwrap_err(); assert_eq!(e.status,502); assert!(!e.message.contains("ab-cd"));
    assert_eq!(f.requests("searchUserByPhoneNumber")[0]["phone_number"],"84912345678");
    assert_eq!(f.manager().call_out(json!({"peer":"+84abc"})).await.unwrap_err().status,400);
}

#[tokio::test]
async fn say_requires_active_call_and_valid_input_before_using_tts() {
    let f=Fixture::new(false); let c=f.outgoing(false).await;
    assert_eq!(f.manager().say(json!({"id":c["id"],"text":""})).await.unwrap_err().status,400);
    assert_eq!(f.manager().say(json!({"id":c["id"],"text":"xin chào"})).await.unwrap_err().status,404);
    f.ready(7,42,true,&["8.0.0"]).await; f.emit(MediaEvent::Connection(42,ConnectionState::Connected)).await;
    let invalid_tts = f.manager().say(json!({"id":c["id"],"text":"xin chào","rate":"fast"})).await.unwrap_err();
    assert_eq!(invalid_tts.status, 400);
    assert!(invalid_tts.message.contains("TTS_RATE"));
    assert_eq!(f.manager().say(json!({"id":c["id"],"audio_b64":"!!"})).await.unwrap_err().status,400);
    let encoded=wav(&[0,0,1,0]); assert_eq!(&encoded[..4],b"RIFF"); assert_eq!(u32::from_le_bytes(encoded[40..44].try_into().unwrap()),4);
}

#[tokio::test]
async fn speech_writes_pcm_sets_saying_then_restores_mic_unless_user_changed_it() {
    for change in [json!({"camera_off":true}), json!({"mic":"silence"})] {
        let f=Fixture::new(false); let (mut actor,mut commands)=local_actor(&f);
        fs::create_dir_all(f.dir.join("live")).unwrap();
        let devices=Devices{mic:"http://r1/mic?k=secret".into(),mic_format:"s16le16k".into(),..Devices::default()};
        let mut c=LiveCall::new(42,true,true,devices,&f.dir);
        c.state="active"; c.media_created=true; c.td_id=Some(7); c.active=Some(Instant::now());
        actor.calls.push(c);
        assert_eq!(actor.calls[0].info()["saying"], json!(false));
        let pcm = vec![7u8; 9600];
        let reply=actor.start_speech(0,&pcm).await.unwrap(); assert_eq!(reply,json!({"seconds":0.1}));
        assert_eq!(actor.calls[0].info()["saying"], json!(true));
        assert_eq!(actor.calls[0].devices.mic_format, "s16le48k");
        let path=PathBuf::from(&actor.calls[0].devices.mic);
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("pcm"));
        assert_eq!(fs::read(&path).unwrap(), pcm);
        assert!(matches!(f.ops().last().unwrap(), MediaOp::Sources(_, StreamMode::Capture, d)
            if d.microphone.as_ref().unwrap().input.contains("-re -f s16le -ar 48000 -ac 1 -i")
            && !d.microphone.as_ref().unwrap().input.contains("probesize")));
        let mut body = change;
        body["id"] = json!(actor.calls[0].id);
        actor.set_media(body.clone()).await.unwrap();
        let restore=tokio::time::timeout(Duration::from_secs(3),commands.recv()).await.unwrap().unwrap();
        if let Command::RestoreMic(id,revision,mic,fmt,path)=restore { actor.restore_mic(&id,revision,mic,fmt,path).await; } else { panic!("restore"); }
        assert!(!path.exists()); assert!(actor.calls[0].speech_revision.is_none());
        assert_eq!(actor.calls[0].info()["saying"], json!(false));
        if body["mic"] == "silence" { assert_eq!(actor.calls[0].devices.mic,"silence"); }
        else {
            assert_eq!(actor.calls[0].devices.mic,"http://r1/mic?k=secret");
            assert_eq!(actor.calls[0].devices.mic_format,"s16le16k");
            assert!(matches!(f.ops().last().unwrap(), MediaOp::Sources(_,StreamMode::Capture,d) if d.camera.is_none()));
        }
        actor.end(0,"test xong").await;
    }
}

#[tokio::test]
async fn hangup_reports_elapsed_seconds_not_time_since_ringing() {
    let f=Fixture::new(false); let (mut actor,_)=local_actor(&f);
    let mut c=LiveCall::new(42,true,true,Devices::default(),&f.dir); c.td_id=Some(7); c.active=Some(Instant::now()-Duration::from_secs(42));
    actor.calls.push(c); actor.discard(0).await.unwrap();
    assert_eq!(f.requests("discardCall")[0]["duration"],42); assert_eq!(f.requests("discardCall")[0]["is_video"],true);
    actor.calls[0].active=None; actor.discard(0).await.unwrap(); assert_eq!(f.requests("discardCall")[1]["duration"],0);
}

#[tokio::test]
async fn native_binding_protocol_and_callback_registration_smoke_test() {
    let f=Fixture::new(false);
    let protocol=NTgCalls::get_protocol().unwrap(); assert!(!protocol.library_versions.is_empty());
    let manager=CallManager::new(f.td.clone(),&f.dir).unwrap();
    assert_eq!(manager.calls().await.unwrap(),json!({"calls":[]}));
    manager.shutdown(Duration::from_secs(3)).await.unwrap();
}

#[tokio::test]
async fn browser_mic_reaches_capture_and_ends_with_call() {
    let f=Fixture::new(false);
    let c=f.manager().call_out(json!({"user_id":42,"mic":"browser"})).await.unwrap();
    let id=c["id"].as_str().unwrap();
    assert_eq!(f.manager().audio_in(id,vec![0;3200]).await.unwrap_err().status,409);
    f.ready(7,42,true,&["8.0.0"]).await;
    f.emit(MediaEvent::Connection(42,ConnectionState::Connected)).await;
    let command=f.ops().iter().find_map(|op|match op { MediaOp::Sources(_,StreamMode::Capture,d)=>Some(d.microphone.as_ref().unwrap().input.clone()),_=>None }).unwrap();
    let url=command.split_whitespace().find(|s|s.starts_with("http://127.0.0.1:")).unwrap();
    let pcm=vec![7;3200];
    assert_eq!(f.manager().audio_in(id,pcm.clone()).await.unwrap(),json!({"ok":true,"bytes":3200}));
    let mut response=reqwest::get(url).await.unwrap();
    let mut received=Vec::new();
    while received.len()<pcm.len() { received.extend_from_slice(&response.chunk().await.unwrap().unwrap()); }
    assert_eq!(&received[..pcm.len()],pcm.as_slice());
    drop(response);
    assert_eq!(f.manager().audio_in(id,vec![1]).await.unwrap_err().status,400);
    assert_eq!(f.manager().audio_in(id,vec![0;65_538]).await.unwrap_err().status,413);
    f.manager().hangup(json!({"id":id})).await.unwrap();
    assert_eq!(f.manager().audio_in(id,vec![0;2]).await.unwrap_err().status,404);
    assert!(reqwest::get(url).await.is_err());
    let trace=fs::read_to_string(f.dir.join("calltrace").join(format!("{id}.log"))).unwrap();
    assert!(!trace.contains(url) && !trace.contains("127.0.0.1"));
    let other=f.manager().call_out(json!({"user_id":43,"mic":"silence"})).await.unwrap();
    assert_eq!(f.manager().audio_in(other["id"].as_str().unwrap(),vec![0;2]).await.unwrap_err().status,409);
}

#[tokio::test]
async fn browser_audio_uses_decoded_frames_and_never_logs_tokens() {
    let f=Fixture::new(false);
    f.manager().settings(json!({"speaker":"browser"})).await.unwrap();
    let c=f.outgoing(false).await;let id=c["id"].as_str().unwrap();
    assert_eq!(c["playback"]["kind"],"browser");
    let response=f.manager().media(json!({"id":id,"action":"audio_out"})).await.unwrap();
    let url=response["audio_out"]["url"].as_str().unwrap();
    let token=url.split("?t=").nth(1).unwrap();
    assert_eq!(token.len(),32);
    assert_eq!(f.manager().audio_out(id,"wrong").await.err().unwrap().status,403);
    let mut stream=f.manager().audio_out(id,token).await.unwrap();
    f.ready(7,42,true,&["8.0.0"]).await;
    assert!(matches!(&f.ops()[2],MediaOp::Sources(_,StreamMode::Playback,d) if d.microphone.as_ref().unwrap().media_source==MediaSource::External));
    let metadata=FrameData{absolute_capture_timestamp_ms:0,rotation:VideoRotation::VideoRotation0,width:0,height:0};
    f.emit(MediaEvent::Frames(42,StreamMode::Playback,StreamDevice::Microphone,vec![Frame{ssrc:0,data:vec![1,2,3,4],frame_data:metadata}])).await;
    assert_eq!(stream.audio.recv().await.unwrap(),[1,2,3,4]);
    f.manager().hangup(json!({"id":id})).await.unwrap();
    assert!(stream.stopped.changed().await.is_err());
    assert_eq!(f.manager().audio_out(id,token).await.err().unwrap().status,404);
    let trace=fs::read_to_string(f.dir.join("calltrace").join(format!("{id}.log"))).unwrap();
    assert!(!trace.contains(token));assert!(!f.info().await.to_string().contains(token));
}

#[tokio::test]
async fn audio_out_after_connect_switches_file_playback_to_external() {
    // AI mở luồng nghe SAU khi cuộc gọi đã nối, loa không cấu hình (kind="file" — đúng ca AI bật, connect_now bỏ loa):
    // phải đổi Playback sang External để khung tiếng tới on_frames, không thì tiếng chảy hết vào file, AI điếc.
    let f=Fixture::new(false);
    f.manager().settings(json!({"speaker":""})).await.unwrap();
    let c=f.outgoing(false).await;let id=c["id"].as_str().unwrap();
    assert_eq!(c["playback"]["kind"],"file");
    f.ready(7,42,true,&["8.0.0"]).await;
    let before=f.ops().len();
    let response=f.manager().media(json!({"id":id,"action":"audio_out"})).await.unwrap();
    let token=response["audio_out"]["url"].as_str().unwrap().split("?t=").nth(1).unwrap().to_owned();
    let ops=f.ops();
    assert_eq!(ops.len(),before+1);
    assert!(matches!(ops.last().unwrap(),MediaOp::Sources(_,StreamMode::Playback,d) if d.microphone.as_ref().unwrap().media_source==MediaSource::External));
    let mut stream=f.manager().audio_out(id,&token).await.unwrap();
    let metadata=FrameData{absolute_capture_timestamp_ms:0,rotation:VideoRotation::VideoRotation0,width:0,height:0};
    f.emit(MediaEvent::Frames(42,StreamMode::Playback,StreamDevice::Microphone,vec![Frame{ssrc:0,data:vec![5,6,7,8],frame_data:metadata}])).await;
    assert_eq!(stream.audio.recv().await.unwrap(),[5,6,7,8]);
    // mở lần hai không đổi nguồn lần nữa
    f.manager().media(json!({"id":id,"action":"audio_out"})).await.unwrap();
    assert_eq!(f.ops().len(),before+1);
}

#[tokio::test]
async fn browser_devices_auto_catalog_and_deferred_camera_validate_before_call() {
    let f=Fixture::new(false);
    assert_eq!(f.manager().settings(Value::Null).await.unwrap()["auto"],false);
    for settings in [json!({"mic":"silence","camera":"black","speaker":""}),json!({"mic":"browser","camera":"browser","speaker":"browser"})] {
        f.manager().settings(settings).await.unwrap();
        assert_eq!(f.manager().settings(Value::Null).await.unwrap()["auto"],false);
    }
    for (key,value) in [("mic","unknown"),("camera","unknown"),("speaker","unknown"),("mic_format","unknown")] {
        let mut body=json!({});body[key]=json!(value);
        assert_eq!(f.manager().settings(body.clone()).await.unwrap_err().status,400);
        body["user_id"]=json!(42);
        assert_eq!(f.manager().call_out(body).await.unwrap_err().status,400);
    }
    assert!(f.requests("createCall").is_empty());
    // Camera «browser» (webcam trình duyệt) đã được hỗ trợ: không còn chặn 501 BROWSER_VIDEO_UNAVAILABLE.
    assert!(Devices::default().check_camera(true,false).is_ok());
    for body in [json!({"mic":"http://camera.example/audio"}),json!({"mic":"silence","camera":"rtsp://camera.example/video"}),
        json!({"camera":"black","speaker":R1})] {
        f.manager().settings(body).await.unwrap();
        assert_eq!(f.manager().settings(Value::Null).await.unwrap()["auto"],true);
    }
    assert_eq!(f.manager().devices(Value::Null).await.unwrap()["catalog"],catalog::empty());
    let body=json!({"mic":[{"id":"sensor.mic","name":"Mic"},{"id":"sensor.mic","name":"Trùng"}],"speaker":[],"camera":[]});
    let saved=f.manager().devices(body).await.unwrap();
    assert_eq!(saved["auto"],true);assert_eq!(saved["settings"]["auto"],true);
    assert_eq!(saved["catalog"]["mic"].as_array().unwrap().len(),1);
    let second=CallManager::spawn(f.td.clone(),f.dir.clone(),Box::new(FakeMedia(Arc::new(Mutex::new(FakeState::default())))),false,None).unwrap();
    assert_eq!(second.devices(Value::Null).await.unwrap()["catalog"],saved["catalog"]);
    second.shutdown(Duration::from_secs(3)).await.unwrap();
}

#[tokio::test]
async fn shutdown_discards_active_call_once_and_rejects_new_requests() {
    let f=Fixture::new(false); f.outgoing(false).await;
    f.manager().shutdown(Duration::from_secs(3)).await.unwrap();
    assert_eq!(f.requests("discardCall").len(),1);
    assert_eq!(f.manager().calls().await.unwrap_err().status,503);
    f.manager().shutdown(Duration::from_secs(3)).await.unwrap();
    assert_eq!(f.requests("discardCall").len(),1);
}

#[test]
fn history_pruning_never_prunes_live_calls_or_forgets_handled_ids() {
    // Kiểm tra qua actor thuần, không cần thread hay NTgCalls thật.
    let (updates,_)=broadcast::channel(1); let (sender,_)=mpsc::unbounded_channel(); let (events,_)=broadcast::channel(1);
    let media=FakeMedia(Arc::new(Mutex::new(FakeState::default()))); let protocol=media.protocol().unwrap();
    let mut actor=Actor {td:Arc::new(FakeTd{requests:Arc::new(Mutex::new(Vec::new())),updates,early:Mutex::new(None),fail:Mutex::new(None)}),media:Box::new(media),protocol,p2p:false,data_dir:"data".into(),defaults:Devices::default(),catalog:catalog::empty(),calls:vec![],handled:HashSet::from([99]),handled_order:VecDeque::from([99]),events,sender,profile:(640,360,15),cast_port:None};
    for i in 0..27 { let mut c=LiveCall::new(100+i,true,false,Devices::default(),Path::new("data")); c.state="ended"; c.ended=Some(Instant::now()); actor.calls.push(c); }
    actor.calls.push(LiveCall::new(1,true,false,Devices::default(),Path::new("data")));
    actor.prune(); assert_eq!(actor.calls.len(),21); assert_eq!(actor.calls[0].user_id,107); assert_eq!(actor.calls.last().unwrap().state,"dialing"); assert!(actor.handled.contains(&99));
}


#[test]
fn nguon_truc_tiep_co_co_giam_tre_dung_cho() {
    // Lỗi đo được 21/11: thiếu cờ ⇒ mic/camera trễ ≈ 3,2 s. Cờ PHẢI đứng trước `-i`; nguồn tổng hợp (im lặng/thử) không đổi.
    let live = [
        mic_command("rtsp://192.0.2.1/mic", "auto"),
        mic_command("http://192.0.2.1/mic?k=x", "s16le16k"),
        mic_command("http://192.0.2.1/a.wav", "auto"),
        camera_command("rtsp://192.0.2.1/v", (640, 360, 15)),
        camera_command("http://192.0.2.1/v.mp4", (640, 360, 15)),
    ];
    for cmd in &live {
        assert!(cmd.contains("-fflags nobuffer") && cmd.contains("-flags low_delay") && cmd.contains("-analyzeduration 500000") && cmd.contains("-probesize 65536"), "{cmd}");
        assert!(cmd.find("-fflags").unwrap() < cmd.find(" -i ").unwrap(), "cờ giảm trễ phải đứng TRƯỚC -i: {cmd}");
    }
    assert!(!mic_command("silence", "auto").contains("nobuffer") && !mic_command("test", "auto").contains("nobuffer"));
    assert!(!camera_command("black", (640, 360, 15)).contains("nobuffer"));
}

#[test]
fn on_frames_separates_audio_and_video_channels_so_full_video_never_drops_audio() {
    let (frame_tx, mut frame_rx) = mpsc::channel(3);
    let (audio_tx, mut audio_rx) = mpsc::channel(64);
    let meta = FrameData { absolute_capture_timestamp_ms: 0, rotation: VideoRotation::VideoRotation0, width: 8, height: 4 };
    let mk = |bytes: Vec<u8>| vec![Frame { ssrc: 0, data: bytes, frame_data: meta.clone() }];

    // Capture mode bị lọc ở cả hai loại.
    media::dispatch_playback_frames(42, StreamMode::Capture, StreamDevice::Camera, mk(vec![0; 48]), &frame_tx, &audio_tx);
    media::dispatch_playback_frames(42, StreamMode::Capture, StreamDevice::Microphone, mk(vec![1, 2]), &frame_tx, &audio_tx);
    assert!(frame_rx.try_recv().is_err());
    assert!(audio_rx.try_recv().is_err());

    // Bơm đầy kênh hình (dung lượng 3) + 1 khung hình tràn (Screen cũng đi vào kênh hình).
    for _ in 0..3 {
        media::dispatch_playback_frames(42, StreamMode::Playback, StreamDevice::Camera, mk(vec![0; 48]), &frame_tx, &audio_tx);
    }
    media::dispatch_playback_frames(42, StreamMode::Playback, StreamDevice::Screen, mk(vec![1; 48]), &frame_tx, &audio_tx);

    // Kênh hình đang đầy cứng, nhưng khung tiếng vẫn đi vào kênh audio riêng mà không rớt.
    for i in 0..10u8 {
        media::dispatch_playback_frames(42, StreamMode::Playback, StreamDevice::Microphone, mk(vec![i, 0]), &frame_tx, &audio_tx);
    }

    let mut got_audio = 0;
    while let Ok(MediaEvent::Frames(42, StreamMode::Playback, StreamDevice::Microphone, _)) = audio_rx.try_recv() {
        got_audio += 1;
    }
    assert_eq!(got_audio, 10);
    assert!(audio_rx.try_recv().is_err());

    let mut got_video = 0;
    while let Ok(MediaEvent::Frames(42, StreamMode::Playback, StreamDevice::Camera | StreamDevice::Screen, _)) = frame_rx.try_recv() {
        got_video += 1;
    }
    assert_eq!(got_video, 3);
    assert!(frame_rx.try_recv().is_err());
}

#[tokio::test]
async fn actor_run_delivers_audio_and_video_from_separate_channels() {
    let f = Fixture::new(false);
    f.outgoing(false).await;
    f.ready(7, 42, true, &["8.0.0"]).await;
    let (frame_tx, audio_tx) = {
        let s = f.media.lock().unwrap();
        (s.frames.clone().unwrap(), s.audio.clone().unwrap())
    };
    let meta = FrameData { absolute_capture_timestamp_ms: 0, rotation: VideoRotation::VideoRotation0, width: 8, height: 4 };
    assert!(audio_tx.try_send(MediaEvent::Frames(42, StreamMode::Playback, StreamDevice::Microphone, vec![Frame { ssrc: 0, data: vec![0, 128, 255, 127], frame_data: meta.clone() }])).is_ok());
    assert!(frame_tx.try_send(MediaEvent::Frames(42, StreamMode::Playback, StreamDevice::Camera, vec![Frame { ssrc: 0, data: vec![0; 48], frame_data: meta }])).is_ok());
    let info = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let info = f.info().await;
            if info["frames_in"] == 1 && info["view"]["frames"] == 1 { break info; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert_eq!(info["frame_bytes"], 4);
    assert_eq!(info["peak"], 32768);
    assert_eq!(info["view"]["size"], "8x4");
}

#[test]
fn mark_handled_caps_history_at_256() {
    let (updates, _) = broadcast::channel(1);
    let (sender, _) = mpsc::unbounded_channel();
    let (events, _) = broadcast::channel(1);
    let media = FakeMedia(Arc::new(Mutex::new(FakeState::default())));
    let protocol = media.protocol().unwrap();
    let mut actor = Actor {
        td: Arc::new(FakeTd {
            requests: Arc::new(Mutex::new(Vec::new())),
            updates,
            early: Mutex::new(None),
            fail: Mutex::new(None),
        }),
        media: Box::new(media),
        protocol,
        p2p: false,
        data_dir: "data".into(),
        defaults: Devices::default(),
        catalog: catalog::empty(),
        calls: vec![],
        handled: HashSet::new(),
        handled_order: VecDeque::new(),
        events,
        sender,
        profile: (640, 360, 15),
        cast_port: None,
    };
    for id in 0..300 {
        actor.mark_handled(id);
    }
    assert_eq!(actor.handled.len(), 256);
    assert_eq!(actor.handled_order.len(), 256);
    assert!(!actor.handled.contains(&0));
    assert!(!actor.handled.contains(&43));
    assert!(actor.handled.contains(&44));
    assert!(actor.handled.contains(&299));
}

#[tokio::test]
async fn actor_end_cleans_up_call_live_files() {
    let temp_dir = std::env::temp_dir().join(format!("tele_test_clean_{}", std::process::id()));
    let live_dir = temp_dir.join("live");
    let _ = fs::create_dir_all(&live_dir);
    let (updates, _) = broadcast::channel(1);
    let (sender, _) = mpsc::unbounded_channel();
    let (events, _) = broadcast::channel(1);
    let media = FakeMedia(Arc::new(Mutex::new(FakeState::default())));
    let protocol = media.protocol().unwrap();
    let mut actor = Actor {
        td: Arc::new(FakeTd {
            requests: Arc::new(Mutex::new(Vec::new())),
            updates,
            early: Mutex::new(None),
            fail: Mutex::new(None),
        }),
        media: Box::new(media),
        protocol,
        p2p: false,
        data_dir: temp_dir.clone(),
        defaults: Devices::default(),
        catalog: catalog::empty(),
        calls: vec![],
        handled: HashSet::new(),
        handled_order: VecDeque::new(),
        events,
        sender,
        profile: (640, 360, 15),
        cast_port: None,
    };
    let c = LiveCall::new(101, true, false, Devices::default(), &temp_dir);
    let cid = c.id.clone();
    actor.calls.push(c);

    let f1 = live_dir.join(format!("{cid}_in.pcm"));
    let f2 = live_dir.join(format!("{cid}_say_1.pcm"));
    let f_other = live_dir.join("other_say_1.pcm");
    fs::write(&f1, b"in").unwrap();
    fs::write(&f2, b"say").unwrap();
    fs::write(&f_other, b"other").unwrap();

    actor.end(0, "test finished").await;

    assert!(!f1.exists());
    assert!(!f2.exists());
    assert!(f_other.exists());
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn shell_playback_rejects_custom_cmd() {
    assert_eq!(playback_description("shell", "touch /tmp/pwn", "data/live/in.pcm", false).unwrap_err().status, 400);
    assert!(playback_description("shell", "", "data/live/in.pcm", false).is_ok());

    let f = Fixture::new(false);
    let c = f.outgoing(false).await;
    let id = c["id"].as_str().unwrap();
    assert_eq!(f.manager().playback(json!({"id": id, "kind": "shell", "cmd": "id"})).await.unwrap_err().status, 400);
    f.ready(7, 42, true, &["8.0.0"]).await;
    assert_eq!(f.manager().playback(json!({"id": id, "kind": "shell", "cmd": "id"})).await.unwrap_err().status, 400);
    let ok = f.manager().playback(json!({"id": id, "kind": "shell"})).await.unwrap();
    assert_eq!(ok["playback"]["kind"], "shell");
}

#[test]
fn source_rejects_local_paths_crlf_and_link_local_ssrf() {
    for bad in [
        "/etc/passwd",
        "./local/secret.wav",
        "http://example.com/a\r\nb",
        "http://169.254.169.254/latest/meta-data/",
        "http://[::ffff:169.254.169.254]/audio",
        "http://0.0.0.0/audio",
        "http://[::]/audio",
        "http://[fe80::1]/audio",
    ] {
        assert_eq!(sources::source(&json!(bad), "mic").unwrap_err().status, 400, "{bad:?}");
        assert_eq!(sources::source(&json!(bad), "camera").unwrap_err().status, 400, "{bad:?}");
    }
    for bad_spk in [
        "http://169.254.169.254/api/r1_speaker/intercom/E?k=1",
        "http://[::ffff:169.254.169.254]/api/r1_speaker/intercom/E?k=1",
        "http://0.0.0.0/api/r1_speaker/intercom/E?k=1",
        "http://[fe80::1]/api/dahua_talk/duplex/E?k=1",
    ] {
        assert!(!valid_speaker_url(bad_spk), "{bad_spk}");
        assert_eq!(speaker_command(bad_spk).unwrap_err().status, 400, "{bad_spk}");
        assert_eq!(sources::speaker_args(bad_spk).unwrap_err().status, 400, "{bad_spk}");
    }

    assert!(mic_command("rtsp://cam/s", "auto").contains("-protocol_whitelist rtsp,tcp,rtp,pipe "));
    assert!(mic_command("rtsps://cam/s", "auto").contains("-protocol_whitelist rtsps,tls,tcp,rtp,pipe "));
    assert!(mic_command("http://cam/s", "auto").contains("-protocol_whitelist http,tcp,pipe "));
    assert!(mic_command("https://cam/s", "auto").contains("-protocol_whitelist https,tls,tcp,pipe "));
    assert!(mic_command("rtmp://cam/s", "auto").contains("-protocol_whitelist rtmp,tcp,pipe "));
    assert!(mic_command("/tmp/say.pcm", "s16le48k").contains("-protocol_whitelist file,pipe "));
    assert!(camera_command("rtsp://cam/v", (640, 360, 15)).contains("-protocol_whitelist rtsp,tcp,rtp,pipe "));
    assert!(camera_command("https://cam/v", (640, 360, 15)).contains("-protocol_whitelist https,tls,tcp,pipe "));
    assert!(speaker_command(R1).unwrap().contains("-protocol_whitelist http,tcp,pipe "));
    assert!(speaker_command(DUPLEX).unwrap().contains("-protocol_whitelist https,tls,tcp,pipe "));
    assert!(sources::speaker_args(R1).unwrap().windows(2).any(|w| w[0] == "-protocol_whitelist" && w[1] == "http,tcp,pipe"));
    assert!(sources::speaker_args(DUPLEX).unwrap().windows(2).any(|w| w[0] == "-protocol_whitelist" && w[1] == "https,tls,tcp,pipe"));
}

#[tokio::test]
async fn failed_capture_switch_preserves_live_browser_mic() {
    let f = Fixture::new(false);
    let c = f.manager().call_out(json!({"user_id": 42, "mic": "browser"})).await.unwrap();
    let id = c["id"].as_str().unwrap();
    f.ready(7, 42, true, &["8.0.0"]).await;
    f.emit(MediaEvent::Connection(42, ConnectionState::Connected)).await;
    f.manager().audio_in(id, vec![0; 640]).await.unwrap();

    f.media.lock().unwrap().fail = Some("capture");
    assert_eq!(f.manager().media(json!({"id": id, "mic": "silence"})).await.unwrap_err().status, 502);
    f.media.lock().unwrap().fail = None;

    assert_eq!(f.info().await["mic_muted"], false);
    f.manager().audio_in(id, vec![0; 640]).await.unwrap();
}

#[tokio::test]
async fn audio_out_retries_after_playback_failure_and_validates_playback_kind() {
    let f = Fixture::new(false);
    let c = f.outgoing(false).await;
    let id = c["id"].as_str().unwrap();
    f.ready(7, 42, true, &["8.0.0"]).await;

    f.media.lock().unwrap().fail = Some("playback");
    assert_eq!(f.manager().media(json!({"id": id, "action": "audio_out"})).await.unwrap_err().status, 502);
    f.media.lock().unwrap().fail = None;

    let res = f.manager().media(json!({"id": id, "action": "audio_out"})).await.unwrap();
    assert!(res["audio_out"]["url"].as_str().unwrap().contains("/api/live/audio/out/"));
    assert!(matches!(f.ops().last().unwrap(), MediaOp::Sources(_, StreamMode::Playback, d)
        if d.microphone.as_ref().unwrap().media_source == MediaSource::External));

    assert_eq!(f.manager().playback(json!({"id": id, "kind": "invalid"})).await.unwrap_err().status, 400);
    assert_eq!(f.manager().playback(json!({"id": id, "kind": "shell", "cmd": "id"})).await.unwrap_err().status, 400);
    assert_eq!(f.manager().playback(json!({"id": id, "kind": "push", "url": "http://evil/x"})).await.unwrap_err().status, 400);
}

#[test]
fn ready_parse_preserves_negative_signed_reflector_ids() {
    let val = json!({
        "encryption_key": STANDARD.encode([1u8; 32]),
        "protocol": {"library_versions": ["8.0.0"]},
        "servers": [
            {"id": -42i64, "ip_address": "149.154.1.1", "ipv6_address": "::1", "port": 443,
                "type": {"@type": "callServerTypeTelegramReflector", "peer_tag": STANDARD.encode([3u8; 16]), "is_tcp": true}},
            {"id": "-99", "ip_address": "149.154.1.2", "ipv6_address": "::1", "port": 443,
                "type": {"@type": "callServerTypeTelegramReflector", "peer_tag": STANDARD.encode([4u8; 16]), "is_tcp": false}},
            {"id": 12345u64, "ip_address": "149.154.1.3", "ipv6_address": "::1", "port": 3478,
                "type": {"@type": "callServerTypeWebrtc", "supports_turn": true, "supports_stun": true, "username": "u", "password": "p"}}
        ]
    });
    let parsed = Ready::parse(&val, false).unwrap();
    assert_eq!(parsed.servers[0].id, (-42i64) as u64);
    assert_eq!(parsed.servers[1].id, (-99i64) as u64);
    assert_eq!(parsed.servers[2].id, 12345u64);
}

#[tokio::test]
async fn interrupting_speech_clears_saying_and_caps_pending_signaling() {
    let f = Fixture::new(false);
    let (mut actor, _commands) = local_actor(&f);
    fs::create_dir_all(f.dir.join("live")).unwrap();
    let devices = Devices { mic: "http://r1/mic?k=secret".into(), mic_format: "s16le16k".into(), ..Devices::default() };
    let mut c = LiveCall::new(42, true, false, devices, &f.dir);
    let id = c.id.clone();
    c.state = "active";
    c.media_created = true;
    c.td_id = Some(7);
    c.active = Some(Instant::now());
    actor.calls.push(c);

    actor.start_speech(0, &vec![7u8; 9600]).await.unwrap();
    assert_eq!(actor.calls[0].info()["saying"], json!(true));

    actor.set_media(json!({"id": id, "mic_format": "auto"})).await.unwrap();
    assert_eq!(actor.calls[0].info()["saying"], json!(true));
    let rev = actor.calls[0].capture_revision;
    let say_path = PathBuf::from(&actor.calls[0].devices.mic);
    actor.restore_mic(&id, rev, "http://r1/mic?k=secret".into(), "s16le16k".into(), say_path).await;
    assert_eq!(actor.calls[0].devices.mic, "http://r1/mic?k=secret");
    assert_eq!(actor.calls[0].devices.mic_format, "auto");

    actor.start_speech(0, &vec![7u8; 9600]).await.unwrap();
    assert_eq!(actor.calls[0].info()["saying"], json!(true));
    actor.set_media(json!({"id": id, "mic": "silence"})).await.unwrap();
    assert_eq!(actor.calls[0].info()["saying"], json!(false));
    assert!(actor.calls[0].speech_revision.is_none());
    actor.end(0, "test xong").await;

    let mut pending_call = LiveCall::new(99, true, false, Devices::default(), &f.dir);
    pending_call.td_id = Some(88);
    actor.calls.push(pending_call);
    let idx = actor.calls.len() - 1;
    for _ in 0..(MAX_PENDING_SIGNALING + 20) {
        actor.update(json!({"@type": "updateNewCallSignalingData", "call_id": 88, "data": "AAH/"})).await.unwrap();
    }
    assert_eq!(actor.calls[idx].pending_signaling.len(), MAX_PENDING_SIGNALING);
}

#[tokio::test]
async fn catalog_anon_id_resolves_in_call_out_media_and_playback() {
    let f = Fixture::new(false);
    let saved = f.manager().devices(json!({
        "mic": [
            {"id": "http://192.0.2.10/mic1?k=SECRET1", "name": "Mic 1"},
            {"id": "http://192.0.2.10/mic2?k=SECRET2", "name": "Mic 2"}
        ],
        "speaker": [
            {"id": R1, "name": "Loa R1"},
            {"id": DUPLEX, "name": "Loa Duplex"}
        ],
        "camera": [
            {"id": "rtsp://192.0.2.10/cam1", "name": "Cam 1"}
        ]
    })).await.unwrap();
    let mic1_anon = saved["catalog"]["mic"][0]["id"].as_str().unwrap().to_owned();
    let mic2_anon = saved["catalog"]["mic"][1]["id"].as_str().unwrap().to_owned();
    let spk1_anon = saved["catalog"]["speaker"][0]["id"].as_str().unwrap().to_owned();
    let spk2_anon = saved["catalog"]["speaker"][1]["id"].as_str().unwrap().to_owned();
    let cam1_anon = saved["catalog"]["camera"][0]["id"].as_str().unwrap().to_owned();

    let c = f.manager().call_out(json!({
        "user_id": 42,
        "video": true,
        "mic": mic1_anon,
        "camera": cam1_anon,
        "speaker": spk1_anon
    })).await.unwrap();
    let id = c["id"].as_str().unwrap();
    f.ready(7, 42, true, &["8.0.0"]).await;

    let ops = f.ops();
    assert!(matches!(&ops[1], MediaOp::Sources(_, StreamMode::Capture, d)
        if d.microphone.as_ref().unwrap().input == mic_command("http://192.0.2.10/mic1?k=SECRET1", "auto")
        && d.camera.as_ref().unwrap().input == camera_command("rtsp://192.0.2.10/cam1", (640, 360, 15))));
    assert!(matches!(&ops[2], MediaOp::Sources(_, StreamMode::Playback, d)
        if d.microphone.as_ref().unwrap().input == speaker_command(R1).unwrap()));

    f.manager().media(json!({"id": id, "mic": mic2_anon})).await.unwrap();
    assert!(matches!(f.ops().last().unwrap(), MediaOp::Sources(_, StreamMode::Capture, d)
        if d.microphone.as_ref().unwrap().input == mic_command("http://192.0.2.10/mic2?k=SECRET2", "auto")));

    f.manager().playback(json!({"id": id, "kind": "push", "url": spk2_anon})).await.unwrap();
    assert!(matches!(f.ops().last().unwrap(), MediaOp::Sources(_, StreamMode::Playback, d)
        if d.microphone.as_ref().unwrap().input == speaker_command(DUPLEX).unwrap()));
}

#[test]
fn rx_rotation_does_not_overflow_on_large_multiple_of_90() {
    std::env::set_var("TELEGRAM_RX_ROTATE", "65520");
    let r1 = rx_rotation(VideoRotation::VideoRotation90);
    std::env::set_var("TELEGRAM_RX_ROTATE", "65430");
    let r2 = rx_rotation(VideoRotation::VideoRotation270);
    std::env::remove_var("TELEGRAM_RX_ROTATE");
    assert_eq!(r1, 90);
    assert_eq!(r2, 180);
}
