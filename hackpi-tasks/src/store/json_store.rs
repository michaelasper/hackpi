use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::meta::MetaIdGenerator;
use crate::task::{Task, TaskUpdate};
use crate::workflow::WorkflowProfile;

/// File-backed task store that reads/writes individual JSON files in
/// `.hackpi/tasks/TSK-*.json` format.
pub struct JsonTaskStore {
    pub(crate) tasks_dir: PathBuf,
    pub(crate) id_gen: MetaIdGenerator,
    /// Loaded workflow profiles, keyed by name. Thread-safe for concurrent
    /// access and hot-reload swaps.
    pub(crate) workflows: Arc<RwLock<HashMap<String, WorkflowProfile>>>,
    /// Per-task locks preventing concurrent read-modify-write races on the
    /// same task file. The map is populated lazily as tasks are accessed.
    pub(crate) task_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// Monotonically increasing counter for unique temp file names, avoiding
    /// collisions when multiple writes to the same task race on the temp path.
    pub(crate) next_temp_id: AtomicU64,
}

impl JsonTaskStore {
    /// Create a new `JsonTaskStore` rooted at the given tasks directory.
    ///
    /// The directory (and any parent directories) will be created if they
    /// don't already exist. The built-in default workflow is included
    /// automatically.
    pub async fn new(tasks_dir: PathBuf) -> Result<Self> {
        let id_gen = MetaIdGenerator::new(tasks_dir.clone()).await?;
        let mut workflows = HashMap::new();
        let default = WorkflowProfile::default_workflow();
        workflows.insert(default.name.clone(), default);
        Ok(Self {
            tasks_dir,
            id_gen,
            workflows: Arc::new(RwLock::new(workflows)),
            task_locks: Arc::new(RwLock::new(HashMap::new())),
            next_temp_id: AtomicU64::new(0),
        })
    }

    /// Create a `JsonTaskStore` with additional workflow profiles loaded from
    /// the given directory. The built-in default workflow is always included.
    pub async fn with_workflows(
        tasks_dir: PathBuf,
        workflows_dir: &std::path::Path,
    ) -> Result<Self> {
        let mut workflows = HashMap::new();
        let default = WorkflowProfile::default_workflow();
        workflows.insert(default.name.clone(), default);

        let loaded = WorkflowProfile::load_from_dir(workflows_dir).await?;
        workflows.extend(loaded);

        let id_gen = MetaIdGenerator::new(tasks_dir.clone()).await?;
        Ok(Self {
            tasks_dir,
            id_gen,
            workflows: Arc::new(RwLock::new(workflows)),
            task_locks: Arc::new(RwLock::new(HashMap::new())),
            next_temp_id: AtomicU64::new(0),
        })
    }

    /// Get the workflow profile for the given name, falling back to the
    /// built-in default if not found.
    pub(crate) async fn get_workflow(&self, name: &str) -> WorkflowProfile {
        let guard = self.workflows.read().await;
        if let Some(wf) = guard.get(name) {
            wf.clone()
        } else {
            // Fall back to built-in default
            tracing::debug!(
                workflow_name = %name,
                "unknown workflow, falling back to default"
            );
            WorkflowProfile::default_workflow()
        }
    }

    /// Reload workflow profiles from the given directory, validating each one
    /// before swapping. Invalid files are logged as warnings and skipped.
    /// The built-in default workflow is always preserved.
    pub async fn reload_workflows(&self, workflows_dir: &std::path::Path) -> Result<()> {
        let loaded = WorkflowProfile::load_from_dir(workflows_dir).await?;

        let mut guard = self.workflows.write().await;
        // Always keep the built-in default
        guard.retain(|name, _| name == "default");
        guard.extend(loaded);

        tracing::info!("Reloaded {} workflow profiles", guard.len());
        Ok(())
    }

    /// Build the file path for a given task ID.
    pub(crate) fn task_path(&self, id: &str) -> PathBuf {
        self.tasks_dir.join(format!("{id}.json"))
    }

    /// Acquire a per-task lock that serializes read-modify-write cycles for
    /// the given task ID. Multiple concurrent calls with the same ID will
    /// synchronize on the same underlying `Mutex`, ensuring exclusive access.
    ///
    /// Uses double-checked locking: reads under a shared `RwLock` guard first,
    /// then acquires the write guard only when a new entry must be inserted.
    pub(crate) async fn task_lock(&self, id: &str) -> Arc<Mutex<()>> {
        let map = self.task_locks.read().await;
        if let Some(lock) = map.get(id) {
            return lock.clone();
        }
        drop(map);

        let mut map = self.task_locks.write().await;
        let entry = map
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())));
        entry.clone()
    }

    /// Atomically write a task file: write to a temp file first, then rename.
    /// The temp file name includes a unique counter to avoid collisions when
    /// concurrent writes target the same task (even with per-task locking, the
    /// temp name uniqueness provides defense-in-depth).
    pub(crate) async fn write_task_file(&self, task: &Task) -> Result<()> {
        let target = self.task_path(&task.id);
        let content = serde_json::to_string_pretty(task).with_context(|| "serializing task")?;

        // Use a unique temp name per write to avoid collisions
        let temp_id = self.next_temp_id.fetch_add(1, Ordering::Relaxed);
        let temp_name = format!(".{}.{}.tmp", task.id, temp_id);
        let temp_path = self.tasks_dir.join(&temp_name);

        tokio::fs::write(&temp_path, &content)
            .await
            .with_context(|| format!("writing temp file {}", temp_path.display()))?;

        // Atomic rename
        tokio::fs::rename(&temp_path, &target)
            .await
            .with_context(|| format!("renaming {} -> {}", temp_path.display(), target.display()))?;

        Ok(())
    }

    /// Apply a `TaskUpdate` to an existing task, returning the updated task.
    pub(crate) fn apply_update(task: &mut Task, update: &TaskUpdate) {
        if let Some(ref title) = update.title {
            task.title = title.clone();
        }
        if let Some(ref description) = update.description {
            task.description = description.clone();
        }
        if let Some(ref state) = update.state {
            task.state = state.clone();
        }
        if let Some(priority) = update.priority {
            task.priority = priority;
        }
        if let Some(ref workflow) = update.workflow {
            task.workflow = workflow.clone();
        }
        if let Some(ref blocked_by) = update.blocked_by {
            task.blocked_by = blocked_by.clone();
        }
        if let Some(ref labels) = update.labels {
            task.labels = labels.clone();
        }
        if let Some(ref assignee) = update.assignee {
            task.assignee = assignee.clone();
        }
        task.updated_at = chrono::Utc::now();
    }
}
