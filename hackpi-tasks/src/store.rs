use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use tracing;

use crate::meta::MetaIdGenerator;
use crate::task::{NewTask, Task, TaskFilter, TaskPriority, TaskUpdate};

// ── TaskStore Trait ─────────────────────────────────────────────────────────

/// Async trait for task persistence backends.
#[async_trait]
pub trait TaskStore: Send + Sync {
    /// Create a new task from the given input.
    async fn create(&self, input: &NewTask) -> Result<Task>;

    /// Retrieve a task by its ID (e.g., "TSK-001").
    async fn get(&self, id: &str) -> Result<Option<Task>>;

    /// Update a task with the given partial update.
    async fn update(&self, id: &str, update: &TaskUpdate) -> Result<Option<Task>>;

    /// Delete a task by its ID.
    async fn delete(&self, id: &str) -> Result<bool>;

    /// List tasks matching the given filter.
    async fn list(&self, filter: &TaskFilter) -> Result<Vec<Task>>;

    /// Get the tasks that block the given task (i.e., tasks whose IDs are in
    /// this task's `blocked_by` list).
    async fn blocked_by(&self, id: &str) -> Result<Vec<Task>>;

    /// Get the tasks that this task is blocking (i.e., tasks whose
    /// `blocked_by` contains this task's ID).
    async fn blocking(&self, id: &str) -> Result<Vec<Task>>;
}

// ── JsonTaskStore ───────────────────────────────────────────────────────────

/// File-backed task store that reads/writes individual JSON files in
/// `.hackpi/tasks/TSK-*.json` format.
pub struct JsonTaskStore {
    tasks_dir: PathBuf,
    id_gen: MetaIdGenerator,
}

impl JsonTaskStore {
    /// Create a new `JsonTaskStore` rooted at the given tasks directory.
    ///
    /// The directory (and any parent directories) will be created if they
    /// don't already exist.
    pub async fn new(tasks_dir: PathBuf) -> Result<Self> {
        let id_gen = MetaIdGenerator::new(tasks_dir.clone()).await?;
        Ok(Self { tasks_dir, id_gen })
    }

    /// Build the file path for a given task ID.
    fn task_path(&self, id: &str) -> PathBuf {
        self.tasks_dir.join(format!("{id}.json"))
    }

    /// Atomically write a task file: write to a temp file first, then rename.
    async fn write_task_file(&self, task: &Task) -> Result<()> {
        let target = self.task_path(&task.id);
        let content = serde_json::to_string_pretty(task).with_context(|| "serializing task")?;

        // Write to a temp file in the same directory
        let temp_name = format!(".{}.tmp", task.id);
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
    fn apply_update(task: &mut Task, update: &TaskUpdate) {
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

    /// Check if a task matches the given filter.
    fn matches_filter(task: &Task, filter: &TaskFilter) -> bool {
        if let Some(ref state) = filter.state {
            if task.state != *state {
                return false;
            }
        }
        if let Some(priority) = filter.priority {
            if task.priority != priority {
                return false;
            }
        }
        if let Some(ref labels) = filter.labels {
            if !labels.iter().all(|label| task.labels.contains(label)) {
                return false;
            }
        }
        if let Some(ref assignee) = filter.assignee {
            match &task.assignee {
                Some(a) if a == assignee => {}
                _ => return false,
            }
        }
        if let Some(ref workflow) = filter.workflow {
            if task.workflow != *workflow {
                return false;
            }
        }
        true
    }

    /// Scan all task files in the tasks directory.
    async fn scan_all_tasks(&self) -> Result<Vec<Task>> {
        let mut tasks = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.tasks_dir)
            .await
            .with_context(|| format!("reading directory {}", self.tasks_dir.display()))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| "reading directory entry")?
        {
            let path = entry.path();

            // Only process TSK-*.json files, skip _meta.json and temp files
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("TSK-") || !name.ends_with(".json") {
                continue;
            }

            let content = tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("reading {}", path.display()))?;
            let task: Task = serde_json::from_str(&content)
                .with_context(|| format!("parsing {}", path.display()))?;
            tasks.push(task);
        }

        // Sort by ID for deterministic ordering
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }
}

#[async_trait]
impl TaskStore for JsonTaskStore {
    async fn create(&self, input: &NewTask) -> Result<Task> {
        let id = self.id_gen.next_id().await?;
        let now = chrono::Utc::now();

        let task = Task {
            id,
            title: input.title.clone(),
            description: input.description.clone().unwrap_or_default(),
            state: "todo".to_string(),
            priority: input.priority.unwrap_or(TaskPriority::None),
            workflow: input
                .workflow
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            blocked_by: Vec::new(),
            labels: input.labels.clone().unwrap_or_default(),
            assignee: input.assignee.clone(),
            created_at: now,
            updated_at: now,
        };

        self.write_task_file(&task).await?;
        tracing::debug!(task_id = %task.id, "created task");
        Ok(task)
    }

    async fn get(&self, id: &str) -> Result<Option<Task>> {
        let path = self.task_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("reading task {id}"))?;
        let task: Task =
            serde_json::from_str(&content).with_context(|| format!("parsing task {id}"))?;
        Ok(Some(task))
    }

    async fn update(&self, id: &str, update: &TaskUpdate) -> Result<Option<Task>> {
        let mut task = match self.get(id).await? {
            Some(t) => t,
            None => return Ok(None),
        };

        Self::apply_update(&mut task, update);
        self.write_task_file(&task).await?;
        tracing::debug!(task_id = %task.id, "updated task");
        Ok(Some(task))
    }

    async fn delete(&self, id: &str) -> Result<bool> {
        let path = self.task_path(id);
        if !path.exists() {
            return Ok(false);
        }
        tokio::fs::remove_file(&path)
            .await
            .with_context(|| format!("deleting task {id}"))?;
        tracing::debug!(task_id = id, "deleted task");
        Ok(true)
    }

    async fn list(&self, filter: &TaskFilter) -> Result<Vec<Task>> {
        let all_tasks = self.scan_all_tasks().await?;
        let filtered = all_tasks
            .into_iter()
            .filter(|t| Self::matches_filter(t, filter))
            .collect();
        Ok(filtered)
    }

    async fn blocked_by(&self, id: &str) -> Result<Vec<Task>> {
        let task = match self.get(id).await? {
            Some(t) => t,
            None => return Ok(Vec::new()),
        };

        let mut blockers = Vec::new();
        for blocker_id in &task.blocked_by {
            if let Some(blocker) = self.get(blocker_id).await? {
                blockers.push(blocker);
            }
        }
        Ok(blockers)
    }

    async fn blocking(&self, id: &str) -> Result<Vec<Task>> {
        let all_tasks = self.scan_all_tasks().await?;
        let blocking: Vec<Task> = all_tasks
            .into_iter()
            .filter(|t| t.blocked_by.contains(&id.to_string()))
            .collect();
        Ok(blocking)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a fresh JsonTaskStore in a temp directory.
    async fn setup_store() -> (tempfile::TempDir, JsonTaskStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let store = JsonTaskStore::new(tasks_dir).await.expect("create store");
        (dir, store)
    }

    // ── Create Tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_basic_task() {
        let (_dir, store) = setup_store().await;
        let input = NewTask::new("My first task");
        let task = store.create(&input).await.expect("create");

        assert_eq!(task.id, "TSK-001");
        assert_eq!(task.title, "My first task");
        assert_eq!(task.description, "");
        assert_eq!(task.state, "todo");
        assert_eq!(task.priority, TaskPriority::None);
        assert_eq!(task.workflow, "default");
        assert!(task.blocked_by.is_empty());
        assert!(task.labels.is_empty());
        assert!(task.assignee.is_none());
        assert!(task.created_at <= chrono::Utc::now());
        assert!(task.updated_at <= chrono::Utc::now());
    }

    #[tokio::test]
    async fn create_task_with_all_fields() {
        let (_dir, store) = setup_store().await;
        let input = NewTask {
            title: "Complex task".to_string(),
            description: Some("With details".to_string()),
            priority: Some(TaskPriority::High),
            workflow: Some("kanban".to_string()),
            labels: Some(vec!["backend".to_string()]),
            assignee: Some("alice".to_string()),
        };
        let task = store.create(&input).await.expect("create");

        assert_eq!(task.title, "Complex task");
        assert_eq!(task.description, "With details");
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.workflow, "kanban");
        assert_eq!(task.labels, vec!["backend"]);
        assert_eq!(task.assignee, Some("alice".to_string()));
    }

    #[tokio::test]
    async fn create_multiple_tasks_sequential_ids() {
        let (_dir, store) = setup_store().await;

        let t1 = store.create(&NewTask::new("Task 1")).await.expect("create");
        let t2 = store.create(&NewTask::new("Task 2")).await.expect("create");
        let t3 = store.create(&NewTask::new("Task 3")).await.expect("create");

        assert_eq!(t1.id, "TSK-001");
        assert_eq!(t2.id, "TSK-002");
        assert_eq!(t3.id, "TSK-003");
    }

    #[tokio::test]
    async fn create_persists_to_disk() {
        let (_dir, store) = setup_store().await;
        let task = store
            .create(&NewTask::new("Persisted"))
            .await
            .expect("create");

        let path = store.task_path(&task.id);
        assert!(path.exists(), "task file should exist on disk");

        let content = tokio::fs::read_to_string(&path).await.expect("read");
        let loaded: Task = serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded.title, "Persisted");
    }

    // ── Get Tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_existing_task() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Get me")).await.expect("create");

        let found = store.get(&created.id).await.expect("get");
        assert!(found.is_some());
        let task = found.expect("task");
        assert_eq!(task.title, "Get me");
        assert_eq!(task.id, created.id);
    }

    #[tokio::test]
    async fn get_nonexistent_task() {
        let (_dir, store) = setup_store().await;
        let result = store.get("TSK-999").await.expect("get");
        assert!(result.is_none());
    }

    // ── Update Tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn update_title() {
        let (_dir, store) = setup_store().await;
        let created = store
            .create(&NewTask::new("Original"))
            .await
            .expect("create");

        let update = TaskUpdate {
            title: Some("Updated title".to_string()),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");

        assert_eq!(updated.title, "Updated title");
        assert_eq!(updated.description, ""); // unchanged
    }

    #[tokio::test]
    async fn update_state_and_priority() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        let update = TaskUpdate {
            state: Some("in_progress".to_string()),
            priority: Some(TaskPriority::Urgent),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");

        assert_eq!(updated.state, "in_progress");
        assert_eq!(updated.priority, TaskPriority::Urgent);
    }

    #[tokio::test]
    async fn update_assignee_to_some() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        let update = TaskUpdate {
            assignee: Some(Some("bob".to_string())),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");

        assert_eq!(updated.assignee, Some("bob".to_string()));
    }

    #[tokio::test]
    async fn update_assignee_to_none() {
        let (_dir, store) = setup_store().await;
        let input = NewTask {
            title: "Task".to_string(),
            assignee: Some("alice".to_string()),
            ..Default::default()
        };
        let created = store.create(&input).await.expect("create");
        assert_eq!(created.assignee, Some("alice".to_string()));

        let update = TaskUpdate {
            assignee: Some(None),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");

        assert!(updated.assignee.is_none());
    }

    #[tokio::test]
    async fn update_nonexistent_task() {
        let (_dir, store) = setup_store().await;
        let update = TaskUpdate {
            title: Some("Nope".to_string()),
            ..Default::default()
        };
        let result = store.update("TSK-999", &update).await.expect("update");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn update_touches_updated_at() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        // Small delay to ensure timestamps differ
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let update = TaskUpdate {
            title: Some("Updated".to_string()),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");

        assert!(updated.updated_at > created.updated_at);
    }

    #[tokio::test]
    async fn update_empty_is_noop_except_updated_at() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let update = TaskUpdate::default();
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");

        assert_eq!(updated.title, created.title);
        assert_eq!(updated.state, created.state);
        assert!(updated.updated_at > created.updated_at);
    }

    // ── Delete Tests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_existing_task() {
        let (_dir, store) = setup_store().await;
        let created = store
            .create(&NewTask::new("Delete me"))
            .await
            .expect("create");

        let deleted = store.delete(&created.id).await.expect("delete");
        assert!(deleted);

        let found = store.get(&created.id).await.expect("get");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_task() {
        let (_dir, store) = setup_store().await;
        let deleted = store.delete("TSK-999").await.expect("delete");
        assert!(!deleted);
    }

    // ── List Tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_empty_store() {
        let (_dir, store) = setup_store().await;
        let tasks = store.list(&TaskFilter::default()).await.expect("list");
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn list_all_tasks_with_empty_filter() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task 1")).await.expect("create");
        store.create(&NewTask::new("Task 2")).await.expect("create");
        store.create(&NewTask::new("Task 3")).await.expect("create");

        let tasks = store.list(&TaskFilter::default()).await.expect("list");
        assert_eq!(tasks.len(), 3);
    }

    #[tokio::test]
    async fn list_filter_by_state() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Todo 1")).await.expect("create");

        let in_progress = store
            .create(&NewTask::new("In Progress"))
            .await
            .expect("create");
        store
            .update(
                &in_progress.id,
                &TaskUpdate {
                    state: Some("in_progress".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update");

        store.create(&NewTask::new("Todo 2")).await.expect("create");

        let filter = TaskFilter {
            state: Some("in_progress".to_string()),
            ..Default::default()
        };
        let tasks = store.list(&filter).await.expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "In Progress");
    }

    #[tokio::test]
    async fn list_filter_by_priority() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Low")).await.expect("create");

        let input = NewTask {
            title: "Urgent".to_string(),
            priority: Some(TaskPriority::Urgent),
            ..Default::default()
        };
        store.create(&input).await.expect("create");

        let filter = TaskFilter {
            priority: Some(TaskPriority::Urgent),
            ..Default::default()
        };
        let tasks = store.list(&filter).await.expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Urgent");
    }

    #[tokio::test]
    async fn list_filter_by_labels() {
        let (_dir, store) = setup_store().await;

        let input1 = NewTask {
            title: "Backend".to_string(),
            labels: Some(vec!["backend".to_string(), "rust".to_string()]),
            ..Default::default()
        };
        store.create(&input1).await.expect("create");

        let input2 = NewTask {
            title: "Frontend".to_string(),
            labels: Some(vec!["frontend".to_string()]),
            ..Default::default()
        };
        store.create(&input2).await.expect("create");

        // Filter by single label — should match task with both labels
        let filter = TaskFilter {
            labels: Some(vec!["rust".to_string()]),
            ..Default::default()
        };
        let tasks = store.list(&filter).await.expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Backend");
    }

    #[tokio::test]
    async fn list_filter_by_assignee() {
        let (_dir, store) = setup_store().await;

        let input1 = NewTask {
            title: "Alice task".to_string(),
            assignee: Some("alice".to_string()),
            ..Default::default()
        };
        store.create(&input1).await.expect("create");

        let input2 = NewTask {
            title: "Bob task".to_string(),
            assignee: Some("bob".to_string()),
            ..Default::default()
        };
        store.create(&input2).await.expect("create");

        store
            .create(&NewTask::new("Unassigned"))
            .await
            .expect("create");

        let filter = TaskFilter {
            assignee: Some("alice".to_string()),
            ..Default::default()
        };
        let tasks = store.list(&filter).await.expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Alice task");
    }

    #[tokio::test]
    async fn list_filter_by_workflow() {
        let (_dir, store) = setup_store().await;

        let input = NewTask {
            title: "Custom flow".to_string(),
            workflow: Some("kanban".to_string()),
            ..Default::default()
        };
        store.create(&input).await.expect("create");
        store
            .create(&NewTask::new("Default flow"))
            .await
            .expect("create");

        let filter = TaskFilter {
            workflow: Some("kanban".to_string()),
            ..Default::default()
        };
        let tasks = store.list(&filter).await.expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Custom flow");
    }

    #[tokio::test]
    async fn list_combined_filter() {
        let (_dir, store) = setup_store().await;

        let input = NewTask {
            title: "Match".to_string(),
            workflow: Some("kanban".to_string()),
            priority: Some(TaskPriority::High),
            ..Default::default()
        };
        store.create(&input).await.expect("create");

        // Same workflow but different priority — should NOT match
        let input2 = NewTask {
            title: "No match".to_string(),
            workflow: Some("kanban".to_string()),
            priority: Some(TaskPriority::Low),
            ..Default::default()
        };
        store.create(&input2).await.expect("create");

        let filter = TaskFilter {
            workflow: Some("kanban".to_string()),
            priority: Some(TaskPriority::High),
            ..Default::default()
        };
        let tasks = store.list(&filter).await.expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Match");
    }

    // ── Blocked By / Blocking Tests ─────────────────────────────────────

    #[tokio::test]
    async fn blocked_by_returns_blockers() {
        let (_dir, store) = setup_store().await;

        let t1 = store
            .create(&NewTask::new("Blocker 1"))
            .await
            .expect("create");
        let t2 = store
            .create(&NewTask::new("Blocker 2"))
            .await
            .expect("create");

        let input = NewTask::new("Blocked task");
        let t3 = store.create(&input).await.expect("create");

        // Set t3 to be blocked by t1 and t2
        store
            .update(
                &t3.id,
                &TaskUpdate {
                    blocked_by: Some(vec![t1.id.clone(), t2.id.clone()]),
                    ..Default::default()
                },
            )
            .await
            .expect("update");

        let blockers = store.blocked_by(&t3.id).await.expect("blocked_by");
        assert_eq!(blockers.len(), 2);

        let blocker_ids: Vec<&str> = blockers.iter().map(|t| t.id.as_str()).collect();
        assert!(blocker_ids.contains(&t1.id.as_str()));
        assert!(blocker_ids.contains(&t2.id.as_str()));
    }

    #[tokio::test]
    async fn blocked_by_nonexistent_blocker_skipped() {
        let (_dir, store) = setup_store().await;

        let t1 = store.create(&NewTask::new("Task")).await.expect("create");

        // Set blocked_by to include a nonexistent task
        store
            .update(
                &t1.id,
                &TaskUpdate {
                    blocked_by: Some(vec!["TSK-999".to_string()]),
                    ..Default::default()
                },
            )
            .await
            .expect("update");

        let blockers = store.blocked_by(&t1.id).await.expect("blocked_by");
        assert!(
            blockers.is_empty(),
            "nonexistent blockers should be skipped"
        );
    }

    #[tokio::test]
    async fn blocked_by_nonexistent_task_returns_empty() {
        let (_dir, store) = setup_store().await;
        let blockers = store.blocked_by("TSK-999").await.expect("blocked_by");
        assert!(blockers.is_empty());
    }

    #[tokio::test]
    async fn blocking_returns_tasks_blocked_by_this_one() {
        let (_dir, store) = setup_store().await;

        let t1 = store
            .create(&NewTask::new("Blocker"))
            .await
            .expect("create");
        let _t2 = store
            .create(&NewTask::new("Unrelated"))
            .await
            .expect("create");

        let input = NewTask::new("Blocked by t1");
        let t3 = store.create(&input).await.expect("create");
        store
            .update(
                &t3.id,
                &TaskUpdate {
                    blocked_by: Some(vec![t1.id.clone()]),
                    ..Default::default()
                },
            )
            .await
            .expect("update");

        let input2 = NewTask::new("Also blocked by t1");
        let t4 = store.create(&input2).await.expect("create");
        store
            .update(
                &t4.id,
                &TaskUpdate {
                    blocked_by: Some(vec![t1.id.clone()]),
                    ..Default::default()
                },
            )
            .await
            .expect("update");

        let blocking = store.blocking(&t1.id).await.expect("blocking");
        assert_eq!(blocking.len(), 2);

        let blocking_ids: Vec<&str> = blocking.iter().map(|t| t.id.as_str()).collect();
        assert!(blocking_ids.contains(&t3.id.as_str()));
        assert!(blocking_ids.contains(&t4.id.as_str()));
    }

    #[tokio::test]
    async fn blocking_nothing_returns_empty() {
        let (_dir, store) = setup_store().await;
        let t1 = store
            .create(&NewTask::new("Lone wolf"))
            .await
            .expect("create");

        let blocking = store.blocking(&t1.id).await.expect("blocking");
        assert!(blocking.is_empty());
    }

    // ── Atomic Write Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn atomic_write_no_leftover_temp_files() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.expect("create");

        // Ensure no temp files remain
        let mut entries = tokio::fs::read_dir(&store.tasks_dir)
            .await
            .expect("read dir");
        while let Some(entry) = entries.next_entry().await.expect("next") {
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with('.'),
                "no temp files should remain: {name}"
            );
        }
    }

    #[tokio::test]
    async fn task_file_is_valid_json() {
        let (_dir, store) = setup_store().await;
        let task = store
            .create(&NewTask::new("JSON check"))
            .await
            .expect("create");

        let path = store.task_path(&task.id);
        let content = tokio::fs::read_to_string(&path).await.expect("read");

        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse");
        assert_eq!(parsed["id"], "TSK-001");
        assert_eq!(parsed["title"], "JSON check");
    }
}
