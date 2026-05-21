use crate::store::TaskStore;
use crate::task::{NewTask, TaskFilter, TaskUpdate};
use anyhow::Result;

use super::formatting::format_task_detail;

// ── TaskCommand ──────────────────────────────────────────────────────────────

/// Parsed task slash command.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskCommand {
    /// Create a new task with the given title.
    Create { title: String },
    /// List all tasks.
    List,
    /// Show full details for a specific task.
    Show { id: String },
    /// Move a task to a new state.
    Move { id: String, state: String },
    /// Shorthand for moving a task to "done".
    Done { id: String },
    /// Add a blocking relationship.
    Block { id: String, blocked_by: String },
    /// Remove a blocking relationship.
    Unblock { id: String, blocked_by: String },
    /// Add a label to a task.
    Label { id: String, label: String },
    /// Assign a task to someone.
    Assign { id: String, assignee: String },
}

// ── Handle ───────────────────────────────────────────────────────────────────

/// Execute a parsed `TaskCommand` against a `TaskStore` and return a formatted
/// output string.
pub async fn handle_task_command(cmd: &TaskCommand, store: &dyn TaskStore) -> Result<String> {
    match cmd {
        TaskCommand::Create { title } => {
            let input = NewTask::new(title.clone());
            let task = store.create(&input).await?;
            Ok(format!("Created {}: \"{}\"", task.id, task.title))
        }
        TaskCommand::List => {
            let tasks = store.list(&TaskFilter::default()).await?;
            if tasks.is_empty() {
                return Ok("No tasks found.".to_string());
            }
            let mut lines = Vec::with_capacity(tasks.len());
            for task in &tasks {
                lines.push(format!("{} [{}] {}", task.id, task.state, task.title));
            }
            Ok(lines.join("\n"))
        }
        TaskCommand::Show { id } => {
            let task = store.get(id).await?;
            match task {
                Some(t) => Ok(format_task_detail(&t)),
                None => Ok(format!("Task {id} not found.")),
            }
        }
        TaskCommand::Move { id, state } => {
            let existing = store.get(id).await?;
            match existing {
                Some(ref t) => {
                    let old_state = t.state.clone();
                    let update = TaskUpdate {
                        state: Some(state.clone()),
                        ..Default::default()
                    };
                    match store.update(id, &update).await? {
                        Some(_) => Ok(format!("Transitioned {id}: {old_state} → {state}")),
                        None => Ok(format!("Task {id} not found during update.")),
                    }
                }
                None => Ok(format!("Task {id} not found.")),
            }
        }
        TaskCommand::Done { id } => {
            let existing = store.get(id).await?;
            match existing {
                Some(ref t) => {
                    let old_state = t.state.clone();
                    let update = TaskUpdate {
                        state: Some("done".to_string()),
                        ..Default::default()
                    };
                    match store.update(id, &update).await? {
                        Some(_) => Ok(format!("Transitioned {id}: {old_state} → done")),
                        None => Ok(format!("Task {id} not found during update.")),
                    }
                }
                None => Ok(format!("Task {id} not found.")),
            }
        }
        TaskCommand::Block { id, blocked_by } => {
            let existing = store.get(id).await?;
            match existing {
                Some(ref t) => {
                    // Validate that the blocker task exists
                    if store.get(blocked_by).await?.is_none() {
                        return Ok(format!(
                            "Cannot block: blocker task {blocked_by} does not exist."
                        ));
                    }
                    let mut blockers = t.blocked_by.clone();
                    if blockers.contains(&blocked_by.clone()) {
                        return Ok(format!("{id} is already blocked by {blocked_by}"));
                    }
                    blockers.push(blocked_by.clone());
                    let update = TaskUpdate {
                        blocked_by: Some(blockers),
                        ..Default::default()
                    };
                    store.update(id, &update).await?;
                    Ok(format!("{id} now blocked by {blocked_by}"))
                }
                None => Ok(format!("Task {id} not found.")),
            }
        }
        TaskCommand::Unblock { id, blocked_by } => {
            let existing = store.get(id).await?;
            match existing {
                Some(ref t) => {
                    let mut blockers = t.blocked_by.clone();
                    if !blockers.contains(&blocked_by.clone()) {
                        return Ok(format!("{id} is not blocked by {blocked_by}"));
                    }
                    blockers.retain(|b| b != blocked_by);
                    let update = TaskUpdate {
                        blocked_by: Some(blockers),
                        ..Default::default()
                    };
                    store.update(id, &update).await?;
                    Ok(format!("{id} no longer blocked by {blocked_by}"))
                }
                None => Ok(format!("Task {id} not found.")),
            }
        }
        TaskCommand::Label { id, label } => {
            let existing = store.get(id).await?;
            match existing {
                Some(ref t) => {
                    let mut labels = t.labels.clone();
                    if labels.contains(&label.clone()) {
                        return Ok(format!("{id} already has label '{label}'"));
                    }
                    labels.push(label.clone());
                    let update = TaskUpdate {
                        labels: Some(labels),
                        ..Default::default()
                    };
                    store.update(id, &update).await?;
                    Ok(format!("Added label '{label}' to {id}"))
                }
                None => Ok(format!("Task {id} not found.")),
            }
        }
        TaskCommand::Assign { id, assignee } => {
            let update = TaskUpdate {
                assignee: Some(Some(assignee.clone())),
                ..Default::default()
            };
            match store.update(id, &update).await? {
                Some(_) => Ok(format!("Assigned {id} to {assignee}")),
                None => Ok(format!("Task {id} not found.")),
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::NewTask;

    /// Helper to create a fresh JsonTaskStore in a temp directory.
    async fn setup_store() -> (tempfile::TempDir, crate::store::JsonTaskStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let store = crate::store::JsonTaskStore::new(tasks_dir)
            .await
            .expect("create store");
        (dir, store)
    }

    #[tokio::test]
    async fn handle_create_task() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::Create {
            title: "Add logging".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("Created TSK-001"));
        assert!(output.contains("Add logging"));
    }

    #[tokio::test]
    async fn handle_list_empty() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::List;
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert_eq!(output, "No tasks found.");
    }

    #[tokio::test]
    async fn handle_list_with_tasks() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task 1")).await.unwrap();
        store.create(&NewTask::new("Task 2")).await.unwrap();

        let cmd = TaskCommand::List;
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("TSK-001 [todo] Task 1"));
        assert!(output.contains("TSK-002 [todo] Task 2"));
    }

    #[tokio::test]
    async fn handle_show_existing() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("My task")).await.unwrap();

        let cmd = TaskCommand::Show {
            id: "TSK-001".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("TSK-001"));
        assert!(output.contains("My task"));
        assert!(output.contains("todo"));
    }

    #[tokio::test]
    async fn handle_show_not_found() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::Show {
            id: "TSK-999".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("not found"));
    }

    #[tokio::test]
    async fn handle_move_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let cmd = TaskCommand::Move {
            id: "TSK-001".to_string(),
            state: "in_progress".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("Transitioned TSK-001"));
        assert!(output.contains("todo → in_progress"));
    }

    #[tokio::test]
    async fn handle_move_task_invalid_transition() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let cmd = TaskCommand::Move {
            id: "TSK-001".to_string(),
            state: "done".to_string(),
        };
        let result = handle_task_command(&cmd, &store).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid transition"));
    }

    #[tokio::test]
    async fn handle_move_task_not_found() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::Move {
            id: "TSK-999".to_string(),
            state: "in_progress".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("not found"));
    }

    #[tokio::test]
    async fn handle_done_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();
        // Move to in_progress first (valid transition)
        store
            .update(
                "TSK-001",
                &TaskUpdate {
                    state: Some("in_progress".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let cmd = TaskCommand::Done {
            id: "TSK-001".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("Transitioned TSK-001"));
        assert!(output.contains("in_progress → done"));
    }

    #[tokio::test]
    async fn handle_done_task_not_found() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::Done {
            id: "TSK-999".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("not found"));
    }

    #[tokio::test]
    async fn handle_block_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Blocker")).await.unwrap();
        store.create(&NewTask::new("Blocked")).await.unwrap();

        let cmd = TaskCommand::Block {
            id: "TSK-002".to_string(),
            blocked_by: "TSK-001".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert_eq!(output, "TSK-002 now blocked by TSK-001");
    }

    #[tokio::test]
    async fn handle_block_already_blocked() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Blocker")).await.unwrap();
        store.create(&NewTask::new("Blocked")).await.unwrap();

        // Block first
        let cmd = TaskCommand::Block {
            id: "TSK-002".to_string(),
            blocked_by: "TSK-001".to_string(),
        };
        handle_task_command(&cmd, &store).await.unwrap();

        // Block again
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert_eq!(output, "TSK-002 is already blocked by TSK-001");
    }

    #[tokio::test]
    async fn handle_block_not_found() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::Block {
            id: "TSK-999".to_string(),
            blocked_by: "TSK-001".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("not found"));
    }

    #[tokio::test]
    async fn handle_unblock_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Blocker")).await.unwrap();
        store.create(&NewTask::new("Blocked")).await.unwrap();

        // Block first
        let block_cmd = TaskCommand::Block {
            id: "TSK-002".to_string(),
            blocked_by: "TSK-001".to_string(),
        };
        handle_task_command(&block_cmd, &store).await.unwrap();

        // Unblock
        let unblock_cmd = TaskCommand::Unblock {
            id: "TSK-002".to_string(),
            blocked_by: "TSK-001".to_string(),
        };
        let output = handle_task_command(&unblock_cmd, &store).await.unwrap();
        assert_eq!(output, "TSK-002 no longer blocked by TSK-001");
    }

    #[tokio::test]
    async fn handle_unblock_not_blocked() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let cmd = TaskCommand::Unblock {
            id: "TSK-001".to_string(),
            blocked_by: "TSK-002".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert_eq!(output, "TSK-001 is not blocked by TSK-002");
    }

    #[tokio::test]
    async fn handle_label_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let cmd = TaskCommand::Label {
            id: "TSK-001".to_string(),
            label: "backend".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert_eq!(output, "Added label 'backend' to TSK-001");
    }

    #[tokio::test]
    async fn handle_label_duplicate() {
        let (_dir, store) = setup_store().await;
        let input = NewTask {
            title: "Task".to_string(),
            labels: Some(vec!["backend".to_string()]),
            ..Default::default()
        };
        store.create(&input).await.unwrap();

        let cmd = TaskCommand::Label {
            id: "TSK-001".to_string(),
            label: "backend".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert_eq!(output, "TSK-001 already has label 'backend'");
    }

    #[tokio::test]
    async fn handle_label_not_found() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::Label {
            id: "TSK-999".to_string(),
            label: "test".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("not found"));
    }

    #[tokio::test]
    async fn handle_assign_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let cmd = TaskCommand::Assign {
            id: "TSK-001".to_string(),
            assignee: "alice".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert_eq!(output, "Assigned TSK-001 to alice");
    }

    #[tokio::test]
    async fn handle_assign_not_found() {
        let (_dir, store) = setup_store().await;
        let cmd = TaskCommand::Assign {
            id: "TSK-999".to_string(),
            assignee: "alice".to_string(),
        };
        let output = handle_task_command(&cmd, &store).await.unwrap();
        assert!(output.contains("not found"));
    }

    #[tokio::test]
    async fn handle_create_and_list_and_show() {
        let (_dir, store) = setup_store().await;

        // Create
        let create_cmd = TaskCommand::Create {
            title: "Implement auth".to_string(),
        };
        let output = handle_task_command(&create_cmd, &store).await.unwrap();
        assert!(output.contains("Created TSK-001"));

        // List
        let list_cmd = TaskCommand::List;
        let output = handle_task_command(&list_cmd, &store).await.unwrap();
        assert!(output.contains("TSK-001 [todo] Implement auth"));

        // Show
        let show_cmd = TaskCommand::Show {
            id: "TSK-001".to_string(),
        };
        let output = handle_task_command(&show_cmd, &store).await.unwrap();
        assert!(output.contains("Implement auth"));
        assert!(output.contains("todo"));
    }
}
