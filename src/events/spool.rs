use std::{collections::VecDeque, fs::{self, DirBuilder, File, OpenOptions}, io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};

use rustix::fs::{flock, open, FlockOperation, Mode, OFlags};
use serde_json::{json, Value};

const MAX_FILES: usize = 10_000;
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EVENT_BYTES: u64 = 64 * 1024;
const PENDING: &str = ".pending";

pub(super) struct Spool {
    dir: File,
    path: PathBuf,
    cursor: u64,
    bytes: u64,
    entries: VecDeque<Entry>,
}

struct Entry { id: u64, bytes: u64, event: Option<Value> }

fn event_path(id: u64) -> String { format!("{id:012}.json") }

impl Spool {
    pub(super) fn open(path: PathBuf) -> io::Result<Self> {
        if let Err(error) = DirBuilder::new().recursive(true).mode(0o700).create(&path) {
            if error.kind() != io::ErrorKind::AlreadyExists { return Err(error); }
        }
        let dir: File = open(&path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?.into();
        flock(&dir, FlockOperation::NonBlockingLockExclusive)?;
        dir.set_permissions(fs::Permissions::from_mode(0o700))?;
        match fs::remove_file(path.join(PENDING)) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = name.strip_suffix(".json").and_then(|s| s.parse::<u64>().ok()) else { continue };
            if id == 0 || event_path(id) != name || !entry.file_type()?.is_file() { continue; }
            entries.push(Entry { id, bytes: entry.metadata()?.len(), event: None });
        }
        entries.sort_unstable_by_key(|entry| entry.id);
        let cursor = entries.last().map_or(0, |entry| entry.id);
        let bytes = entries.iter().map(|entry| entry.bytes).sum();
        let mut spool = Self { dir, path, cursor, bytes, entries: entries.into() };
        spool.rotate()?;
        for entry in &mut spool.entries {
            if entry.bytes <= MAX_EVENT_BYTES {
                entry.event = restore(&spool.path.join(event_path(entry.id)), entry.id);
            }
        }
        Ok(spool)
    }

    pub(super) fn append(&mut self, mut event: Value) -> io::Result<Value> {
        let id = self.cursor.checked_add(1).ok_or_else(|| io::Error::other("cursor đầy"))?;
        event["id"] = json!(id);
        let bytes = serde_json::to_vec(&event)?;
        let len = bytes.len() as u64;
        if len > MAX_EVENT_BYTES { return Err(io::Error::other("sự kiện vượt giới hạn")); }
        let pending = self.path.join(PENDING);
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&pending)?;
        let written = (|| {
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&pending, self.path.join(event_path(id)))
        })();
        if written.is_err() { let _ = fs::remove_file(&pending); }
        written?;
        self.cursor = id;
        self.bytes += len;
        self.entries.push_back(Entry { id, bytes: len, event: Some(event.clone()) });
        self.rotate()?;
        self.dir.sync_all()?;
        Ok(event)
    }

    fn rotate(&mut self) -> io::Result<()> {
        while self.entries.len() > MAX_FILES || (self.bytes > MAX_BYTES && self.entries.len() > 1) {
            let entry = self.entries.front().unwrap();
            fs::remove_file(self.path.join(event_path(entry.id)))?;
            self.bytes -= self.entries.pop_front().unwrap().bytes;
        }
        Ok(())
    }

    pub(super) fn poll(&self, since: u64) -> Value {
        let events: Vec<_> = self.entries.iter().filter(|e| e.id > since).filter_map(|e| e.event.as_ref()).collect();
        json!({"events":events,"cursor":self.cursor})
    }
}

fn restore(path: &Path, id: u64) -> Option<Value> {
    let fd = open(path, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()).ok()?;
    let file = File::from(fd);
    if !file.metadata().ok()?.is_file() { return None; }
    file.set_permissions(fs::Permissions::from_mode(0o600)).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_EVENT_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_EVENT_BYTES { return None; }
    let event: Value = serde_json::from_slice(&bytes).ok()?;
    (event["id"].as_u64() == Some(id)).then_some(event)
}
