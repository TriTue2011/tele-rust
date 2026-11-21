//! Sổ tài khoản và bố cục phiên tương thích `store.py`.
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub(crate) fn account_id(phone: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, phone.as_bytes()).as_ref()[..8]
        .iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> f64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64() }

#[derive(Clone)]
pub(crate) struct Account {
    pub phone: String,
    pub display_name: String,
    pub status: String,
    added_at: f64,
    last_seen: f64,
    legacy_session: bool,
}

impl Account {
    fn parse(v: &Value) -> io::Result<Self> {
        Ok(Self {
            phone: v["phone"].as_str().filter(|s| !s.is_empty()).ok_or_else(invalid)?.into(),
            display_name: v["display_name"].as_str().unwrap_or_default().into(),
            status: v["status"].as_str().unwrap_or("offline").into(),
            added_at: v["added_at"].as_f64().unwrap_or_default(),
            last_seen: v["last_seen"].as_f64().unwrap_or_default(),
            legacy_session: v["legacy_session"].as_bool().unwrap_or(false),
        })
    }

    fn disk(&self) -> Value {
        let mut v=json!({"phone":self.phone, "display_name":self.display_name, "status":self.status,
            "added_at":self.added_at, "last_seen":self.last_seen});
        if self.legacy_session { v["legacy_session"]=json!(true); }
        v
    }
}

fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "accounts.json không hợp lệ") }

pub(crate) struct AccountStore {
    data: PathBuf,
    rows: Vec<Account>,
    pub legacy: bool,
}

impl AccountStore {
    pub fn open(data: &Path, phone: &str) -> io::Result<Self> {
        let path = data.join("accounts.json");
        let (rows, legacy) = match fs::read(path) {
            Ok(bytes) => {
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                let rows = value.as_array().ok_or_else(invalid)?.iter().map(Account::parse)
                    .collect::<io::Result<Vec<_>>>()?;
                (rows, false)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound =>
                (Vec::new(), !phone.is_empty() && data.join("tdlib/files").is_dir()),
            Err(e) => return Err(e),
        };
        Ok(Self { data: data.into(), rows, legacy })
    }

    pub fn rows(&self) -> &[Account] { &self.rows }

    pub fn session_dir(&self, phone: &str) -> PathBuf {
        if self.rows.iter().any(|a|a.phone==phone && a.legacy_session) {return self.data.join("tdlib");}
        self.data.join("tdlib").join(account_id(phone))
    }

    pub fn prepare(&self, phone: &str) -> io::Result<PathBuf> {
        let root = self.data.join("tdlib");
        DirBuilder::new().mode(0o700).recursive(true).create(&root)?;
        let session = self.session_dir(phone);
        crate::td::login::ensure_session_dir(&session)?;
        Ok(session)
    }

    pub fn put_online(&mut self, phone: &str, name: String) -> io::Result<()> {
        let session = self.prepare(phone)?;
        let ledger = session.join("tai_khoan.json");
        if !ledger.exists() {
            atomic_json(&ledger, &json!({"phone":phone,"mock":false}))?;
        }
        let mut rows = self.rows.clone();
        let moment = now();
        if let Some(row) = rows.iter_mut().find(|a| a.phone == phone) {
            if !name.is_empty() { row.display_name = name; }
            row.status = "online".into();
            row.last_seen = moment;
        } else {
            rows.push(Account { phone: phone.into(), display_name: name, status: "online".into(),
                added_at: moment, last_seen: moment, legacy_session:false });
        }
        self.save(rows)
    }

    pub fn refresh_name(&mut self, phone: &str, name: &str) -> io::Result<()> {
        let Some(index) = self.rows.iter().position(|a|a.phone==phone) else { return Ok(()) };
        if name.is_empty() || self.rows[index].display_name == name { return Ok(()) }
        let mut rows = self.rows.clone();
        rows[index].display_name = name.into();
        self.save(rows)
    }

    fn save(&mut self, rows: Vec<Account>) -> io::Result<()> {
        atomic_json(&self.data.join("accounts.json"), &Value::Array(rows.iter().map(Account::disk).collect()))?;
        self.rows = rows;
        Ok(())
    }

    pub fn keep_legacy(&mut self,phone:&str)->io::Result<()> {
        if !self.legacy {return Ok(());}
        let moment=now();
        let mut rows=self.rows.clone();
        rows.push(Account{phone:phone.into(),display_name:String::new(),status:"online".into(),
            added_at:moment,last_seen:moment,legacy_session:true});
        self.save(rows)?;
        self.legacy=false;
        Ok(())
    }

    /// Chỉ gọi sau Closed; giữ DB và khóa cùng nhau trong thư mục có thể khôi phục.
    pub fn archive(&mut self,phone:&str)->io::Result<PathBuf> {
        if !self.rows.iter().any(|a|a.phone==phone) {return Err(io::Error::from(io::ErrorKind::NotFound));}
        let source=self.session_dir(phone);
        let archive=self.data.join("tdlib").join(format!(".da-xoa-{}-{}",SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos(),account_id(phone)));
        let legacy=source==self.data.join("tdlib");
        let mut moved=Vec::new();
        if legacy {
            DirBuilder::new().mode(0o700).create(&archive)?;
            for name in ["files","khoa","tai_khoan.json"] {
                if source.join(name).exists() {
                    if let Err(error)=fs::rename(source.join(name),archive.join(name)) {
                        for previous in moved.iter().rev() {fs::rename(archive.join(previous),source.join(previous))?;}
                        return Err(error);
                    }
                    moved.push(name);
                }
            }
        } else { fs::rename(&source,&archive)?; }
        let rows=self.rows.iter().filter(|a|a.phone!=phone).cloned().collect();
        if let Err(error)=self.save(rows) {
            if legacy {
                for name in moved.iter().rev() {fs::rename(archive.join(name),source.join(name))?;}
            } else {fs::rename(&archive,&source)?;}
            return Err(error);
        }
        Ok(archive)
    }
}

pub(crate) fn atomic_json(path: &Path, value: &Value) -> io::Result<()> {
    let mut nonce = [0; 16];
    rustls::crypto::ring::default_provider().secure_random.fill(&mut nonce).map_err(|_| io::Error::other("không tạo được tên tệp tạm"))?;
    let temp = path.with_extension(format!("{:032x}.tmp", u128::from_ne_bytes(nonce)));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
    let written = (|| {
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() { let _ = fs::remove_file(temp); }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn python_layout_and_existing_legacy_session() {
        let dir = std::env::temp_dir().join(format!("tggw-accounts-{}", std::process::id()));
        let legacy = dir.join("tdlib/files");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("sentinel"), b"old session").unwrap();
        let phone = "+84900000000";
        let mut store = AccountStore::open(&dir, phone).unwrap();
        assert!(store.legacy);
        assert!(!dir.join("accounts.json").exists());
        store.put_online(phone, "Tên thử".into()).unwrap();
        let session = store.session_dir(phone);
        assert_eq!(session.file_name().unwrap(), "e01f8e2c832852ae");
        assert_eq!(fs::read(legacy.join("sentinel")).unwrap(), b"old session");
        let reopened = AccountStore::open(&dir, phone).unwrap();
        assert!(!reopened.legacy);
        assert_eq!(reopened.rows()[0].display_name, "Tên thử");
        assert_eq!(reopened.rows()[0].status, "online");
        for (path, expected) in [(dir.join("accounts.json"), 0o600), (session.join("tai_khoan.json"), 0o600),
            (session.join("khoa"), 0o600), (session.join("files"), 0o700)] {
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o777, expected);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_is_kept_when_second_account_is_saved_and_archive_can_roll_back() {
        let dir=PathBuf::from(format!("target/accounts-legacy-{}",std::process::id()));
        fs::create_dir_all(dir.join("tdlib/files")).unwrap();
        fs::write(dir.join("tdlib/files/sentinel"),b"legacy").unwrap();
        let phone="+84900000000";
        let mut store=AccountStore::open(&dir,phone).unwrap();
        store.keep_legacy(phone).unwrap();
        store.put_online("+84900000001","Second".into()).unwrap();
        let mut store=AccountStore::open(&dir,phone).unwrap();
        assert_eq!(store.session_dir(phone),dir.join("tdlib"));
        assert_eq!(store.rows().len(),2);
        let archive=store.archive(phone).unwrap();
        assert_eq!(fs::read(archive.join("files/sentinel")).unwrap(),b"legacy");
        assert!(store.session_dir("+84900000001").join("files").is_dir());
        let second=store.session_dir("+84900000001");
        fs::remove_file(dir.join("accounts.json")).unwrap();
        fs::create_dir(dir.join("accounts.json")).unwrap();
        assert!(store.archive("+84900000001").is_err());
        assert!(second.join("khoa").is_file());
        assert_eq!(store.rows().len(),1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refresh_name_does_not_create_legacy_ledger_or_write_empty_unchanged_or_unknown_names() {
        use std::os::unix::fs::MetadataExt;
        let dir=PathBuf::from(format!("target/accounts-refresh-legacy-{}",std::process::id()));
        fs::create_dir_all(dir.join("tdlib/files")).unwrap();
        let phone="+84900000000";
        let mut store=AccountStore::open(&dir,phone).unwrap();
        store.refresh_name(phone,"Tên tươi").unwrap();
        assert!(store.legacy);
        assert!(store.rows().is_empty());
        assert!(!dir.join("accounts.json").exists());
        store.keep_legacy(phone).unwrap();
        store.refresh_name(phone,"Tên lưu").unwrap();
        let ledger=fs::File::open(dir.join("accounts.json")).unwrap();
        let before=ledger.metadata().unwrap();
        let bytes=fs::read(dir.join("accounts.json")).unwrap();
        for (phone,name) in [(phone,"Tên lưu"),(phone,""),("+84900000001","Tên lạ")] {
            store.refresh_name(phone,name).unwrap();
        }
        let after=fs::metadata(dir.join("accounts.json")).unwrap();
        assert_eq!(before.ino(),after.ino());
        assert_eq!(before.modified().unwrap(),after.modified().unwrap());
        assert_eq!(fs::read(dir.join("accounts.json")).unwrap(),bytes);
        assert_eq!(AccountStore::open(&dir,phone).unwrap().session_dir(phone),dir.join("tdlib"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_name_refresh_preserves_memory_and_can_be_retried() {
        let dir=PathBuf::from(format!("target/accounts-refresh-error-{}",std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let phone="+84900000000";
        let mut store=AccountStore::open(&dir,phone).unwrap();
        store.put_online(phone,"Tên cũ".into()).unwrap();
        let path=dir.join("accounts.json");
        let saved=dir.join("accounts.saved");
        fs::rename(&path,&saved).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(store.refresh_name(phone,"Tên mới").is_err());
        assert_eq!(store.rows()[0].display_name,"Tên cũ");
        assert!(!fs::read_dir(&dir).unwrap().any(|e|e.unwrap().path().extension().is_some_and(|s|s=="tmp")));
        fs::remove_dir(&path).unwrap();
        fs::rename(saved,&path).unwrap();
        store.refresh_name(phone,"Tên mới").unwrap();
        assert_eq!(AccountStore::open(&dir,phone).unwrap().rows()[0].display_name,"Tên mới");
        fs::remove_dir_all(dir).unwrap();
    }
}
