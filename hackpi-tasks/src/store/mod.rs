pub mod crud;
pub mod json_store;
pub mod query;
pub mod traits;
pub mod transitions;

pub use json_store::JsonTaskStore;
pub use traits::TaskStore;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::task::{NewTask, Task, TaskFilter, TaskPriority, TaskUpdate};

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

    // ── Workflow Integration Tests ──────────────────────────────────────

    #[tokio::test]
    async fn create_defaults_to_default_workflow() {
        let (_dir, store) = setup_store().await;
        let task = store.create(&NewTask::new("Task")).await.expect("create");
        assert_eq!(task.workflow, "default");
    }

    #[tokio::test]
    async fn get_workflow_fallback_to_default() {
        let (_dir, store) = setup_store().await;
        let wf = store.get_workflow("nonexistent_workflow").await;
        assert_eq!(wf.name, "default");
    }

    #[tokio::test]
    async fn create_with_custom_workflow() {
        let (_dir, store) = setup_store().await;
        let input = NewTask {
            title: "Custom".to_string(),
            workflow: Some("custom".to_string()),
            ..Default::default()
        };
        let task = store.create(&input).await.expect("create");
        assert_eq!(task.workflow, "custom");
    }

    #[tokio::test]
    async fn update_valid_transition_succeeds() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");
        assert_eq!(created.state, "todo");

        let update = TaskUpdate {
            state: Some("in_progress".to_string()),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");
        assert_eq!(updated.state, "in_progress");
    }

    #[tokio::test]
    async fn update_invalid_transition_fails() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");
        assert_eq!(created.state, "todo");

        // todo → done is not allowed in the default workflow
        let update = TaskUpdate {
            state: Some("done".to_string()),
            ..Default::default()
        };
        let result = store.update(&created.id, &update).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Invalid transition"),
            "error should mention Invalid transition: {err}"
        );
        assert!(
            err.contains("\"todo\" → \"done\""),
            "error should mention the states: {err}"
        );
        assert!(
            err.contains("default"),
            "error should mention the workflow name: {err}"
        );
    }

    #[tokio::test]
    async fn update_invalid_transition_done_to_in_progress() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        // First, do a valid transition: todo → in_progress → done
        store
            .update(
                &created.id,
                &TaskUpdate {
                    state: Some("in_progress".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update")
            .expect("some");

        store
            .update(
                &created.id,
                &TaskUpdate {
                    state: Some("done".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update")
            .expect("some");

        // Now try to go back from done → in_progress (invalid)
        let result = store
            .update(
                &created.id,
                &TaskUpdate {
                    state: Some("in_progress".to_string()),
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("\"done\" → \"in_progress\""),
            "error should mention the states: {err}"
        );
    }

    #[tokio::test]
    async fn update_same_state_is_allowed() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        // Transitioning to the same state should be a no-op and succeed
        let update = TaskUpdate {
            state: Some("todo".to_string()),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");
        assert_eq!(updated.state, "todo");
    }

    #[tokio::test]
    async fn update_non_state_fields_bypass_transition_check() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        // Updating just the title should not trigger transition validation
        let update = TaskUpdate {
            title: Some("Updated".to_string()),
            ..Default::default()
        };
        let updated = store
            .update(&created.id, &update)
            .await
            .expect("update")
            .expect("some");
        assert_eq!(updated.title, "Updated");
        assert_eq!(updated.state, "todo");
    }

    #[tokio::test]
    async fn update_valid_multi_step_transition() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        // todo → in_progress → in_review → done
        let steps = ["in_progress", "in_review", "done"];
        let mut current_id = created.id.clone();

        for state in steps {
            let update = TaskUpdate {
                state: Some(state.to_string()),
                ..Default::default()
            };
            let updated = store
                .update(&current_id, &update)
                .await
                .expect("update")
                .expect("some");
            assert_eq!(updated.state, state);
            current_id = updated.id;
        }
    }

    #[tokio::test]
    async fn update_invalid_transition_cancelled_to_todo() {
        let (_dir, store) = setup_store().await;
        let created = store.create(&NewTask::new("Task")).await.expect("create");

        // todo → cancelled (valid)
        store
            .update(
                &created.id,
                &TaskUpdate {
                    state: Some("cancelled".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update")
            .expect("some");

        // cancelled → todo (invalid)
        let result = store
            .update(
                &created.id,
                &TaskUpdate {
                    state: Some("todo".to_string()),
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("\"cancelled\" → \"todo\""),
            "error should mention the states: {err}"
        );
    }

    #[tokio::test]
    async fn with_workflows_loads_custom_workflow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        let yaml = r#"
name: fast
description: "Fast track"
states:
  - new
  - complete
transitions:
  - from: new
    to:
      - complete
"#;
        tokio::fs::write(workflows_dir.join("fast.yaml"), yaml)
            .await
            .expect("write");

        let store = JsonTaskStore::with_workflows(tasks_dir, &workflows_dir)
            .await
            .expect("create store");

        // Create a task with the custom workflow
        let input = NewTask {
            title: "Fast task".to_string(),
            workflow: Some("fast".to_string()),
            ..Default::default()
        };
        let task = store.create(&input).await.expect("create");
        assert_eq!(task.workflow, "fast");
        assert_eq!(task.state, "new"); // Initial state is first state in workflow
    }

    #[tokio::test]
    async fn custom_workflow_initial_state_no_todo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        // Workflow with no "todo" state — task should start at "new"
        let yaml = r#"
name: fast
description: "Fast track"
states:
  - new
  - complete
transitions:
  - from: new
    to:
      - complete
"#;
        tokio::fs::write(workflows_dir.join("fast.yaml"), yaml)
            .await
            .expect("write");

        let store = JsonTaskStore::with_workflows(tasks_dir, &workflows_dir)
            .await
            .expect("create store");

        let input = NewTask {
            title: "Fast task".to_string(),
            workflow: Some("fast".to_string()),
            ..Default::default()
        };
        let task = store.create(&input).await.expect("create");

        // Should NOT be "todo" — that state doesn't exist in this workflow
        assert_eq!(
            task.state, "new",
            "should use workflow's first state, not hardcoded 'todo'"
        );

        // Valid transition from "new" to "complete" should work
        let updated = store
            .update(
                &task.id,
                &TaskUpdate {
                    state: Some("complete".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update")
            .expect("some");
        assert_eq!(updated.state, "complete");
    }

    #[tokio::test]
    async fn unknown_workflow_falls_back_to_default() {
        let (_dir, store) = setup_store().await;

        // Create a task with a nonexistent workflow
        let input = NewTask {
            title: "Unknown workflow task".to_string(),
            workflow: Some("nonexistent".to_string()),
            ..Default::default()
        };
        let task = store.create(&input).await.expect("create");
        assert_eq!(task.workflow, "nonexistent");

        // Update should still use default workflow validation as fallback
        let update = TaskUpdate {
            state: Some("done".to_string()),
            ..Default::default()
        };
        let result = store.update(&task.id, &update).await;
        assert!(
            result.is_err(),
            "should fail since default workflow doesn't allow todo → done"
        );
    }

    #[tokio::test]
    async fn custom_workflow_enforces_transitions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        // Custom workflow: todo → wip → done (no direct todo → done)
        let yaml = r#"
name: custom
description: "Custom workflow"
states:
  - todo
  - wip
  - done
transitions:
  - from: todo
    to:
      - wip
  - from: wip
    to:
      - done
"#;
        tokio::fs::write(workflows_dir.join("custom.yaml"), yaml)
            .await
            .expect("write");

        let store = JsonTaskStore::with_workflows(tasks_dir, &workflows_dir)
            .await
            .expect("create store");

        let task = store
            .create(&NewTask {
                title: "Custom task".to_string(),
                workflow: Some("custom".to_string()),
                ..Default::default()
            })
            .await
            .expect("create");
        assert_eq!(task.workflow, "custom");
        assert_eq!(task.state, "todo");

        // Valid transition: todo → wip (allowed in custom workflow)
        let updated = store
            .update(
                &task.id,
                &TaskUpdate {
                    state: Some("wip".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update")
            .expect("some");
        assert_eq!(updated.state, "wip");

        // Invalid transition: wip → todo (not in custom workflow transitions)
        let result = store
            .update(
                &task.id,
                &TaskUpdate {
                    state: Some("todo".to_string()),
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_err(), "wip → todo should be invalid");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid transition"), "error: {err}");
        assert!(
            err.contains("custom"),
            "error should mention workflow: {err}"
        );

        // Valid transition: wip → done (allowed in custom workflow)
        let updated = store
            .update(
                &task.id,
                &TaskUpdate {
                    state: Some("done".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update")
            .expect("some");
        assert_eq!(updated.state, "done");

        // Invalid transition from terminal state: done → wip
        let result = store
            .update(
                &task.id,
                &TaskUpdate {
                    state: Some("wip".to_string()),
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_err(), "done → wip should be invalid");
    }

    // ── Concurrency Tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn concurrent_updates_to_same_task_all_succeed() {
        let (_dir, store) = setup_store().await;
        let store = Arc::new(store);
        let task = store
            .create(&NewTask::new("Concurrent task"))
            .await
            .expect("create");
        let id = task.id.clone();

        let mut handles = vec![];
        for i in 0..20 {
            let s = Arc::clone(&store);
            let id_clone = id.clone();
            handles.push(tokio::spawn(async move {
                s.update(
                    &id_clone,
                    &TaskUpdate {
                        title: Some(format!("Update {i}")),
                        priority: Some(TaskPriority::High),
                        ..Default::default()
                    },
                )
                .await
            }));
        }

        for handle in handles {
            let result = handle.await.expect("join");
            assert!(result.is_ok(), "concurrent update should succeed");
            let updated = result
                .expect("is_ok already checked")
                .expect("task should exist");
            assert!(
                updated.title.starts_with("Update "),
                "title should be set: {}",
                updated.title
            );
        }

        // Final state should be valid and have priority set
        let final_task = store.get(&id).await.expect("get").expect("exists");
        assert!(final_task.title.starts_with("Update "));
        assert_eq!(final_task.priority, TaskPriority::High);
    }

    #[tokio::test]
    async fn concurrent_updates_different_tasks_independent() {
        let (_dir, store) = setup_store().await;
        let store = Arc::new(store);

        let t1 = store.create(&NewTask::new("Task A")).await.expect("create");
        let t2 = store.create(&NewTask::new("Task B")).await.expect("create");

        // Concurrently update two different tasks — should not block each other
        let s1 = Arc::clone(&store);
        let id1 = t1.id.clone();
        let h1 = tokio::spawn(async move {
            s1.update(
                &id1,
                &TaskUpdate {
                    title: Some("Task A updated".to_string()),
                    priority: Some(TaskPriority::Urgent),
                    ..Default::default()
                },
            )
            .await
        });

        let s2 = Arc::clone(&store);
        let id2 = t2.id.clone();
        let h2 = tokio::spawn(async move {
            s2.update(
                &id2,
                &TaskUpdate {
                    title: Some("Task B updated".to_string()),
                    priority: Some(TaskPriority::Low),
                    ..Default::default()
                },
            )
            .await
        });

        let r1 = h1
            .await
            .expect("join")
            .expect("update A")
            .expect("task A exists");
        let r2 = h2
            .await
            .expect("join")
            .expect("update B")
            .expect("task B exists");

        assert_eq!(r1.title, "Task A updated");
        assert_eq!(r1.priority, TaskPriority::Urgent);
        assert_eq!(r2.title, "Task B updated");
        assert_eq!(r2.priority, TaskPriority::Low);
    }

    #[tokio::test]
    async fn concurrent_update_and_delete_same_task() {
        let (_dir, store) = setup_store().await;
        let store = Arc::new(store);
        let task = store
            .create(&NewTask::new("Race me"))
            .await
            .expect("create");
        let id = task.id.clone();

        // Spawn update and delete racing on the same task
        let s_update = Arc::clone(&store);
        let id_update = id.clone();
        let update_handle = tokio::spawn(async move {
            s_update
                .update(
                    &id_update,
                    &TaskUpdate {
                        title: Some("Updated".to_string()),
                        ..Default::default()
                    },
                )
                .await
        });

        let s_delete = Arc::clone(&store);
        let id_for_delete = id.clone();
        let delete_handle = tokio::spawn(async move { s_delete.delete(&id_for_delete).await });

        let (update_result, delete_result) = tokio::join!(update_handle, delete_handle);

        // Both should succeed without panics or corruption
        let _ = update_result.expect("join");
        let deleted = delete_result.expect("join");

        // The task should be gone (delete won the race) or the update won
        let final_state = store.get(&id).await.expect("get");
        if final_state.is_some() {
            assert!(
                deleted.unwrap_or(false),
                "if task still exists, delete should have returned true"
            );
        }
    }

    #[tokio::test]
    async fn concurrent_writes_to_same_task_no_temp_collision() {
        let (_dir, store) = setup_store().await;
        let store = Arc::new(store);

        // Create a task, then spam updates to trigger many temp file writes
        let task = store
            .create(&NewTask::new("Temp collision test"))
            .await
            .expect("create");
        let id = task.id.clone();

        let mut handles = vec![];
        for i in 0..50 {
            let s = Arc::clone(&store);
            let id_clone = id.clone();
            handles.push(tokio::spawn(async move {
                s.update(
                    &id_clone,
                    &TaskUpdate {
                        title: Some(format!("Write {i}")),
                        ..Default::default()
                    },
                )
                .await
            }));
        }

        for handle in handles {
            handle
                .await
                .expect("join")
                .expect("concurrent write should not panic");
        }

        // No leftover temp files should exist
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
    async fn reload_workflows_adds_new_profiles() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        let store = JsonTaskStore::new(tasks_dir).await.expect("create store");

        // Initially should have only default workflow
        {
            let guard = store.workflows.read().await;
            assert!(guard.contains_key("default"));
            assert_eq!(guard.len(), 1);
        }

        // Add a custom workflow file
        let yaml = r#"
name: custom
description: "Custom"
states:
  - a
  - b
transitions:
  - from: a
    to:
      - b
"#;
        tokio::fs::write(workflows_dir.join("custom.yaml"), yaml)
            .await
            .expect("write");

        store
            .reload_workflows(&workflows_dir)
            .await
            .expect("reload");

        {
            let guard = store.workflows.read().await;
            assert!(guard.contains_key("default"));
            assert!(guard.contains_key("custom"));
            assert_eq!(guard.len(), 2);
        }
    }
}
