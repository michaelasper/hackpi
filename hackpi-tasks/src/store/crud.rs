use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::store::json_store::JsonTaskStore;
use crate::store::traits::TaskStore;
use crate::task::{NewTask, Task, TaskFilter, TaskPriority, TaskUpdate};

#[async_trait]
impl TaskStore for JsonTaskStore {
    async fn create(&self, input: &NewTask) -> Result<Task> {
        let id = self.id_gen.next_id().await?;
        let now = chrono::Utc::now();
        let workflow_name = input
            .workflow
            .clone()
            .unwrap_or_else(|| "default".to_string());
        let workflow = self.get_workflow(&workflow_name).await;

        let task = Task {
            id,
            title: input.title.clone(),
            description: input.description.clone().unwrap_or_default(),
            state: workflow.initial_state().to_string(),
            priority: input.priority.unwrap_or(TaskPriority::None),
            workflow: workflow_name,
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
        let task_lock = self.task_lock(id).await;
        let _guard = task_lock.lock().await;

        let mut task = match self.get(id).await? {
            Some(t) => t,
            None => return Ok(None),
        };

        // Validate state transition before applying the update
        if let Some(ref new_state) = update.state {
            self.validate_state_transition(&task.state, new_state, &task.workflow)
                .await?;
        }

        Self::apply_update(&mut task, update);
        self.write_task_file(&task).await?;
        tracing::debug!(task_id = %task.id, "updated task");
        Ok(Some(task))
    }

    async fn delete(&self, id: &str) -> Result<bool> {
        let task_lock = self.task_lock(id).await;
        let _guard = task_lock.lock().await;

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
