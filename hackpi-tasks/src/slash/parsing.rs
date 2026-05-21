use super::commands::TaskCommand;

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
}
