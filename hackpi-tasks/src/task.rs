use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Task Priority ───────────────────────────────────────────────────────────

/// Priority levels for tasks, ordered from highest to lowest urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    None,
    Low,
    Medium,
    High,
    Urgent,
}

// ── Task ────────────────────────────────────────────────────────────────────

/// A task in the hackpi task management system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    /// Unique identifier in TSK-XXX format.
    pub id: String,
    /// Short summary of the task.
    pub title: String,
    /// Detailed description of the task.
    pub description: String,
    /// Current state within the workflow (e.g., "todo", "in_progress", "done").
    pub state: String,
    /// Priority level.
    pub priority: TaskPriority,
    /// Name of the workflow this task belongs to.
    pub workflow: String,
    /// IDs of tasks that block this one.
    pub blocked_by: Vec<String>,
    /// Labels/tags for categorization.
    pub labels: Vec<String>,
    /// Optional assignee identifier.
    pub assignee: Option<String>,
    /// When this task was created.
    pub created_at: DateTime<Utc>,
    /// When this task was last updated.
    pub updated_at: DateTime<Utc>,
}

// ── NewTask ─────────────────────────────────────────────────────────────────

/// Input for creating a new task. Only `title` is required;
/// all other fields have sensible defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewTask {
    #[doc = "Task title. Use `NewTask::new(title)` for construction with defaults."]
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<TaskPriority>,
    pub workflow: Option<String>,
    pub labels: Option<Vec<String>>,
    pub assignee: Option<String>,
}

impl NewTask {
    /// Create a new `NewTask` with the given title and default values for
    /// everything else.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            priority: None,
            workflow: None,
            labels: None,
            assignee: None,
        }
    }
}

// ── TaskUpdate ──────────────────────────────────────────────────────────────

/// Partial update to an existing task. Only fields set to `Some(..)` will be
/// changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<String>,
    pub priority: Option<TaskPriority>,
    pub workflow: Option<String>,
    pub blocked_by: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub assignee: Option<Option<String>>,
}

// ── TaskFilter ──────────────────────────────────────────────────────────────

/// Filter criteria for listing tasks. All fields are optional; `None` means
/// "don't filter by this field".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskFilter {
    pub state: Option<String>,
    pub priority: Option<TaskPriority>,
    pub labels: Option<Vec<String>>,
    pub assignee: Option<String>,
    pub workflow: Option<String>,
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ── TaskPriority Tests ───────────────────────────────────────────────

    #[test]
    fn task_priority_ordering() {
        assert!(TaskPriority::Urgent > TaskPriority::High);
        assert!(TaskPriority::High > TaskPriority::Medium);
        assert!(TaskPriority::Medium > TaskPriority::Low);
        assert!(TaskPriority::Low > TaskPriority::None);
    }

    #[test]
    fn task_priority_equality() {
        assert_eq!(TaskPriority::Medium, TaskPriority::Medium);
        assert_ne!(TaskPriority::High, TaskPriority::Low);
    }

    #[test]
    fn task_priority_serde_lowercase() {
        // Serialization should produce lowercase
        let json = serde_json::to_string(&TaskPriority::Urgent).expect("serialize");
        assert_eq!(json, "\"urgent\"");

        let json = serde_json::to_string(&TaskPriority::High).expect("serialize");
        assert_eq!(json, "\"high\"");

        let json = serde_json::to_string(&TaskPriority::None).expect("serialize");
        assert_eq!(json, "\"none\"");
    }

    #[test]
    fn task_priority_deserialize_lowercase() {
        let p: TaskPriority = serde_json::from_str("\"urgent\"").expect("deserialize");
        assert_eq!(p, TaskPriority::Urgent);

        let p: TaskPriority = serde_json::from_str("\"medium\"").expect("deserialize");
        assert_eq!(p, TaskPriority::Medium);

        let p: TaskPriority = serde_json::from_str("\"none\"").expect("deserialize");
        assert_eq!(p, TaskPriority::None);
    }

    #[test]
    fn task_priority_serde_roundtrip() {
        for priority in [
            TaskPriority::Urgent,
            TaskPriority::High,
            TaskPriority::Medium,
            TaskPriority::Low,
            TaskPriority::None,
        ] {
            let json = serde_json::to_string(&priority).expect("serialize");
            let back: TaskPriority = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(priority, back, "roundtrip failed for {priority:?}");
        }
    }

    #[test]
    fn task_priority_invalid_deserialize() {
        let result = serde_json::from_str::<TaskPriority>("\"invalid\"");
        assert!(result.is_err(), "should fail for invalid priority string");
    }

    // ── Task Serde Tests ─────────────────────────────────────────────────

    #[test]
    fn task_serde_roundtrip() {
        let now = Utc::now();
        let task = Task {
            id: "TSK-001".to_string(),
            title: "Implement feature X".to_string(),
            description: "Detailed description here".to_string(),
            state: "todo".to_string(),
            priority: TaskPriority::High,
            workflow: "default".to_string(),
            blocked_by: vec!["TSK-002".to_string()],
            labels: vec!["backend".to_string(), "rust".to_string()],
            assignee: Some("alice".to_string()),
            created_at: now,
            updated_at: now,
        };

        let json = serde_json::to_string_pretty(&task).expect("serialize");
        let back: Task = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(task, back);
    }

    #[test]
    fn task_serde_optional_fields_none() {
        let now = Utc::now();
        let task = Task {
            id: "TSK-003".to_string(),
            title: "Simple task".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: TaskPriority::None,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: now,
            updated_at: now,
        };

        let json = serde_json::to_string(&task).expect("serialize");
        let back: Task = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(task, back);
        assert!(back.assignee.is_none());
        assert!(back.blocked_by.is_empty());
        assert!(back.labels.is_empty());
    }

    #[test]
    fn task_json_structure() {
        let now = Utc::now();
        let task = Task {
            id: "TSK-001".to_string(),
            title: "Test".to_string(),
            description: "Desc".to_string(),
            state: "in_progress".to_string(),
            priority: TaskPriority::Medium,
            workflow: "kanban".to_string(),
            blocked_by: vec![],
            labels: vec!["feature".to_string()],
            assignee: None,
            created_at: now,
            updated_at: now,
        };

        let json: serde_json::Value = serde_json::to_value(&task).expect("serialize to value");

        assert_eq!(json["id"], "TSK-001");
        assert_eq!(json["title"], "Test");
        assert_eq!(json["state"], "in_progress");
        assert_eq!(json["priority"], "medium");
        assert_eq!(json["workflow"], "kanban");
        assert!(json["assignee"].is_null());
        assert_eq!(json["labels"][0], "feature");
    }

    // ── NewTask Tests ────────────────────────────────────────────────────

    #[test]
    fn new_task_basic() {
        let nt = NewTask::new("Build the thing");
        assert_eq!(nt.title, "Build the thing");
        assert!(nt.description.is_none());
        assert!(nt.priority.is_none());
        assert!(nt.workflow.is_none());
        assert!(nt.labels.is_none());
        assert!(nt.assignee.is_none());
    }

    #[test]
    fn new_task_with_all_fields() {
        let nt = NewTask {
            title: "Full task".to_string(),
            description: Some("With details".to_string()),
            priority: Some(TaskPriority::Urgent),
            workflow: Some("custom".to_string()),
            labels: Some(vec!["critical".to_string()]),
            assignee: Some("bob".to_string()),
        };
        assert_eq!(nt.title, "Full task");
        assert_eq!(nt.description.as_deref(), Some("With details"));
        assert_eq!(nt.priority, Some(TaskPriority::Urgent));
        assert_eq!(nt.workflow.as_deref(), Some("custom"));
        assert_eq!(nt.labels.as_ref().map(|l| l.len()), Some(1));
        assert_eq!(nt.assignee.as_deref(), Some("bob"));
    }

    #[test]
    fn new_task_serde_roundtrip() {
        let nt = NewTask::new("Test task");
        let json = serde_json::to_string(&nt).expect("serialize");
        let back: NewTask = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(nt.title, back.title);
    }

    // ── TaskUpdate Tests ─────────────────────────────────────────────────

    #[test]
    fn task_update_default_is_all_none() {
        let update = TaskUpdate::default();
        assert!(update.title.is_none());
        assert!(update.description.is_none());
        assert!(update.state.is_none());
        assert!(update.priority.is_none());
        assert!(update.workflow.is_none());
        assert!(update.blocked_by.is_none());
        assert!(update.labels.is_none());
        assert!(update.assignee.is_none());
    }

    #[test]
    fn task_update_partial() {
        let update = TaskUpdate {
            title: Some("New title".to_string()),
            state: Some("in_progress".to_string()),
            ..Default::default()
        };
        assert_eq!(update.title.as_deref(), Some("New title"));
        assert_eq!(update.state.as_deref(), Some("in_progress"));
        assert!(update.description.is_none());
    }

    #[test]
    fn task_update_serde_roundtrip() {
        let update = TaskUpdate {
            title: Some("Updated".to_string()),
            priority: Some(TaskPriority::High),
            ..Default::default()
        };
        let json = serde_json::to_string(&update).expect("serialize");
        let back: TaskUpdate = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(update.title, back.title);
        assert_eq!(update.priority, back.priority);
    }

    // ── TaskFilter Tests ─────────────────────────────────────────────────

    #[test]
    fn task_filter_default_is_all_none() {
        let filter = TaskFilter::default();
        assert!(filter.state.is_none());
        assert!(filter.priority.is_none());
        assert!(filter.labels.is_none());
        assert!(filter.assignee.is_none());
        assert!(filter.workflow.is_none());
    }

    #[test]
    fn task_filter_with_criteria() {
        let filter = TaskFilter {
            state: Some("todo".to_string()),
            priority: Some(TaskPriority::High),
            labels: Some(vec!["backend".to_string()]),
            assignee: None,
            workflow: None,
        };
        assert_eq!(filter.state.as_deref(), Some("todo"));
        assert_eq!(filter.priority, Some(TaskPriority::High));
        assert_eq!(filter.labels.as_ref().map(|l| l.len()), Some(1));
    }

    #[test]
    fn task_filter_serde_roundtrip() {
        let filter = TaskFilter {
            state: Some("done".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&filter).expect("serialize");
        let back: TaskFilter = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(filter.state, back.state);
    }
}
