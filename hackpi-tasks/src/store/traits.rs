use anyhow::Result;
use async_trait::async_trait;

use crate::task::{NewTask, Task, TaskFilter, TaskUpdate};

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
