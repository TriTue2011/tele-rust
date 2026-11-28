//! Ghép dòng cập nhật riêng của từng tài khoản bằng con trỏ chung.
use std::{collections::{BTreeMap,VecDeque}, fs, io, path::{Path,PathBuf}};
use serde_json::{json,Value};

#[derive(Default)]
pub(crate) struct Feed {
    cursors: BTreeMap<String,i64>,
    items: VecDeque<Value>,
    cursor: i64,
}
impl Feed {
    pub fn push(&mut self,phone:&str,mut item:Value) {
        self.cursor+=1;
        item["id"]=json!(self.cursor);item["account"]=json!(phone);
        self.items.push_back(item);
        if self.items.len()>2000 {self.items.pop_front();}
    }
    pub fn source_cursor(&self,phone:&str)->i64 { self.cursors.get(phone).copied().unwrap_or_default() }
    pub fn forget_source(&mut self,phone:&str) {self.cursors.remove(phone);}
    pub fn ingest(&mut self, phone: &str, result: &Value) {
        let previous = self.source_cursor(phone);
        let mut latest = previous.max(result["cursor"].as_i64().unwrap_or(previous));
        for item in result["items"].as_array().cloned().unwrap_or_default() {
            let id = item["id"].as_i64().unwrap_or_default();
            if id > previous {
                latest = latest.max(id);
                self.push(phone, item);
            }
        }
        self.cursors.insert(phone.into(), latest);
    }
    pub fn poll(&self,since:i64)->Value {
        let items:Vec<_>=self.items.iter().filter(|v|v["id"].as_i64().is_some_and(|id| id > since)).collect();
        let first=self.items.front().and_then(|v|v["id"].as_i64()).unwrap_or(self.cursor+1);
        json!({"items":items,"cursor":self.cursor,"reset":since>0 && since<first-1})
    }
}

pub(crate) struct Journal { path:PathBuf, data:Value }
impl Journal {
    pub fn open(data:&Path)->io::Result<Self> {
        fs::create_dir_all(data)?;
        let path=data.join("account_events.json");
        let data: Value = match fs::read(&path) {
            Ok(bytes)=>serde_json::from_slice(&bytes)?,
            Err(e) if e.kind()==io::ErrorKind::NotFound=>json!({"cursor":0,"sources":{},"events":[]}),
            Err(e)=>return Err(e),
        };
        let valid = data["cursor"].is_i64()
            && data["sources"].is_object()
            && data["events"].as_array().is_some_and(|events| events.iter().all(|v| v["id"].is_i64()));
        if !valid {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "sổ sự kiện không hợp lệ"));
        }
        Ok(Self{path,data})
    }
    pub fn source_cursor(&self,phone:&str)->i64 { self.data["sources"][phone].as_i64().unwrap_or_default() }
    pub fn forget_source(&mut self,phone:&str)->io::Result<()> {
        let mut data=self.data.clone();
        if let Some(sources) = data["sources"].as_object_mut() {
            sources.remove(phone);
        }
        crate::accounts::atomic_json(&self.path,&data)?;
        self.data=data;
        Ok(())
    }
    pub fn ingest(&mut self,phone:&str,result:&Value)->io::Result<()> {
        let mut data=self.data.clone();
        let mut cursor=data["cursor"].as_i64().unwrap_or_default();
        let events=data["events"].as_array_mut().ok_or_else(||io::Error::other("sổ sự kiện không hợp lệ"))?;
        let previous=self.source_cursor(phone);
        let mut latest = previous.max(result["cursor"].as_i64().unwrap_or(previous));
        for mut event in result["events"].as_array().cloned().unwrap_or_default() {
            let id = event["id"].as_i64().unwrap_or_default();
            if id <= previous { continue; }
            latest = latest.max(id);
            cursor += 1; event["id"] = json!(cursor); events.push(event);
        }
        if cursor==self.data["cursor"].as_i64().unwrap_or_default() && latest==previous { return Ok(()); }
        if events.len()>10000 { events.drain(..events.len()-10000); }
        data["cursor"]=json!(cursor);
        data["sources"][phone]=json!(latest);
        crate::accounts::atomic_json(&self.path,&data)?;
        self.data=data;
        Ok(())
    }
    pub fn poll(&self,since:i64)->Value {
        let events:Vec<_>=self.data["events"].as_array()
            .map(|events| events.iter().filter(|v|v["id"].as_i64().is_some_and(|id| id > since)).collect())
            .unwrap_or_default();
        json!({"events":events,"cursor":self.data["cursor"]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_ingest_cursor_never_regresses_on_out_of_order_poll() {
        let mut feed = Feed::default();
        feed.ingest("+84900000001", &json!({"items": [{"id": 10}, {"id": 7}], "cursor": 8}));
        assert_eq!(feed.source_cursor("+84900000001"), 10);
        feed.ingest("+84900000001", &json!({"items": [{"id": 9}], "cursor": 6}));
        assert_eq!(feed.source_cursor("+84900000001"), 10);
        assert_eq!(feed.poll(0)["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn journal_open_rejects_malformed_schema_without_panicking() {
        let dir = std::env::temp_dir().join(format!("tggw-journal-malformed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for bad in [
            r#"{"cursor":"bad","sources":{},"events":[]}"#,
            r#"{"cursor":0,"sources":[],"events":[]}"#,
            r#"{"cursor":0,"sources":{},"events":{}}"#,
            r#"{"cursor":0,"sources":{},"events":[{"id":"bad"}]}"#,
        ] {
            fs::write(dir.join("account_events.json"), bad).unwrap();
            let err = Journal::open(&dir).err().expect("malformed journal must return Err");
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
