use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

/// Internal metadata for the task store, persisted as `_meta.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Meta {
    /// The next sequential ID to allocate.
    pub next_id: u32,
    /// Schema version (for future migrations).
    pub version: u32,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            next_id: 1,
            version: 1,
        }
    }
}

impl Meta {
    /// Returns the meta file path for a given tasks directory.
    pub(crate) fn path(tasks_dir: &Path) -> PathBuf {
        tasks_dir.join("_meta.json")
    }

    /// Load meta from disk, or create a default if it doesn't exist.
    pub(crate) async fn load(tasks_dir: &Path) -> Result<Self> {
        let path = Self::path(tasks_dir);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("reading {}", path.display()))?;
            let meta: Meta = serde_json::from_str(&content)
                .with_context(|| format!("parsing {}", path.display()))?;
            Ok(meta)
        } else {
            Ok(Meta::default())
        }
    }

    /// Save meta to disk.
    pub(crate) async fn save(&self, tasks_dir: &Path) -> Result<()> {
        let path = Self::path(tasks_dir);
        let content = serde_json::to_string_pretty(self).with_context(|| "serializing meta")?;
        tokio::fs::write(&path, content)
            .await
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// Format the next ID as `TSK-XXX` (zero-padded to at least 3 digits).
    pub(crate) fn format_id(id: u32) -> String {
        if id < 1000 {
            format!("TSK-{id:03}")
        } else {
            format!("TSK-{id}")
        }
    }
}

/// Thread-safe ID generator backed by the meta file.
pub(crate) struct MetaIdGenerator {
    meta: Mutex<Meta>,
    tasks_dir: PathBuf,
}

impl MetaIdGenerator {
    /// Create a new generator, loading or initializing meta from disk.
    pub async fn new(tasks_dir: PathBuf) -> Result<Self> {
        // Ensure the tasks directory exists
        tokio::fs::create_dir_all(&tasks_dir)
            .await
            .with_context(|| format!("creating tasks directory {}", tasks_dir.display()))?;

        let meta = Meta::load(&tasks_dir).await?;
        Ok(Self {
            meta: Mutex::new(meta),
            tasks_dir,
        })
    }

    /// Allocate the next ID, persisting the updated meta atomically.
    /// Returns the formatted task ID string (e.g., "TSK-001").
    pub async fn next_id(&self) -> Result<String> {
        let mut meta = self.meta.lock().await;
        let id = meta.next_id;
        meta.next_id += 1;

        // Persist the updated meta
        meta.save(&self.tasks_dir).await?;

        Ok(Meta::format_id(id))
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_default_values() {
        let meta = Meta::default();
        assert_eq!(meta.next_id, 1);
        assert_eq!(meta.version, 1);
    }

    #[test]
    fn meta_format_id_small_numbers() {
        assert_eq!(Meta::format_id(1), "TSK-001");
        assert_eq!(Meta::format_id(10), "TSK-010");
        assert_eq!(Meta::format_id(100), "TSK-100");
        assert_eq!(Meta::format_id(999), "TSK-999");
    }

    #[test]
    fn meta_format_id_large_numbers() {
        assert_eq!(Meta::format_id(1000), "TSK-1000");
        assert_eq!(Meta::format_id(12345), "TSK-12345");
    }

    #[test]
    fn meta_serde_roundtrip() {
        let meta = Meta {
            next_id: 42,
            version: 1,
        };
        let json = serde_json::to_string(&meta).expect("serialize");
        let back: Meta = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(meta.next_id, back.next_id);
        assert_eq!(meta.version, back.version);
    }

    #[tokio::test]
    async fn meta_load_creates_default_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");

        // Directory doesn't exist yet
        tokio::fs::create_dir_all(&tasks_dir)
            .await
            .expect("create dir");

        let meta = Meta::load(&tasks_dir).await.expect("load");
        assert_eq!(meta.next_id, 1);
        assert_eq!(meta.version, 1);
    }

    #[tokio::test]
    async fn meta_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        tokio::fs::create_dir_all(&tasks_dir)
            .await
            .expect("create dir");

        let meta = Meta {
            next_id: 10,
            version: 1,
        };
        meta.save(&tasks_dir).await.expect("save");

        let loaded = Meta::load(&tasks_dir).await.expect("load");
        assert_eq!(loaded.next_id, 10);
        assert_eq!(loaded.version, 1);
    }

    #[tokio::test]
    async fn meta_save_creates_valid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        tokio::fs::create_dir_all(&tasks_dir)
            .await
            .expect("create dir");

        let meta = Meta {
            next_id: 5,
            version: 1,
        };
        meta.save(&tasks_dir).await.expect("save");

        let content = tokio::fs::read_to_string(tasks_dir.join("_meta.json"))
            .await
            .expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse");
        assert_eq!(parsed["next_id"], 5);
        assert_eq!(parsed["version"], 1);
    }

    #[tokio::test]
    async fn id_generator_sequential_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");

        let gen = MetaIdGenerator::new(tasks_dir)
            .await
            .expect("create generator");

        let id1 = gen.next_id().await.expect("id1");
        let id2 = gen.next_id().await.expect("id2");
        let id3 = gen.next_id().await.expect("id3");

        assert_eq!(id1, "TSK-001");
        assert_eq!(id2, "TSK-002");
        assert_eq!(id3, "TSK-003");
    }

    #[tokio::test]
    async fn id_generator_persists_counter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");

        // First generator allocates two IDs
        let gen1 = MetaIdGenerator::new(tasks_dir.clone()).await.expect("gen1");
        gen1.next_id().await.expect("id1");
        gen1.next_id().await.expect("id2");

        // Second generator should continue from where we left off
        let gen2 = MetaIdGenerator::new(tasks_dir).await.expect("gen2");
        let id3 = gen2.next_id().await.expect("id3");
        assert_eq!(id3, "TSK-003");
    }

    #[tokio::test]
    async fn id_generator_creates_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("nested").join("tasks");

        assert!(!tasks_dir.exists());

        let gen = MetaIdGenerator::new(tasks_dir.clone())
            .await
            .expect("create");
        let id = gen.next_id().await.expect("id");

        assert!(tasks_dir.exists());
        assert_eq!(id, "TSK-001");
    }

    #[tokio::test]
    async fn id_generator_concurrent_allocation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");

        let gen = std::sync::Arc::new(MetaIdGenerator::new(tasks_dir).await.expect("create"));

        // Spawn 10 concurrent tasks that each allocate one ID
        let mut handles = vec![];
        for _ in 0..10 {
            let gen_clone = std::sync::Arc::clone(&gen);
            handles.push(tokio::spawn(async move {
                gen_clone.next_id().await.expect("id")
            }));
        }

        let mut ids: Vec<String> = vec![];
        for handle in handles {
            ids.push(handle.await.expect("join"));
        }

        // All IDs should be unique
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "all IDs should be unique");

        // IDs should be TSK-001 through TSK-010
        let expected: Vec<String> = (1..=10).map(Meta::format_id).collect();
        let mut ids_sorted = ids;
        ids_sorted.sort();
        assert_eq!(ids_sorted, expected);
    }
}
