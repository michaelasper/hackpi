use crate::store::TaskStore;
use crate::task::{NewTask, TaskFilter, TaskUpdate};
use anyhow::Result;

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

// ── Parse ────────────────────────────────────────────────────────────────────

/// Parse a slash command input string (after `/task ` or `/tasks`) into a
/// `TaskCommand`.
///
/// # Examples
/// - `"create Add logging"` → `TaskCommand::Create { title: "Add logging" }`
/// - `"list"` → `TaskCommand::List`
/// - `"show TSK-001"` → `TaskCommand::Show { id: "TSK-001" }`
pub fn parse_slash_task_command(input: &str) -> Result<TaskCommand, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Missing task subcommand. Usage: /task <create|list|show|move|done|block|unblock|label|assign> [args]".to_string());
    }

    let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
    let subcommand = parts[0].to_lowercase();
    let rest = parts.get(1).copied().unwrap_or("").trim();

    match subcommand.as_str() {
        "create" => {
            if rest.is_empty() {
                return Err("Missing title. Usage: /task create <title>".to_string());
            }
            Ok(TaskCommand::Create {
                title: rest.to_string(),
            })
        }
        "list" | "ls" => Ok(TaskCommand::List),
        "show" | "get" => {
            let id = parse_task_id(rest)?;
            Ok(TaskCommand::Show { id })
        }
        "move" | "mv" => {
            let (id, state) = parse_id_and_arg(rest, "state")?;
            Ok(TaskCommand::Move {
                id,
                state: state.to_lowercase(),
            })
        }
        "done" | "complete" | "finish" => {
            let id = parse_task_id(rest)?;
            Ok(TaskCommand::Done { id })
        }
        "block" => {
            let (id, blocked_by) = parse_id_and_arg(rest, "blocked_by")?;
            Ok(TaskCommand::Block { id, blocked_by })
        }
        "unblock" => {
            let (id, blocked_by) = parse_id_and_arg(rest, "blocked_by")?;
            Ok(TaskCommand::Unblock { id, blocked_by })
        }
        "label" | "tag" => {
            let (id, label) = parse_id_and_arg(rest, "label")?;
            Ok(TaskCommand::Label { id, label })
        }
        "assign" => {
            let (id, assignee) = parse_id_and_arg(rest, "assignee")?;
            Ok(TaskCommand::Assign { id, assignee })
        }
        other => Err(format!(
            "Unknown task subcommand: '{other}'. Available: create, list, show, move, done, block, unblock, label, assign"
        )),
    }
}

/// Parse a task ID from input, validating it starts with "TSK-".
fn parse_task_id(input: &str) -> Result<String, String> {
    let id = input.trim().to_uppercase();
    if id.is_empty() {
        return Err("Missing task ID. Usage: /task <subcommand> TSK-XXX".to_string());
    }
    if !id.starts_with("TSK-") {
        return Err(format!(
            "Invalid task ID: '{id}'. Task IDs must start with 'TSK-'"
        ));
    }
    Ok(id)
}

/// Parse two whitespace-separated tokens: an ID and a second argument.
fn parse_id_and_arg(input: &str, arg_name: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = input.trim().splitn(2, char::is_whitespace).collect();
    if parts.is_empty() || parts[0].trim().is_empty() {
        return Err(format!(
            "Missing task ID and {arg_name}. Usage: /task <subcommand> TSK-XXX <{arg_name}>"
        ));
    }
    let id = parse_task_id(parts[0])?;
    let arg = parts.get(1).copied().unwrap_or("").trim().to_string();
    if arg.is_empty() {
        return Err(format!(
            "Missing {arg_name}. Usage: /task <subcommand> TSK-XXX <{arg_name}>"
        ));
    }
    Ok((id, arg))
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

/// Format a task for full detail display.
pub fn format_task_detail(task: &crate::task::Task) -> String {
    let mut lines = Vec::new();
    lines.push(format!("═══ {} ═══", task.id));
    lines.push(format!("Title:   {}", task.title));
    if !task.description.is_empty() {
        lines.push(format!("Description: {}", task.description));
    }
    lines.push(format!("State:   {}", task.state));
    lines.push(format!("Priority: {:?}", task.priority));
    lines.push(format!("Workflow: {}", task.workflow));
    if let Some(ref assignee) = task.assignee {
        lines.push(format!("Assignee: {assignee}"));
    }
    if !task.labels.is_empty() {
        lines.push(format!("Labels:  {}", task.labels.join(", ")));
    }
    if !task.blocked_by.is_empty() {
        lines.push(format!("Blocked by: {}", task.blocked_by.join(", ")));
    }
    lines.push(format!(
        "Created: {}",
        task.created_at.format("%Y-%m-%d %H:%M UTC")
    ));
    lines.push(format!(
        "Updated: {}",
        task.updated_at.format("%Y-%m-%d %H:%M UTC")
    ));
    lines.join("\n")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parse Tests: Create ──────────────────────────────────────────────

    #[test]
    fn parse_create_basic() {
        let cmd = parse_slash_task_command("create Add logging").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Create {
                title: "Add logging".to_string()
            }
        );
    }

    #[test]
    fn parse_create_single_word_title() {
        let cmd = parse_slash_task_command("create Test").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Create {
                title: "Test".to_string()
            }
        );
    }

    #[test]
    fn parse_create_missing_title() {
        let err = parse_slash_task_command("create").unwrap_err();
        assert!(err.contains("Missing title"));
    }

    #[test]
    fn parse_create_with_extra_whitespace() {
        let cmd = parse_slash_task_command("  create   My new task  ").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Create {
                title: "My new task".to_string()
            }
        );
    }

    // ── Parse Tests: List ────────────────────────────────────────────────

    #[test]
    fn parse_list() {
        let cmd = parse_slash_task_command("list").unwrap();
        assert_eq!(cmd, TaskCommand::List);
    }

    #[test]
    fn parse_list_alias_ls() {
        let cmd = parse_slash_task_command("ls").unwrap();
        assert_eq!(cmd, TaskCommand::List);
    }

    // ── Parse Tests: Show ────────────────────────────────────────────────

    #[test]
    fn parse_show() {
        let cmd = parse_slash_task_command("show TSK-001").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Show {
                id: "TSK-001".to_string()
            }
        );
    }

    #[test]
    fn parse_show_alias_get() {
        let cmd = parse_slash_task_command("get TSK-005").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Show {
                id: "TSK-005".to_string()
            }
        );
    }

    #[test]
    fn parse_show_case_insensitive_id() {
        let cmd = parse_slash_task_command("show tsk-001").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Show {
                id: "TSK-001".to_string()
            }
        );
    }

    #[test]
    fn parse_show_missing_id() {
        let err = parse_slash_task_command("show").unwrap_err();
        assert!(err.contains("Missing task ID"));
    }

    #[test]
    fn parse_show_invalid_id_no_prefix() {
        let err = parse_slash_task_command("show 001").unwrap_err();
        assert!(err.contains("Invalid task ID"));
        assert!(err.contains("TSK-"));
    }

    // ── Parse Tests: Move ────────────────────────────────────────────────

    #[test]
    fn parse_move() {
        let cmd = parse_slash_task_command("move TSK-001 in_progress").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Move {
                id: "TSK-001".to_string(),
                state: "in_progress".to_string()
            }
        );
    }

    #[test]
    fn parse_move_alias_mv() {
        let cmd = parse_slash_task_command("mv TSK-003 done").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Move {
                id: "TSK-003".to_string(),
                state: "done".to_string()
            }
        );
    }

    #[test]
    fn parse_move_state_is_lowercased() {
        let cmd = parse_slash_task_command("move TSK-001 IN_PROGRESS").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Move {
                id: "TSK-001".to_string(),
                state: "in_progress".to_string()
            }
        );
    }

    #[test]
    fn parse_move_missing_state() {
        let err = parse_slash_task_command("move TSK-001").unwrap_err();
        assert!(err.contains("Missing state"));
    }

    #[test]
    fn parse_move_missing_id_and_state() {
        let err = parse_slash_task_command("move").unwrap_err();
        assert!(err.contains("Missing task ID"));
    }

    // ── Parse Tests: Done ────────────────────────────────────────────────

    #[test]
    fn parse_done() {
        let cmd = parse_slash_task_command("done TSK-001").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Done {
                id: "TSK-001".to_string()
            }
        );
    }

    #[test]
    fn parse_done_alias_complete() {
        let cmd = parse_slash_task_command("complete TSK-002").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Done {
                id: "TSK-002".to_string()
            }
        );
    }

    #[test]
    fn parse_done_alias_finish() {
        let cmd = parse_slash_task_command("finish TSK-003").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Done {
                id: "TSK-003".to_string()
            }
        );
    }

    #[test]
    fn parse_done_missing_id() {
        let err = parse_slash_task_command("done").unwrap_err();
        assert!(err.contains("Missing task ID"));
    }

    // ── Parse Tests: Block / Unblock ────────────────────────────────────

    #[test]
    fn parse_block() {
        let cmd = parse_slash_task_command("block TSK-003 TSK-001").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Block {
                id: "TSK-003".to_string(),
                blocked_by: "TSK-001".to_string()
            }
        );
    }

    #[test]
    fn parse_unblock() {
        let cmd = parse_slash_task_command("unblock TSK-003 TSK-001").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Unblock {
                id: "TSK-003".to_string(),
                blocked_by: "TSK-001".to_string()
            }
        );
    }

    #[test]
    fn parse_block_missing_blocked_by() {
        let err = parse_slash_task_command("block TSK-003").unwrap_err();
        assert!(err.contains("Missing blocked_by"));
    }

    #[test]
    fn parse_block_missing_both() {
        let err = parse_slash_task_command("block").unwrap_err();
        assert!(err.contains("Missing task ID"));
    }

    // ── Parse Tests: Label ───────────────────────────────────────────────

    #[test]
    fn parse_label() {
        let cmd = parse_slash_task_command("label TSK-001 backend").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Label {
                id: "TSK-001".to_string(),
                label: "backend".to_string()
            }
        );
    }

    #[test]
    fn parse_label_alias_tag() {
        let cmd = parse_slash_task_command("tag TSK-001 urgent").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Label {
                id: "TSK-001".to_string(),
                label: "urgent".to_string()
            }
        );
    }

    #[test]
    fn parse_label_missing_label() {
        let err = parse_slash_task_command("label TSK-001").unwrap_err();
        assert!(err.contains("Missing label"));
    }

    // ── Parse Tests: Assign ──────────────────────────────────────────────

    #[test]
    fn parse_assign() {
        let cmd = parse_slash_task_command("assign TSK-001 alice").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Assign {
                id: "TSK-001".to_string(),
                assignee: "alice".to_string()
            }
        );
    }

    #[test]
    fn parse_assign_missing_assignee() {
        let err = parse_slash_task_command("assign TSK-001").unwrap_err();
        assert!(err.contains("Missing assignee"));
    }

    // ── Parse Tests: Errors ──────────────────────────────────────────────

    #[test]
    fn parse_empty_input() {
        let err = parse_slash_task_command("").unwrap_err();
        assert!(err.contains("Missing task subcommand"));
    }

    #[test]
    fn parse_whitespace_only() {
        let err = parse_slash_task_command("   ").unwrap_err();
        assert!(err.contains("Missing task subcommand"));
    }

    #[test]
    fn parse_unknown_subcommand() {
        let err = parse_slash_task_command("delete TSK-001").unwrap_err();
        assert!(err.contains("Unknown task subcommand"));
        assert!(err.contains("delete"));
    }

    #[test]
    fn parse_subcommand_is_case_insensitive() {
        let cmd = parse_slash_task_command("CREATE My task").unwrap();
        assert_eq!(
            cmd,
            TaskCommand::Create {
                title: "My task".to_string()
            }
        );
    }

    // ── Format Detail Tests ──────────────────────────────────────────────

    #[test]
    fn format_detail_basic_task() {
        let now = chrono::Utc::now();
        let task = crate::task::Task {
            id: "TSK-001".to_string(),
            title: "Test task".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: crate::task::TaskPriority::None,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: now,
            updated_at: now,
        };
        let output = format_task_detail(&task);
        assert!(output.contains("TSK-001"));
        assert!(output.contains("Test task"));
        assert!(output.contains("todo"));
        assert!(output.contains("default"));
    }

    #[test]
    fn format_detail_with_all_fields() {
        let now = chrono::Utc::now();
        let task = crate::task::Task {
            id: "TSK-005".to_string(),
            title: "Complex task".to_string(),
            description: "Detailed description".to_string(),
            state: "in_progress".to_string(),
            priority: crate::task::TaskPriority::High,
            workflow: "kanban".to_string(),
            blocked_by: vec!["TSK-001".to_string(), "TSK-002".to_string()],
            labels: vec!["backend".to_string(), "rust".to_string()],
            assignee: Some("alice".to_string()),
            created_at: now,
            updated_at: now,
        };
        let output = format_task_detail(&task);
        assert!(output.contains("TSK-005"));
        assert!(output.contains("Complex task"));
        assert!(output.contains("Detailed description"));
        assert!(output.contains("in_progress"));
        assert!(output.contains("High"));
        assert!(output.contains("alice"));
        assert!(output.contains("backend, rust"));
        assert!(output.contains("TSK-001, TSK-002"));
    }

    // ── Handle Command Integration Tests ─────────────────────────────────

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
