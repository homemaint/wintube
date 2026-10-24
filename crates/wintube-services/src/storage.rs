use std::path::Path;
use std::sync::Arc;
use chrono::Utc;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use tracing::info;

const TABLE_HISTORY: TableDefinition<&str, &str> = TableDefinition::new("watch_history");
const TABLE_SUBSCRIPTIONS: TableDefinition<&str, &str> = TableDefinition::new("local_subscriptions");
const TABLE_SETTINGS: TableDefinition<&str, &str> = TableDefinition::new("user_settings");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchHistoryItem {
    pub video_id: String,
    pub title: String,
    pub channel_id: String,
    pub channel_name: String,
    pub duration_seconds: i64,
    pub watched_seconds: i64,
    pub completed: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSubscription {
    pub channel_id: String,
    pub channel_name: String,
    pub avatar_url: Option<String>,
    pub subscribed_at: i64,
}

/// 100% Pure Rust, crash-safe, zero-copy embedded database (powered by redb).
/// Manages watch history, resume timestamps, and subscriptions without external DLLs or tracking.
#[derive(Clone)]
pub struct StorageEngine {
    db: Arc<Database>,
}

impl StorageEngine {
    pub fn new<P: AsRef<Path>>(db_path: P) -> anyhow::Result<Self> {
        let db = Database::create(db_path)?;
        let engine = Self { db: Arc::new(db) };
        engine.init_tables()?;
        Ok(engine)
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        let db = Database::builder().create_with_backend(redb::backends::InMemoryBackend::new())?;
        let engine = Self { db: Arc::new(db) };
        engine.init_tables()?;
        Ok(engine)
    }

    fn init_tables(&self) -> anyhow::Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let _ = write_txn.open_table(TABLE_HISTORY)?;
            let _ = write_txn.open_table(TABLE_SUBSCRIPTIONS)?;
            let _ = write_txn.open_table(TABLE_SETTINGS)?;
        }
        write_txn.commit()?;
        info!("Redb storage tables initialized successfully");
        Ok(())
    }

    pub fn update_playback_progress(
        &self,
        video_id: &str,
        title: &str,
        channel_id: &str,
        channel_name: &str,
        duration: i64,
        position: i64,
    ) -> anyhow::Result<()> {
        let now = Utc::now().timestamp();
        let completed = duration > 0 && position >= (duration - 15);

        let item = WatchHistoryItem {
            video_id: video_id.to_string(),
            title: title.to_string(),
            channel_id: channel_id.to_string(),
            channel_name: channel_name.to_string(),
            duration_seconds: duration,
            watched_seconds: position,
            completed,
            updated_at: now,
        };

        let json = serde_json::to_string(&item)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_HISTORY)?;
            table.insert(video_id, json.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_resume_position(&self, video_id: &str) -> anyhow::Result<Option<i64>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE_HISTORY)?;
        if let Some(val) = table.get(video_id)? {
            let item: WatchHistoryItem = serde_json::from_str(val.value())?;
            if item.completed {
                Ok(Some(0))
            } else {
                Ok(Some(item.watched_seconds))
            }
        } else {
            Ok(None)
        }
    }

    pub fn get_recent_history(&self, limit: usize) -> anyhow::Result<Vec<WatchHistoryItem>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE_HISTORY)?;

        let mut items = Vec::new();
        for res in table.iter()? {
            let (_k, v) = res?;
            if let Ok(item) = serde_json::from_str::<WatchHistoryItem>(v.value()) {
                items.push(item);
            }
        }

        // Sort descending by updated_at
        items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        items.truncate(limit);
        Ok(items)
    }

    pub fn add_subscription(&self, channel_id: &str, channel_name: &str, avatar_url: Option<&str>) -> anyhow::Result<()> {
        let now = Utc::now().timestamp();
        let sub = LocalSubscription {
            channel_id: channel_id.to_string(),
            channel_name: channel_name.to_string(),
            avatar_url: avatar_url.map(|s| s.to_string()),
            subscribed_at: now,
        };

        let json = serde_json::to_string(&sub)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_SUBSCRIPTIONS)?;
            table.insert(channel_id, json.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn remove_subscription(&self, channel_id: &str) -> anyhow::Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TABLE_SUBSCRIPTIONS)?;
            table.remove(channel_id)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_subscriptions(&self) -> anyhow::Result<Vec<LocalSubscription>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TABLE_SUBSCRIPTIONS)?;

        let mut subs = Vec::new();
        for res in table.iter()? {
            let (_k, v) = res?;
            if let Ok(sub) = serde_json::from_str::<LocalSubscription>(v.value()) {
                subs.push(sub);
            }
        }

        subs.sort_by(|a, b| a.channel_name.to_lowercase().cmp(&b.channel_name.to_lowercase()));
        Ok(subs)
    }
}
