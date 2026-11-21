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
    pub fn ingest(&mut self,phone:&str,result:&Value) {
        let previous=self.source_cursor(phone);
        for item in result["items"].as_array().cloned().unwrap_or_default() {
            if item["id"].as_i64().unwrap_or_default()>previous {self.push(phone,item);}
        }
        self.cursors.insert(phone.into(),result["cursor"].as_i64().unwrap_or_default());
    }
    pub fn poll(&self,since:i64)->Value {
        let items:Vec<_>=self.items.iter().filter(|v|v["id"].as_i64().unwrap()>since).collect();
        let first=self.items.front().and_then(|v|v["id"].as_i64()).unwrap_or(self.cursor+1);
        json!({"items":items,"cursor":self.cursor,"reset":since>0 && since<first-1})
    }
}

pub(crate) struct Journal { path:PathBuf, data:Value }
impl Journal {
    pub fn open(data:&Path)->io::Result<Self> {
        fs::create_dir_all(data)?;
        let path=data.join("account_events.json");
        let data=match fs::read(&path) {
            Ok(bytes)=>serde_json::from_slice(&bytes)?,
            Err(e) if e.kind()==io::ErrorKind::NotFound=>json!({"cursor":0,"sources":{},"events":[]}),
            Err(e)=>return Err(e),
        };
        Ok(Self{path,data})
    }
    pub fn source_cursor(&self,phone:&str)->i64 { self.data["sources"][phone].as_i64().unwrap_or_default() }
    pub fn forget_source(&mut self,phone:&str)->io::Result<()> {
        let mut data=self.data.clone();
        data["sources"].as_object_mut().unwrap().remove(phone);
        crate::accounts::atomic_json(&self.path,&data)?;
        self.data=data;
        Ok(())
    }
    pub fn ingest(&mut self,phone:&str,result:&Value)->io::Result<()> {
        let mut data=self.data.clone();
        let mut cursor=data["cursor"].as_i64().unwrap_or_default();
        let events=data["events"].as_array_mut().ok_or_else(||io::Error::other("sổ sự kiện không hợp lệ"))?;
        let previous=self.source_cursor(phone);
        for mut event in result["events"].as_array().cloned().unwrap_or_default() {
            if event["id"].as_i64().unwrap_or_default()<=previous {continue;}
            cursor+=1; event["id"]=json!(cursor); events.push(event);
        }
        if cursor==self.data["cursor"].as_i64().unwrap_or_default() { return Ok(()); }
        if events.len()>10000 { events.drain(..events.len()-10000); }
        data["cursor"]=json!(cursor);
        data["sources"][phone]=result["cursor"].clone();
        crate::accounts::atomic_json(&self.path,&data)?;
        self.data=data;
        Ok(())
    }
    pub fn poll(&self,since:i64)->Value {
        let events:Vec<_>=self.data["events"].as_array().unwrap().iter().filter(|v|v["id"].as_i64().unwrap()>since).collect();
        json!({"events":events,"cursor":self.data["cursor"]})
    }
}
