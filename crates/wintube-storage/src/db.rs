//! File-based JSON storage (history + settings).
//! SQLite would need a C compiler on Windows; JSON keeps the build
//! pure-Rust and the binary small. Data dir: %APPDATA%/Wintube/.

use std::path::PathBuf;
use wintube_core::{AppSettings, HistoryEntry};

pub struct Db {
    dir: PathBuf,
}

impl Db {
    pub fn open(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let p = PathBuf::from(path);
        let dir = p.parent().map(|x| x.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn history_path(&self) -> PathBuf { self.dir.join("history.json") }
    fn settings_path(&self) -> PathBuf { self.dir.join("settings.json") }

    pub fn save_history(&self, e: &HistoryEntry) -> Result<(), Box<dyn std::error::Error>> {
        let mut h = self.load_history(500).unwrap_or_default();
        h.retain(|x| x.video_id != e.video_id);
        h.insert(0, e.clone());
        std::fs::write(self.history_path(), serde_json::to_string_pretty(&h)?)?;
        Ok(())
    }

    pub fn load_history(&self, limit: i64) -> Result<Vec<HistoryEntry>, Box<dyn std::error::Error>> {
        let raw = std::fs::read_to_string(self.history_path()).unwrap_or_else(|_| "[]".into());
        let mut v: Vec<HistoryEntry> = serde_json::from_str(&raw).unwrap_or_default();
        v.truncate(limit.max(0) as usize);
        Ok(v)
    }

    pub fn save_settings(&self, s: &AppSettings) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::write(self.settings_path(), serde_json::to_string_pretty(s)?)?;
        Ok(())
    }
    pub fn load_settings(&self) -> AppSettings {
        std::fs::read_to_string(self.settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
}
