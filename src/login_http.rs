//! Đăng nhập HTTP đã xác thực; không ghi dữ liệu đăng nhập vào log.
use std::{collections::HashMap, sync::{Arc,Weak}, time::Duration};
use serde_json::{json,Value};
use tokio::{sync::Mutex,time::Instant};
use crate::{account_router::Accounts, contract::{GwError,TdError}, events::mask_peer, td::{TdClient,login}};

const TTL:Duration=Duration::from_secs(600);
const LOCK:Duration=Duration::from_secs(900);
struct Pending {
    phone:String, client:Arc<TdClient>, created:Instant, failures:u8,
    locked_until:Option<Instant>, stage:String, expired:bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_router::tests::Fixture;

    #[tokio::test]
    async fn abandoned_login_closes_and_locked_id_lasts_fifteen_minutes() {
        let f=Fixture::new("expiry").await;
        let id=f.login("+84900000007").await;
        let logins=f.hub.logins.get().unwrap();
        logins.pending.lock().await.get_mut(&id).unwrap().created=Instant::now()-TTL-Duration::from_secs(1);
        logins.expire().await;
        assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.0,400);
        assert!(f.api.sent.lock().unwrap().iter().any(|(id,r)|*id==1 && r["@type"]=="close"));
        let id=f.login("+84900000007").await;
        for _ in 0..3 {f.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":"bad"})).await;}
        {
            let mut pending=logins.pending.lock().await;
            let entry=pending.get_mut(&id).unwrap();
            entry.created=Instant::now()-TTL-Duration::from_secs(1);
            let left=entry.locked_until.unwrap()-Instant::now();
            assert!(left>Duration::from_secs(898) && left<=LOCK);
        }
        logins.expire().await;
        assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.0,429);
        assert!(f.api.sent.lock().unwrap().iter().any(|(id,r)|*id==2 && r["@type"]=="close"));
        logins.pending.lock().await.get_mut(&id).unwrap().locked_until=Some(Instant::now()-Duration::from_secs(1));
        logins.expire().await;
        assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.0,400);
        f.ready("+84900000007").await;
        f.finish().await;
    }
    #[tokio::test]
    async fn wrong_codes_lock_for_fifteen_minutes_and_expired_clients_close() {
        let f=Fixture::new("lock").await;
        let id=f.login("+84900000000").await;
        for (code,status) in [("bad",400),("11111",400),("11111",429)] {
            let response=f.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":code})).await;
            assert_eq!(response.0,status,"{response:?}");
            assert_eq!(response.1["error"]["code"],if status==429 {"LOGIN_LOCKED"}else{"BAD_REQUEST"});
        }
        assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.0,429);
        assert_eq!(f.send("POST","/ui/api/accounts",json!({"phone":"+84900000000"})).await.0,429);
        let logins=f.hub.logins.get().unwrap();
        {
            let mut pending=logins.pending.lock().await;
            let e=pending.get_mut(&id).unwrap();
            assert!(e.locked_until.unwrap()-Instant::now()>Duration::from_secs(898));
            e.created=Instant::now()-TTL-Duration::from_secs(1);
        }
        logins.expire().await;
        assert!(f.api.sent.lock().unwrap().iter().any(|(_,v)|v["@type"]=="close"));
        assert_eq!(f.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":"22222"})).await.0,429);
        {
            let mut pending=logins.pending.lock().await;
            pending.get_mut(&id).unwrap().locked_until=Some(Instant::now()-Duration::from_secs(1));
        }
        logins.expire().await;
        assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.0,400);
        let id=f.login("+84900000000").await;
        logins.pending.lock().await.get_mut(&id).unwrap().created=Instant::now()-TTL-Duration::from_secs(1);
        logins.expire().await;
        assert_eq!(f.send("GET",&format!("/ui/api/accounts/{id}"),Value::Null).await.0,400);
        f.finish().await;
    }

    #[tokio::test]
    async fn flood_wait_uses_server_seconds_without_counting_as_wrong_code() {
        let f=Fixture::new("flood").await;
        let response=f.send("POST","/ui/api/accounts",json!({"phone":"+84900000300"})).await;
        assert_eq!(response,(429,json!({"error":{"code":"FLOOD_WAIT","message":"Telegram yêu cầu chờ trước khi thử lại.","retry_after_seconds":321}})));
        let id=f.login("+84900000000").await;
        let response=f.send("POST",&format!("/ui/api/accounts/{id}/code"),json!({"code":"33333"})).await;
        assert_eq!(response.0,429);assert_eq!(response.1["error"]["retry_after_seconds"],123);
        assert_eq!(f.hub.logins.get().unwrap().pending.lock().await[&id].failures,0);
        f.finish().await;
    }
}

pub(crate) struct LoginHttp { hub:Weak<Accounts>, pending:Mutex<HashMap<String,Pending>>, operation:Mutex<()> }
fn bad()->GwError { GwError::bad_request("login_id không tồn tại hoặc đã hết hạn.") }
fn locked()->GwError { GwError::new(429,"LOGIN_LOCKED","Đăng nhập bị khóa tạm thời sau nhiều mã sai.") }
fn fail(entry:&mut Pending)->GwError {
    entry.failures+=1;
    if entry.failures>=3 { entry.locked_until=Some(Instant::now()+LOCK); locked() }
    else { GwError::bad_request("Mã không đúng.") }
}
fn error(e:TdError)->GwError {
    let mapped=GwError::from_tdlib(e);
    if mapped.code=="FLOOD_WAIT" { mapped }
    else { GwError::bad_request("Telegram từ chối yêu cầu đăng nhập.") }
}
impl LoginHttp {
    pub fn new(hub:Weak<Accounts>)->Self { Self{hub,pending:Mutex::new(HashMap::new()),operation:Mutex::new(())} }
    async fn sweep(&self,phone:Option<&str>) {
        let expired:Vec<_>={
            let mut pending=self.pending.lock().await;
            let mut closing=Vec::new();
            for entry in pending.values_mut() {
                if !entry.expired && Instant::now().duration_since(entry.created)>TTL && entry.locked_until.is_some_and(|t|t>Instant::now()) {
                    entry.expired=true;closing.push(entry.client.clone());
                }
            }
            let ids:Vec<_>=pending.iter().filter(|(_,e)|
                (Instant::now().duration_since(e.created)>TTL && !e.locked_until.is_some_and(|t|t>Instant::now()))
                || phone==Some(e.phone.as_str())).map(|(id,_)|id.clone()).collect();
            closing.extend(ids.into_iter().filter_map(|id|pending.remove(&id)).map(|e|e.client));closing
        };
        for client in expired { client.close(Duration::from_secs(10)).await; }
    }
    pub async fn shutdown(&self) {
        let entries:Vec<_>=self.pending.lock().await.drain().map(|(_,e)|e).collect();
        for e in entries { e.client.close(Duration::from_secs(10)).await; }
    }
    pub async fn expire(&self) {
        let _operation=self.operation.lock().await;
        self.sweep(None).await;
    }
    pub async fn handle(&self,method:&str,path:&str,body:Value)->Result<Value,GwError> {
        let _operation=self.operation.lock().await;
        self.sweep(None).await;
        let hub=self.hub.upgrade().ok_or_else(bad)?;
        let path=path.trim_start_matches("/ui/api/accounts").trim_start_matches('/');
        if method=="DELETE" {
            let phone=form_urlencoded::parse(format!("phone={}",path.replace('+',"%2B")).as_bytes()).next().map(|(_,v)|v.into_owned()).unwrap_or_default();
            self.sweep(Some(&phone)).await;
            hub.remove(&phone).await?;
            return Ok(json!({"ok":true}));
        }
        if method=="POST" && (path.is_empty()||path=="qr") {
            let phone=body["phone"].as_str().unwrap_or_default().trim();
            if phone.len()<6 { return Err(GwError::bad_request("Số điện thoại không hợp lệ.")); }
            if hub.contains(phone) { return Err(GwError::bad_request("Tài khoản này đã đăng nhập ở gateway. Xoá nó trước nếu muốn đăng nhập lại.")); }
            // Không thể bỏ khoá chống thử sai bằng cách tạo login_id mới cho cùng số.
            if self.pending.lock().await.values().any(|e|e.phone==phone && e.locked_until.is_some_and(|t|t>Instant::now())) { return Err(locked()); }
            self.sweep(Some(phone)).await;
            let session=hub.store.lock().unwrap().prepare(phone).map_err(|_|GwError::bad_request("Không tạo được phiên."))?;
            let client=hub.open_client(&session).map_err(|_|GwError::bad_request("Không mở được phiên TDLib."))?;
            let mut cfg=hub.config.clone(); cfg.phone=phone.into();
            let qr=path=="qr";
            if let Err(e)=login::begin_http(&client,&cfg,&session,qr).await {
                client.close(Duration::from_secs(10)).await;
                return Err(error(e));
            }
            let mut random=[0;16];
            rustls::crypto::ring::default_provider().secure_random.fill(&mut random).map_err(|_|bad())?;
            let id=format!("{:032x}",u128::from_ne_bytes(random));
            let link=client.authorization()["link"].as_str().unwrap_or_default().to_owned();
            if qr && link.is_empty() && client.auth_state()!="authorizationStateReady" {
                client.close(Duration::from_secs(10)).await;
                return Err(GwError::bad_request("Telegram chưa trả mã QR. Thử lại sau vài giây."));
            }
            self.pending.lock().await.insert(id.clone(),Pending{phone:phone.into(),client,created:Instant::now(),failures:0,
                locked_until:None,stage:if qr {"qr"}else{"code"}.into(),expired:false});
            return Ok(if qr { json!({"login_id":id,"qr_link":link}) } else { json!({"login_id":id}) });
        }
        let mut parts=path.split('/');
        let id=parts.next().unwrap_or_default();
        let action=parts.next().unwrap_or_default();
        let mut pending=self.pending.lock().await;
        let entry=pending.get_mut(id).ok_or_else(bad)?;
        if entry.locked_until.is_some_and(|t|t>Instant::now()) { return Err(locked()); }
        if method=="GET" {
            if entry.stage=="qr" {
                if entry.client.auth_state()=="authorizationStateWaitPassword" {entry.stage="password".into();}
                else if entry.client.auth_state()=="authorizationStateReady" {
                    let (phone,client)=(entry.phone.clone(),entry.client.clone());
                    hub.finish(&phone,client).await?; pending.remove(id);
                    return Ok(json!({"login_id":id,"stage":"done","phone":mask_peer(&phone),"qr_link":"","mock":false}));
                }
            }
            let link=if entry.stage=="qr" {entry.client.authorization()["link"].as_str().unwrap_or_default().to_owned()}else{String::new()};
            return Ok(json!({"login_id":id,"stage":entry.stage,"phone":mask_peer(&entry.phone),"qr_link":link}));
        }
        let stage=match action {
            "code"=>{
                let code=body["code"].as_str().unwrap_or_default();
                if !(5..=6).contains(&code.len())||!code.bytes().all(|b|b.is_ascii_digit()) {return Err(fail(entry));}
                login::verify_http(&entry.client,"checkAuthenticationCode","code",code).await
            }
            "password"=>{
                if entry.stage!="password" {return Err(GwError::bad_request("Phiên này không ở bước mật khẩu 2FA."));}
                let password=body["password"].as_str().unwrap_or_default();
                if password.is_empty() {return Err(fail(entry));}
                login::verify_http(&entry.client,"checkAuthenticationPassword","password",password).await
            }
            "qr-confirm"=>{
                if entry.stage!="qr" {return Err(GwError::bad_request("Phiên này không ở bước QR."));}
                if entry.client.auth_state()!="authorizationStateReady" {return Err(GwError::bad_request("QR chưa được quét hoặc chưa xác nhận."));}
                Ok("authorizationStateReady".into())
            }
            _=>return Err(bad()),
        };
        let stage=match stage {
            Ok(s)=>s,
            Err(e)=>{ let mapped=error(e); return Err(if mapped.code=="FLOOD_WAIT" {mapped}else{fail(entry)}); }
        };
        if stage=="authorizationStateWaitPassword" {
            entry.stage="password".into(); return Ok(json!({"ok":true,"needs_password":true,"stage":"password"}));
        }
        let (phone,client)=(entry.phone.clone(),entry.client.clone());
        hub.finish(&phone,client).await?;
        pending.remove(id);
        Ok(json!({"ok":true,"phone":phone,"status":"online","mock":false}))
    }
}
