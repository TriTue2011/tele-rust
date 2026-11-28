//! Persistent browser login for the web UI. Bearer tokens remain the machine API credential.

use std::{collections::{HashMap, VecDeque}, fs::{self, DirBuilder, File, OpenOptions}, io::Read,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf},
    num::NonZeroU32, sync::{Arc, Mutex, MutexGuard}, time::{SystemTime, UNIX_EPOCH}};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::{digest, pbkdf2, rand::{SecureRandom, SystemRandom}};
use rustix::fs::{flock, FlockOperation};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;

const ITERATIONS: u32 = 210_000;
const SALT_BYTES: usize = 16;
const HASH_BYTES: usize = 32;
const SESSION_BYTES: usize = 32;
const SESSION_SECONDS: u64 = 90 * 24 * 60 * 60;
const FAILURE_WINDOW: u64 = 15 * 60;
const MAX_FAILURES: usize = 5;
const MAX_KEYS_PER_IP: usize = 32;
const DUMMY_SALT: [u8; SALT_BYTES] = *b"tggw-dummy-salt!";

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

#[derive(Clone)]
struct Account {
    username: String,
    salt: Vec<u8>,
    password_hash: Vec<u8>,
}

#[derive(Clone)]
struct Session {
    hash: String,
    expires_at: u64,
    remember: bool,
    secure: bool,
}

#[derive(Clone, Default)]
struct Store {
    account: Option<Account>,
    sessions: Vec<Session>,
}

pub struct UiAuth {
    path: Option<PathBuf>,
    gateway_token: String,
    store: Mutex<Store>,
    failures: Mutex<HashMap<(String, String), VecDeque<u64>>>,
    hash_slots: Arc<tokio::sync::Semaphore>,
    clock: Clock,
    secure_cookie: bool,
}

#[derive(Clone, Debug)]
pub struct AuthError {
    pub status: u16,
    pub code: &'static str,
    pub message: &'static str,
    pub retry_after: Option<u64>,
}

pub struct LoginGrant {
    pub username: String,
    pub cookie: String,
}

pub struct ValidSession {
    pub username: String,
    pub refreshed_cookie: Option<String>,
}

pub struct AuthStatus {
    pub configured: bool,
    pub logged_in_as: Option<String>,
    pub refreshed_cookie: Option<String>,
}

impl UiAuth {
    pub fn open(data_dir: &Path, gateway_token: &str, username: Option<&str>, password: Option<&str>, secure_cookie: bool) -> Result<Self, String> {
        DirBuilder::new().mode(0o700).recursive(true).create(data_dir)
            .or_else(|error| if error.kind() == std::io::ErrorKind::AlreadyExists { Ok(()) } else { Err(error) })
            .map_err(|_| "Không tạo được thư mục dữ liệu đăng nhập web.".to_owned())?;
        let path = data_dir.join("ui_auth.json");
        let _disk_lock = lock_dir(data_dir)?;
        let store = match read_auth(&path) {
            Ok(bytes) => {
                decode_store(&bytes)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut store = Store::default();
                match (username, password) {
                    (Some(user), Some(pass)) => {
                        validate_credentials(user, pass)?;
                        drop(_disk_lock);
                        let account = make_account(user, pass)?;
                        let _disk_lock = lock_dir(data_dir)?;
                        match read_auth(&path) {
                            Ok(bytes) => store = decode_store(&bytes)?,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                store.account = Some(account);
                                save(&path, &store)?;
                            }
                            Err(_) => return Err("Không đọc được cấu hình đăng nhập web.".into()),
                        }
                    }
                    (None, None) => (),
                    _ => return Err("TG_UI_USER và TG_UI_PASSWORD phải được đặt cùng nhau.".into()),
                }
                store
            }
            Err(_) => return Err("Không đọc được cấu hình đăng nhập web.".into()),
        };
        Ok(Self {
            path: Some(path), gateway_token: gateway_token.to_owned(), store: Mutex::new(store),
            failures: Mutex::new(HashMap::new()), hash_slots: Arc::new(tokio::sync::Semaphore::new(2)), clock: Arc::new(now), secure_cookie,
        })
    }

    /// An isolated auth store for routers created by library callers and unit tests.
    pub fn in_memory(gateway_token: &str) -> Self {
        Self {
            path: None, gateway_token: gateway_token.to_owned(), store: Mutex::new(Store::default()),
            failures: Mutex::new(HashMap::new()), hash_slots: Arc::new(tokio::sync::Semaphore::new(2)), clock: Arc::new(now), secure_cookie: false,
        }
    }

    pub fn secure_cookie(&self) -> bool { self.secure_cookie }

    pub(crate) async fn hash_permit(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.hash_slots.clone().acquire_owned().await.expect("auth semaphore stays open")
    }

    pub fn status(&self, token: Option<&str>, secure: bool) -> Result<AuthStatus, AuthError> {
        let (_disk_lock, mut store) = self.lock_store()?;
        let configured = store.account.is_some();
        let session = match token.map(|token| self.refresh(&mut store, token, secure)) {
            Some(Ok(session)) => Some(session),
            Some(Err(error)) if error.status != 401 => return Err(error),
            _ => None,
        };
        Ok(AuthStatus {
            configured,
            logged_in_as: session.as_ref().map(|session| session.username.clone()),
            refreshed_cookie: session.and_then(|session| session.refreshed_cookie),
        })
    }

    pub fn setup(&self, token: &str, username: &str, password: &str, secure: bool) -> Result<LoginGrant, AuthError> {
        {
            let (_disk_lock, store) = self.lock_store()?;
            if store.account.is_some() { return Err(error(409, "ALREADY_CONFIGURED", "Tài khoản đã được thiết lập.")); }
        }
        if self.gateway_token.is_empty() || !ct_eq(token.as_bytes(), self.gateway_token.as_bytes()) {
            return Err(error(401, "UNAUTHORIZED", "Thiếu hoặc sai token khởi tạo."));
        }
        validate_credentials(username, password).map_err(|_| error(400, "BAD_REQUEST", "Tên đăng nhập hoặc mật khẩu không hợp lệ."))?;
        let account = make_account(username, password).map_err(|_| storage_error())?;
        let (_disk_lock, mut store) = self.lock_store()?;
        if store.account.is_some() { return Err(error(409, "ALREADY_CONFIGURED", "Tài khoản đã được thiết lập.")); }
        let mut updated = store.clone();
        updated.account = Some(account);
        let grant = create_session(&mut updated, username, true, (self.clock)(), secure || self.secure_cookie)?;
        self.save_store(&updated)?;
        *store = updated;
        Ok(grant)
    }

    pub fn login(&self, username: &str, password: &str, remember: bool, ip: &str, secure: bool) -> Result<LoginGrant, AuthError> {
        if username.len() > 128 || password.len() > 4096 || password.chars().count() > 1024 || ip.len() > 64 {
            return Err(error(401, "INVALID_CREDENTIALS", "Tên hoặc mật khẩu không đúng."));
        }
        let now = (self.clock)();
        let key = (ip.to_owned(), username.to_owned());
        // Reload first so a CLI password reset also clears the previous account's lockout.
        let account = { let (_disk_lock, store) = self.lock_store()?; store.account.clone() };
        // Reserve the attempt atomically before hashing, without holding the account/disk lock.
        self.record_failure(key.clone(), now)?;
        let candidate = account.as_ref().filter(|account| account.username == username);
        let (salt, expected) = candidate.map(|account| (account.salt.as_slice(), account.password_hash.as_slice()))
            .unwrap_or((&DUMMY_SALT, &[0; HASH_BYTES]));
        let valid = verify_password(password, salt, expected) && candidate.is_some();
        if !valid {
            return Err(error(401, "INVALID_CREDENTIALS", "Tên hoặc mật khẩu không đúng."));
        }
        let (_disk_lock, mut store) = self.lock_store()?;
        if !same_account(account.as_ref(), store.account.as_ref()) {
            return Err(error(401, "INVALID_CREDENTIALS", "Tên hoặc mật khẩu không đúng."));
        }
        let mut updated = store.clone();
        let grant = create_session(&mut updated, username, remember, now, secure || self.secure_cookie)?;
        self.save_store(&updated)?;
        *store = updated;
        self.failures.lock().unwrap_or_else(|p| p.into_inner()).remove(&key);
        Ok(grant)
    }

    pub fn validate_session(&self, token: &str, secure: bool) -> Result<ValidSession, AuthError> {
        let (_disk_lock, mut store) = self.lock_store()?;
        self.refresh(&mut store, token, secure)
    }

    fn refresh(&self, store: &mut Store, token: &str, secure: bool) -> Result<ValidSession, AuthError> {
        if token.is_empty() || token.len() > 128 { return Err(unauthorized()); }
        let hash = session_hash(token);
        let now = (self.clock)();
        let Some(index) = store.sessions.iter().position(|session| ct_eq(session.hash.as_bytes(), hash.as_bytes())) else {
            return Err(unauthorized());
        };
        if store.sessions[index].expires_at <= now {
            let mut updated = store.clone();
            updated.sessions.remove(index);
            self.save_store(&updated).map_err(|_| storage_error())?;
            *store = updated;
            return Err(unauthorized());
        }
        let account = store.account.as_ref().map(|account| account.username.clone()).ok_or_else(unauthorized)?;
        let session = &store.sessions[index];
        let refreshed_cookie = if session.expires_at.saturating_sub(now) < SESSION_SECONDS / 2 {
            let refreshed = cookie(token, session.remember, secure || self.secure_cookie || session.secure);
            let mut updated = store.clone();
            updated.sessions[index].expires_at = now.saturating_add(SESSION_SECONDS);
            self.save_store(&updated).map_err(|_| storage_error())?;
            *store = updated;
            Some(refreshed)
        } else { None };
        Ok(ValidSession { username: account, refreshed_cookie })
    }

    pub fn logout(&self, token: &str) -> Result<(), AuthError> {
        let hash = session_hash(token);
        let (_disk_lock, mut store) = self.lock_store()?;
        let index = active_session(&store, &hash, (self.clock)())?;
        let mut updated = store.clone();
        updated.sessions.remove(index);
        self.save_store(&updated).map_err(|_| storage_error())?;
        *store = updated;
        Ok(())
    }

    pub fn logout_all(&self, token: &str) -> Result<(), AuthError> {
        let (_disk_lock, mut store) = self.lock_store()?;
        active_session(&store, &session_hash(token), (self.clock)())?;
        let mut updated = store.clone();
        updated.sessions.clear();
        self.save_store(&updated).map_err(|_| storage_error())?;
        *store = updated;
        Ok(())
    }

    pub fn change_password(&self, token: &str, old: &str, new: &str) -> Result<(), AuthError> {
        let hash = session_hash(token);
        let account = {
            let (_disk_lock, store) = self.lock_store()?;
            active_session(&store, &hash, (self.clock)())?;
            store.account.clone().ok_or_else(unauthorized)?
        };
        let valid = verify_password(old, &account.salt, &account.password_hash);
        if !valid { return Err(error(401, "INVALID_CREDENTIALS", "Tên hoặc mật khẩu không đúng.")); }
        validate_credentials(&account.username, new)
            .map_err(|_| error(400, "BAD_REQUEST", "Mật khẩu mới phải có ít nhất 10 ký tự và không trùng tên người dùng."))?;
        let updated_account = make_account(&account.username, new).map_err(|_| storage_error())?;
        let (_disk_lock, mut store) = self.lock_store()?;
        active_session(&store, &hash, (self.clock)())?;
        if !same_account(Some(&account), store.account.as_ref()) {
            return Err(error(401, "INVALID_CREDENTIALS", "Tên hoặc mật khẩu không đúng."));
        }
        let username = account.username.clone();
        let mut updated = store.clone();
        updated.account = Some(updated_account);
        updated.sessions.retain(|session| ct_eq(session.hash.as_bytes(), hash.as_bytes()));
        self.save_store(&updated).map_err(|_| storage_error())?;
        *store = updated;
        self.failures.lock().unwrap_or_else(|p| p.into_inner()).retain(|(_, user), _| user != &username);
        Ok(())
    }

    pub fn reset_password(data_dir: &Path, password: &str) -> Result<(), String> {
        let path = data_dir.join("ui_auth.json");
        let original = {
            let _disk_lock = lock_dir(data_dir)?;
            let bytes = read_auth(&path).map_err(|_| "Chưa có tài khoản web để đặt lại mật khẩu.".to_owned())?;
            decode_store(&bytes)?.account.ok_or_else(|| "Chưa có tài khoản web để đặt lại mật khẩu.".to_owned())?
        };
        let account = &original;
        validate_credentials(&account.username, password)?;
        let username = account.username.clone();
        let updated_account = make_account(&username, password)?;
        let _disk_lock = lock_dir(data_dir)?;
        let mut store = decode_store(&read_auth(&path).map_err(|_| "Không đọc được cấu hình đăng nhập web.".to_owned())?)?;
        if !same_account(Some(&original), store.account.as_ref()) { return Err("Tài khoản vừa thay đổi; hãy chạy lại lệnh passwd.".into()); }
        store.account = Some(updated_account);
        store.sessions.clear();
        save(&path, &store)
    }

    fn record_failure(&self, key: (String, String), now: u64) -> Result<(), AuthError> {
        let mut failures = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        failures.retain(|_, attempts| {
            while attempts.front().is_some_and(|attempt| now.saturating_sub(*attempt) >= FAILURE_WINDOW) { attempts.pop_front(); }
            !attempts.is_empty()
        });
        if !failures.contains_key(&key) {
            let ip_keys = failures.keys().filter(|(ip, _)| ip == &key.0).count();
            if ip_keys >= MAX_KEYS_PER_IP || failures.len() >= 4096 {
                return Err(login_limited(FAILURE_WINDOW));
            }
        }
        let attempts = failures.entry(key).or_default();
        if attempts.len() >= MAX_FAILURES {
            return Err(login_limited(FAILURE_WINDOW.saturating_sub(now.saturating_sub(*attempts.front().unwrap())).max(1)));
        }
        attempts.push_back(now);
        Ok(())
    }

    fn lock_store(&self) -> Result<(Option<File>, MutexGuard<'_, Store>), AuthError> {
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let disk_lock = if let Some(path) = &self.path {
            // The directory inode survives atomic replacement of ui_auth.json by either process.
            let lock = lock_dir(path.parent().unwrap()).map_err(|_| storage_error())?;
            let loaded = match read_auth(path) {
                Ok(bytes) => decode_store(&bytes).map_err(|_| storage_error())?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Store::default(),
                Err(_) => return Err(storage_error()),
            };
            if !same_account(store.account.as_ref(), loaded.account.as_ref()) {
                self.failures.lock().unwrap_or_else(|p| p.into_inner()).clear();
            }
            *store = loaded;
            Some(lock)
        } else { None };
        Ok((disk_lock, store))
    }

    fn save_store(&self, store: &Store) -> Result<(), AuthError> {
        if let Some(path) = &self.path { save(path, store).map_err(|_| storage_error())?; }
        Ok(())
    }
}

fn lock_dir(path: &Path) -> Result<File, String> {
    let unavailable = |_| "Không khoá được thư mục đăng nhập web.".to_owned();
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::DIRECTORY;
    let dir = OpenOptions::new().read(true).custom_flags(flags.bits() as i32).open(path).map_err(unavailable)?;
    dir.set_permissions(fs::Permissions::from_mode(0o700)).map_err(|_| "Không bảo vệ được thư mục đăng nhập web.".to_owned())?;
    flock(&dir, FlockOperation::LockExclusive).map_err(|_| "Không khoá được thư mục đăng nhập web.".to_owned())?;
    Ok(dir)
}

fn read_auth(path: &Path) -> std::io::Result<Vec<u8>> {
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let mut file = OpenOptions::new().read(true).custom_flags(flags.bits() as i32).open(path)?;
    if !file.metadata()?.is_file() { return Err(std::io::ErrorKind::InvalidInput.into()); }
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn active_session(store: &Store, hash: &str, now: u64) -> Result<usize, AuthError> {
    store.sessions.iter().position(|session| {
        session.expires_at > now && ct_eq(session.hash.as_bytes(), hash.as_bytes())
    }).ok_or_else(unauthorized)
}

fn same_account(first: Option<&Account>, second: Option<&Account>) -> bool {
    match (first, second) {
        (Some(first), Some(second)) => first.username == second.username
            && ct_eq(&first.salt, &second.salt)
            && ct_eq(&first.password_hash, &second.password_hash),
        (None, None) => true,
        _ => false,
    }
}

pub fn secure_cookie_from_env() -> Result<bool, String> {
    match std::env::var("TGGW_UI_COOKIE_SECURE").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => Err("TGGW_UI_COOKIE_SECURE phải là 0 hoặc 1.".into()),
    }
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }

fn error(status: u16, code: &'static str, message: &'static str) -> AuthError {
    AuthError { status, code, message, retry_after: None }
}

fn unauthorized() -> AuthError { error(401, "UNAUTHORIZED", "Chưa đăng nhập hoặc phiên đã hết hạn.") }
fn login_limited(retry: u64) -> AuthError {
    AuthError { status: 429, code: "LOGIN_RATE_LIMITED", message: "Quá nhiều lần đăng nhập sai.", retry_after: Some(retry) }
}
fn storage_error() -> AuthError { error(500, "AUTH_STORAGE", "Không lưu được dữ liệu đăng nhập web.") }

fn validate_credentials(username: &str, password: &str) -> Result<(), String> {
    if username.trim().is_empty() || username.len() > 128 || username.chars().any(char::is_control) {
        return Err("Tên đăng nhập không hợp lệ.".into());
    }
    let count = password.chars().count();
    if count < 10 || count > 1024 || password == username { return Err("Mật khẩu không hợp lệ.".into()); }
    Ok(())
}

fn make_account(username: &str, password: &str) -> Result<Account, String> {
    let rng = SystemRandom::new();
    let mut salt = vec![0; SALT_BYTES];
    rng.fill(&mut salt).map_err(|_| "Không tạo được muối mật khẩu.".to_owned())?;
    let mut password_hash = vec![0; HASH_BYTES];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::new(ITERATIONS).unwrap(), &salt, password.as_bytes(), &mut password_hash);
    Ok(Account { username: username.to_owned(), salt, password_hash })
}

fn verify_password(password: &str, salt: &[u8], expected: &[u8]) -> bool {
    pbkdf2::verify(pbkdf2::PBKDF2_HMAC_SHA256, NonZeroU32::new(ITERATIONS).unwrap(), salt,
        password.as_bytes(), expected).is_ok()
}

fn create_session(store: &mut Store, username: &str, remember: bool, now: u64, secure: bool) -> Result<LoginGrant, AuthError> {
    let rng = SystemRandom::new();
    let mut raw = [0; SESSION_BYTES];
    rng.fill(&mut raw).map_err(|_| error(500, "AUTH_RANDOM", "Không tạo được phiên đăng nhập."))?;
    let token = URL_SAFE_NO_PAD.encode(raw);
    let cookie = cookie(&token, remember, secure);
    store.sessions.retain(|session| session.expires_at > now);
    store.sessions.push(Session { hash: session_hash(&token), expires_at: now.saturating_add(SESSION_SECONDS), remember, secure });
    Ok(LoginGrant { username: username.to_owned(), cookie })
}

fn session_hash(token: &str) -> String {
    digest::digest(&digest::SHA256, token.as_bytes()).as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn cookie(token: &str, remember: bool, secure: bool) -> String {
    let mut value = format!("tggw_session={token}; Path=/; HttpOnly; SameSite=Lax");
    if remember { value.push_str(&format!("; Max-Age={SESSION_SECONDS}")); }
    if secure { value.push_str("; Secure"); }
    value
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool { bool::from(a.ct_eq(b)) }

fn decode_store(bytes: &[u8]) -> Result<Store, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "ui_auth.json không hợp lệ.".to_owned())?;
    let account = if value["account"].is_object() {
        let row = &value["account"];
        let username = row["username"].as_str().ok_or_else(|| "ui_auth.json không hợp lệ.".to_owned())?;
        let salt = URL_SAFE_NO_PAD.decode(row["salt"].as_str().unwrap_or_default()).map_err(|_| "ui_auth.json không hợp lệ.".to_owned())?;
        let password_hash = URL_SAFE_NO_PAD.decode(row["password_hash"].as_str().unwrap_or_default()).map_err(|_| "ui_auth.json không hợp lệ.".to_owned())?;
        if salt.len() < SALT_BYTES || password_hash.len() != HASH_BYTES { return Err("ui_auth.json không hợp lệ.".into()); }
        Some(Account { username: username.into(), salt, password_hash })
    } else { None };
    let sessions = value["sessions"].as_array().ok_or_else(|| "ui_auth.json không hợp lệ.".to_owned())?.iter().map(|row| {
        let hash = row["hash"].as_str().filter(|hash| hash.len() == 64).ok_or_else(|| "ui_auth.json không hợp lệ.".to_owned())?;
        let expires_at = row["expires_at"].as_u64().ok_or_else(|| "ui_auth.json không hợp lệ.".to_owned())?;
        Ok(Session { hash: hash.into(), expires_at, remember: row["remember"].as_bool().unwrap_or(false), secure: row["secure"].as_bool().unwrap_or(false) })
    }).collect::<Result<Vec<_>, String>>()?;
    Ok(Store { account, sessions })
}

fn save(path: &Path, store: &Store) -> Result<(), String> {
    let account = store.account.as_ref().map(|account| json!({
        "username": account.username,
        "salt": URL_SAFE_NO_PAD.encode(&account.salt),
        "password_hash": URL_SAFE_NO_PAD.encode(&account.password_hash),
    })).unwrap_or(Value::Null);
    let sessions = store.sessions.iter().map(|session| json!({
        "hash": session.hash, "expires_at": session.expires_at, "remember": session.remember, "secure": session.secure,
    })).collect::<Vec<_>>();
    crate::accounts::atomic_json(path, &json!({"account":account,"sessions":sessions}))
        .map_err(|_| "Không lưu được dữ liệu đăng nhập web.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn trung01_failure_table_is_bounded_and_expired_keys_are_reclaimed() {
        let auth = UiAuth::in_memory("fake-token");
        for id in 0..8192 { let _ = auth.record_failure(("192.0.2.10".into(), format!("{id:0128}")), 10_000); }
        let failures = auth.failures.lock().unwrap();
        let bytes: usize = failures.capacity() * std::mem::size_of::<((String, String), VecDeque<u64>)>()
            + failures.iter().map(|((ip, user), times)| ip.capacity() + user.capacity() + times.capacity() * 8).sum::<usize>();
        assert!(failures.len() <= 4096, "failure table has {} keys", failures.len());
        assert!(bytes < 2 * 1024 * 1024, "failure table allocation estimate: {bytes}");
        drop(failures);
        auth.record_failure(("192.0.2.10".into(), "after-expiry".into()), 10_900).unwrap();
        assert_eq!(auth.failures.lock().unwrap().len(), 1);
    }

    #[test]
    fn trung01_oversized_credentials_are_rejected_before_allocating_failure_keys() {
        let auth = UiAuth::in_memory("fake-token");
        for (user, password) in [("u".repeat(129), "fake-password-value".into()),
            ("fake-user".into(), "p".repeat(4097))] {
            assert_eq!(auth.login(&user, &password, false, "192.0.2.10", false).err().unwrap().status, 401);
            assert!(auth.failures.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn password_hash_uses_pbkdf2_and_session_storage_keeps_only_hash() {
        let root = std::env::temp_dir().join(format!("tggw-ui-auth-unit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let auth = UiAuth::open(&root, "fake-token", None, None, false).unwrap();
        let grant = auth.setup("fake-token", "fake-user", "fake-password-value", false).unwrap();
        let saved = fs::read_to_string(root.join("ui_auth.json")).unwrap();
        assert!(!saved.contains("fake-password-value"));
        assert!(!saved.contains(grant.cookie.split_once('=').unwrap().1.split(';').next().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn environment_bootstrap_is_used_only_when_the_store_is_missing() {
        let root = std::env::temp_dir().join(format!("tggw-ui-auth-bootstrap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let auth = UiAuth::open(&root, "fake-token", Some("fake-user"), Some("fake-password-value"), false).unwrap();
        assert!(auth.status(None, false).unwrap().configured);
        drop(auth);
        let reopened = UiAuth::open(&root, "fake-token", Some("other-user"), Some("other-password-value"), false).unwrap();
        assert!(reopened.login("fake-user", "fake-password-value", true, "192.0.2.10", false).is_ok());
        assert_eq!(reopened.login("other-user", "other-password-value", true, "192.0.2.10", false).err().unwrap().status, 401);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lock_window_expires_using_injected_clock() {
        let now = Arc::new(AtomicU64::new(10_000));
        let clock: Clock = { let now = now.clone(); Arc::new(move || now.load(Ordering::SeqCst)) };
        let auth = UiAuth {
            path: None, gateway_token: "fake-token".into(), store: Mutex::new(Store::default()),
            failures: Mutex::new(HashMap::new()), hash_slots: Arc::new(tokio::sync::Semaphore::new(2)), clock, secure_cookie: false,
        };
        auth.setup("fake-token", "fake-user", "fake-password-value", false).unwrap();
        for _ in 0..5 { assert_eq!(auth.login("fake-user", "wrong-password-value", false, "192.0.2.10", false).err().unwrap().status, 401); }
        assert_eq!(auth.login("fake-user", "fake-password-value", false, "192.0.2.10", false).err().unwrap().status, 429);
        now.fetch_add(FAILURE_WINDOW, Ordering::SeqCst);
        let grant = auth.login("fake-user", "fake-password-value", false, "192.0.2.10", false).unwrap();
        let token = grant.cookie.split_once('=').unwrap().1.split(';').next().unwrap();
        now.fetch_add(SESSION_SECONDS, Ordering::SeqCst);
        assert_eq!(auth.validate_session(token, false).err().unwrap().status, 401);
    }

    #[tokio::test]
    async fn tc05_tc12_router_observes_fake_clock_lock_expiry_and_session_renewal() {
        use axum::{body::Body, http::{header, Request}};
        use tower::ServiceExt;

        let now = Arc::new(AtomicU64::new(10_000));
        let mut auth = UiAuth::in_memory("fake-token");
        auth.clock = { let now = now.clone(); Arc::new(move || now.load(Ordering::SeqCst)) };
        let auth = Arc::new(auth);
        let grant = auth.setup("fake-token", "fake-user", "fake-password-value", true).unwrap();
        let cookie = grant.cookie.split(';').next().unwrap();
        let stub = Arc::new(crate::rest::ChuaCo);
        let app = crate::rest::router_with_ui_auth(stub.clone(), stub, crate::rest::RestConfig {
            token: "fake-token".into(), rate_limit_per_minute: 120, version: "test".into(),
        }, auth.clone());
        for _ in 0..5 {
            assert_eq!(auth.login("fake-user", "wrong-password-value", false, "unknown", false).err().unwrap().status, 401);
        }
        let login = || Request::post("/ui/api/auth/login").header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"ten":"fake-user","mat_khau":"fake-password-value"}).to_string())).unwrap();
        let limited = app.clone().oneshot(login()).await.unwrap();
        assert_eq!(limited.status(), 429);
        assert_eq!(limited.headers()[header::RETRY_AFTER], "900");
        now.fetch_add(FAILURE_WINDOW, Ordering::SeqCst);
        assert_eq!(app.clone().oneshot(login()).await.unwrap().status(), 200);

        now.fetch_add(SESSION_SECONDS / 2, Ordering::SeqCst);
        let read = || Request::get("/api/live/calls").header(header::COOKIE, cookie).body(Body::empty()).unwrap();
        let renewed = app.clone().oneshot(read()).await.unwrap();
        assert_eq!(renewed.status(), 200);
        let refreshed = renewed.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(refreshed.contains("Max-Age=7776000") && refreshed.contains("Secure"));
        now.fetch_add(SESSION_SECONDS / 2, Ordering::SeqCst);
        assert_eq!(app.clone().oneshot(read()).await.unwrap().status(), 200);
        now.fetch_add(SESSION_SECONDS, Ordering::SeqCst);
        assert_eq!(app.oneshot(read()).await.unwrap().status(), 401);
    }

    #[test]
    fn token_rotation_does_not_invalidate_web_credentials_or_sessions() {
        let root = std::env::temp_dir().join(format!("tggw-ui-auth-rotation-{}", std::process::id()));
        let auth = UiAuth::open(&root, "fake-token", None, None, true).unwrap();
        let grant = auth.setup("fake-token", "fake-user", "fake-password-value", false).unwrap();
        assert!(grant.cookie.contains("Secure"));
        drop(auth);
        let reopened = UiAuth::open(&root, "rotated-fake-token", None, None, true).unwrap();
        let token = grant.cookie.split_once('=').unwrap().1.split(';').next().unwrap();
        assert!(reopened.validate_session(token, false).is_ok());
        assert!(reopened.login("fake-user", "fake-password-value", true, "192.0.2.10", false).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn single_ip_username_spray_cannot_lock_out_other_ips() {
        let now = Arc::new(AtomicU64::new(10_000_u64));
        let clock: Clock = { let now = now.clone(); Arc::new(move || now.load(Ordering::SeqCst)) };
        let auth = UiAuth {
            path: None, gateway_token: "fake-token".into(), store: Mutex::new(Store::default()),
            failures: Mutex::new(HashMap::new()), hash_slots: Arc::new(tokio::sync::Semaphore::new(2)), clock, secure_cookie: false,
        };
        auth.setup("fake-token", "fake-user", "fake-password-value", false).unwrap();
        for i in 0..4096_usize {
            let _ = auth.record_failure(("198.51.100.1".into(), format!("u{i}")), 10_000);
        }
        assert!(auth.failures.lock().unwrap().len() <= MAX_KEYS_PER_IP);
        assert!(auth.login("fake-user", "fake-password-value", false, "192.0.2.50", false).is_ok());
    }
}
