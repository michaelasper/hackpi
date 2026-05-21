use anyhow::{Context, Result};

use crate::store::json_store::JsonTaskStore;
use crate::task::{Task, TaskFilter};

impl JsonTaskStore {
    /// Check if a task matches the given filter.
    pub(crate) fn matches_filter(task: &Task, filter: &TaskFilter) -> bool {
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
    pub(crate) async fn scan_all_tasks(&self) -> Result<Vec<Task>> {
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
