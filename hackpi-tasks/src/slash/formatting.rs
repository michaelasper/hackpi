use crate::task::Task;

/// Format a task for full detail display.
pub fn format_task_detail(task: &Task) -> String {
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
}
