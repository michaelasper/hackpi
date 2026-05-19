use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

    /// Load meta from disk, or create a default if it doesn't exist or is empty.
    pub(crate) async fn load(tasks_dir: &Path) -> Result<Self> {
        let path = Self::path(tasks_dir);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("reading {}", path.display()))?;
            if content.trim().is_empty() {
                return Ok(Meta::default());
            }
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

    /// Synchronous version of `load` for use inside `spawn_blocking` where
    /// `tokio::fs` is not available.
    pub(crate) fn load_sync(tasks_dir: &Path) -> Result<Self> {
        let path = Self::path(tasks_dir);
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            if content.trim().is_empty() {
                return Ok(Meta::default());
            }
            let meta: Meta = serde_json::from_str(&content)
                .with_context(|| format!("parsing {}", path.display()))?;
            Ok(meta)
        } else {
            Ok(Meta::default())
        }
    }

    /// Synchronous version of `save` for use inside `spawn_blocking` where
    /// `tokio::fs` is not available.
    pub(crate) fn save_sync(&self, tasks_dir: &Path) -> Result<()> {
        let path = Self::path(tasks_dir);
        let content = serde_json::to_string_pretty(self).with_context(|| "serializing meta")?;
        std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
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
///
/// Uses both an in-process `Mutex` and a cross-process file lock (via `fs2`) to
/// guarantee unique ID allocation even when multiple `MetaIdGenerator` instances
/// (potentially in different processes) point at the same directory.
pub(crate) struct MetaIdGenerator {
    /// In-process mutex serializing calls within the same process.
    guard: Mutex<()>,
    tasks_dir: PathBuf,
    /// File handle to `_meta.json`, held open for the lifetime of the generator.
    /// Used with `fs2::FileExt` for cross-process exclusive locking.
    lock_file: Arc<std::fs::File>,
}

impl MetaIdGenerator {
    /// Create a new generator, opening (or creating) `_meta.json` for locking.
    pub async fn new(tasks_dir: PathBuf) -> Result<Self> {
        // Ensure the tasks directory exists
        tokio::fs::create_dir_all(&tasks_dir)
            .await
            .with_context(|| format!("creating tasks directory {}", tasks_dir.display()))?;

        let meta_path = Meta::path(&tasks_dir);

        // Open (or create) the meta file. We keep this handle open for the
        // lifetime of the generator to use as a cross-process lock.
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&meta_path)
            .with_context(|| format!("opening meta file {}", meta_path.display()))?;

        // Ensure the meta file has valid initial content if it was just created
        let meta = Meta::load(&tasks_dir).await?;
        meta.save(&tasks_dir).await?;

        Ok(Self {
            guard: Mutex::new(()),
            tasks_dir,
            lock_file: Arc::new(lock_file),
        })
    }

    /// Allocate the next ID, using a cross-process file lock to guarantee
    /// uniqueness even when multiple generators share the same directory.
    ///
    /// The critical section (read–increment–write) runs inside a
    /// `spawn_blocking` call where the exclusive file lock is held, ensuring
    /// that only one writer touches `_meta.json` at a time globally.
    ///
    /// Returns the formatted task ID string (e.g., "TSK-001").
    pub async fn next_id(&self) -> Result<String> {
        // In-process serialization — no two concurrent calls from the same
        // generator will proceed past this point.
        let _guard = self.guard.lock().await;

        let lock_file = Arc::clone(&self.lock_file);
        let tasks_dir = self.tasks_dir.clone();

        // Cross-process locking + I/O happens on a blocking thread since
        // `fs2::FileExt::lock_exclusive` and `std::fs` operations are
        // synchronous.
        let id = tokio::task::spawn_blocking(move || -> Result<u32> {
            // Acquire cross-process exclusive lock. This will block until the
            // lock is available (no other process holds it).
            lock_file
                .lock_exclusive()
                .with_context(|| "acquiring exclusive lock on _meta.json")?;

            // Re-read meta from disk — another process may have changed it
            // since we last looked.
            let meta = Meta::load_sync(&tasks_dir)?;

            let id = meta.next_id;

            // Write the incremented value back
            let next = Meta {
                next_id: meta.next_id + 1,
                ..meta
            };
            next.save_sync(&tasks_dir)?;

            // Explicitly release the lock so the next waiter can proceed
            // immediately instead of waiting for the file handle to drop.
            lock_file
                .unlock()
                .with_context(|| "releasing exclusive lock on _meta.json")?;

            Ok(id)
        })
        .await
        .with_context(|| "blocking task for meta lock panicked")??;

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
    async fn meta_corrupt_json_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        tokio::fs::create_dir_all(&tasks_dir)
            .await
            .expect("create dir");

        // Write invalid JSON to _meta.json
        tokio::fs::write(tasks_dir.join("_meta.json"), "not valid json {{")
            .await
            .expect("write");

        let result = Meta::load(&tasks_dir).await;
        assert!(result.is_err(), "corrupt meta.json should return an error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("_meta.json"),
            "error should mention the meta file, got: {err}"
        );
    }

    #[tokio::test]
    async fn id_generator_cross_instance_duplicate_prevention() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");

        // Two separate generators pointing at the same directory
        let gen1 = MetaIdGenerator::new(tasks_dir.clone()).await.expect("gen1");
        let gen2 = MetaIdGenerator::new(tasks_dir.clone()).await.expect("gen2");

        // Allocate IDs from both generators concurrently
        let ids1 = gen1.next_id().await.expect("id1");
        let ids2 = gen2.next_id().await.expect("id2");
        let ids3 = gen1.next_id().await.expect("id3");
        let ids4 = gen2.next_id().await.expect("id4");

        let ids = vec![ids1, ids2, ids3, ids4];
        let mut sorted = ids.clone();
        sorted.sort();
        let unique_count = sorted.len();
        sorted.dedup();
        assert_eq!(
            unique_count,
            sorted.len(),
            "all IDs should be unique across separate generators, got: {ids:?}"
        );

        // IDs should match TSK-001 through TSK-004
        let expected: Vec<String> = (1..=4).map(Meta::format_id).collect();
        assert_eq!(
            sorted, expected,
            "IDs should be sequential across generators"
        );
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
