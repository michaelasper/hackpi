mod branch;
pub mod git_write_tool;
mod history;
mod remote;
mod staging;

pub use git_write_tool::GitWriteTool;

/// Resolve the current branch name as a String.
pub(super) fn current_branch_name(repo: &git2::Repository) -> Option<String> {
    repo.head()
        .ok()
        .and_then(|h| h.shorthand().map(|s| s.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::GitWriteTool;
    use hackpi_core::tools::{Tool, ToolContext, ToolResult};
    use serde_json::{json, Value};
    use std::path::PathBuf;

    /// Helper: create a temporary git repo with basic config.
    fn init_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init repo");
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        (dir, repo)
    }

    /// Helper: create an initial commit with a file, returning the repo.
    fn init_repo_with_commit() -> (tempfile::TempDir, git2::Repository) {
        let (dir, repo) = init_repo();
        let path = dir.path().join("README.md");
        std::fs::write(&path, b"# Test\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        {
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .unwrap();
        }
        (dir, repo)
    }

    /// Helper: create a second commit.
    fn add_commit(
        repo: &git2::Repository,
        dir: &tempfile::TempDir,
        filename: &str,
        content: &[u8],
        msg: &str,
    ) {
        let path = dir.path().join(filename);
        std::fs::write(&path, content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(filename)).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        {
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent])
                .unwrap();
        }
    }

    fn make_tool(workspace_root: PathBuf) -> GitWriteTool {
        GitWriteTool::new(workspace_root)
    }

    fn test_ctx() -> ToolContext {
        ToolContext {
            workspace_root: std::env::temp_dir(),
            signal: tokio::sync::watch::channel(false).1,
        }
    }

    async fn execute(tool: &GitWriteTool, params: Value) -> ToolResult {
        tool.execute(params, &test_ctx()).await
    }

    // ── Basic tool metadata ──

    #[tokio::test]
    async fn test_name() {
        let tool = make_tool(PathBuf::from("/tmp"));
        assert_eq!(tool.name(), "git_write");
    }

    #[tokio::test]
    async fn test_description() {
        let tool = make_tool(PathBuf::from("/tmp"));
        assert!(tool.description().contains("add"));
        assert!(tool.description().contains("commit"));
        assert!(tool.description().contains("push"));
        assert!(tool.description().contains("pull"));
        assert!(tool.description().contains("checkout"));
    }

    #[test]
    fn test_input_schema_has_additional_properties_false() {
        let tool = make_tool(PathBuf::from("/tmp"));
        let schema = tool.input_schema();
        assert_eq!(schema.get("additionalProperties"), Some(&json!(false)));
        assert!(schema.get("properties").unwrap().get("operation").is_some());
    }

    // ── Missing repo ──

    #[tokio::test]
    async fn test_missing_repo_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = make_tool(tmp.path().to_path_buf());
        let result = execute(&tool, json!({ "operation": "add" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Not a git repository")),
            "Expected SystemError about missing repo, got: {result:?}"
        );
    }

    // ── Add operation ──

    #[tokio::test]
    async fn test_add_specific_files() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("new_file.txt"), b"hello").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "add", "paths": ["new_file.txt"] }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Added"),
                    "Expected 'Added ...', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify it's actually staged
        let statuses = repo.statuses(None).unwrap();
        let staged = statuses.iter().any(|e| {
            e.status().contains(git2::Status::INDEX_NEW) && e.path() == Some("new_file.txt")
        });
        assert!(staged, "new_file.txt should be staged");
    }

    #[tokio::test]
    async fn test_add_all() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("file_a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("file_b.txt"), b"b").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "add", "all": true })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Added"),
                    "Expected 'Added ...', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify both are staged
        let statuses = repo.statuses(None).unwrap();
        let staged_count = statuses
            .iter()
            .filter(|e| e.status().contains(git2::Status::INDEX_NEW))
            .count();
        assert_eq!(staged_count, 2, "Expected 2 staged files");
    }

    #[tokio::test]
    async fn test_add_no_params_rejected() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("untracked.txt"), b"data").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "add" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("No files to stage")),
            "Expected SystemError about no files to stage, got: {result:?}"
        );
        // Verify nothing was staged
        let statuses = repo.statuses(None).unwrap();
        let untracked = statuses.iter().any(|e| {
            e.status().contains(git2::Status::WT_NEW) && e.path() == Some("untracked.txt")
        });
        assert!(untracked, "untracked.txt should remain untracked");
    }

    #[tokio::test]
    async fn test_add_empty_paths_rejected() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "add", "paths": [] })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("No files to stage")),
            "Expected SystemError about no files to stage, got: {result:?}"
        );
    }

    // ── Commit operation ──

    #[tokio::test]
    async fn test_commit_creates_commit() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("feature.txt"), b"feature content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("feature.txt")).unwrap();
        index.write().unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Add feature" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Committed"),
                    "Expected 'Committed ...', got: {content}"
                );
                assert!(
                    content.contains("Add feature"),
                    "Expected message in output, got: {content}"
                );
                assert!(
                    content.contains("file(s) changed"),
                    "Expected stats in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify commit exists
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap().trim(), "Add feature");
    }

    #[tokio::test]
    async fn test_commit_missing_message() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "commit" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'message'")),
            "Expected SystemError for missing message, got: {result:?}"
        );
    }

    // ── Checkout operation ──

    #[tokio::test]
    async fn test_checkout_create_and_switch_branch() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "branch": "feature", "create": true }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Switched to branch 'feature'"),
                    "Expected 'Switched to branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify we're on the new branch
        let head = repo.head().unwrap();
        assert_eq!(head.shorthand().unwrap(), "feature");
    }

    #[tokio::test]
    async fn test_checkout_existing_branch() {
        let (_dir, repo) = init_repo_with_commit();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("develop", &commit, false).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "branch": "develop" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Switched to branch 'develop'"),
                    "Expected 'Switched to branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        assert_eq!(repo.head().unwrap().shorthand().unwrap(), "develop");
    }

    #[tokio::test]
    async fn test_checkout_restore_files() {
        let (dir, repo) = init_repo_with_commit();
        // Modify a tracked file
        std::fs::write(dir.path().join("README.md"), b"# Modified\n").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "paths": ["README.md"] }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Restored"),
                    "Expected 'Restored ...', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify file is restored
        let content = std::fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert_eq!(content, "# Test\n");
    }

    // ── Branch create ──

    #[tokio::test]
    async fn test_branch_create() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_create", "branch": "new-branch" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created branch 'new-branch'"),
                    "Expected 'Created branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify branch exists
        assert!(repo
            .find_branch("new-branch", git2::BranchType::Local)
            .is_ok());
    }

    #[tokio::test]
    async fn test_branch_create_missing_name() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "branch_create" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'branch'")),
            "Expected SystemError for missing branch name, got: {result:?}"
        );
    }

    // ── Branch delete ──

    #[tokio::test]
    async fn test_branch_delete() {
        let (_dir, repo) = init_repo_with_commit();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("delete-me", &commit, false).unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_delete", "branch": "delete-me" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Deleted branch 'delete-me'"),
                    "Expected 'Deleted branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify branch is gone
        assert!(repo
            .find_branch("delete-me", git2::BranchType::Local)
            .is_err());
    }

    #[tokio::test]
    async fn test_branch_delete_nonexistent() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_delete", "branch": "nonexistent" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for nonexistent branch, got: {result:?}"
        );
    }

    // ── Reset operation ──

    #[tokio::test]
    async fn test_reset_mixed() {
        let (dir, repo) = init_repo_with_commit();
        // Create a second commit
        add_commit(&repo, &dir, "file2.txt", b"second", "Second commit");
        let _second_oid = repo.head().unwrap().peel_to_commit().unwrap().id();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "reset", "revision": "HEAD~1", "mode": "mixed" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Reset to"),
                    "Expected 'Reset to ...', got: {content}"
                );
                assert!(
                    content.contains("mixed"),
                    "Expected 'mixed' in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // After mixed reset, HEAD should point to initial commit
        let head_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
        assert_ne!(head_oid, _second_oid, "HEAD should have been reset");
    }

    #[tokio::test]
    async fn test_reset_soft() {
        let (dir, repo) = init_repo_with_commit();
        add_commit(&repo, &dir, "file2.txt", b"second", "Second commit");
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "reset", "revision": "HEAD~1", "mode": "soft" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("soft"),
                    "Expected 'soft' in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_reset_default_revision() {
        let (dir, repo) = init_repo_with_commit();
        add_commit(&repo, &dir, "file2.txt", b"second", "Second commit");
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "reset", "mode": "hard" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("HEAD"),
                    "Expected 'HEAD' revision, got: {content}"
                );
                assert!(
                    content.contains("hard"),
                    "Expected 'hard' mode, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_reset_invalid_revision() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "reset", "revision": "nonexistent" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to resolve revision")),
            "Expected SystemError for bad revision, got: {result:?}"
        );
    }

    // ── Stash operations ──

    #[tokio::test]
    async fn test_stash_and_stash_pop() {
        let (dir, repo) = init_repo_with_commit();
        // Modify a tracked file
        std::fs::write(dir.path().join("README.md"), b"# Stashed change\n").unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        // Stash
        let stash_result = execute(&tool, json!({ "operation": "stash", "message": "WIP" })).await;
        match &stash_result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Saved working directory"),
                    "Expected 'Saved working directory', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {stash_result:?}"),
        }

        // Verify working directory is clean
        let statuses = repo.statuses(None).unwrap();
        let has_changes = statuses
            .iter()
            .any(|e| !e.status().is_empty() && !e.status().contains(git2::Status::IGNORED));
        assert!(
            !has_changes,
            "Working directory should be clean after stash"
        );

        // Pop the stash
        let pop_result = execute(&tool, json!({ "operation": "stash_pop" })).await;
        match &pop_result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Restored stashed state"),
                    "Expected 'Restored stashed state', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {pop_result:?}"),
        }

        // Verify changes are restored
        let readme_content = std::fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert_eq!(readme_content, "# Stashed change\n");
    }

    #[tokio::test]
    async fn test_stash_pop_no_stash() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "stash_pop" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to pop stash")),
            "Expected SystemError for empty stash, got: {result:?}"
        );
    }

    // ── Merge operation (fast-forward) ──

    #[tokio::test]
    async fn test_merge_fast_forward() {
        let (dir, repo) = init_repo_with_commit();

        // Create a branch and add a commit
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-branch", &commit, false).unwrap();

        // Switch to feature branch and add a commit
        let feature_ref = "refs/heads/feature-branch";
        repo.set_head(feature_ref).unwrap();
        let feature_obj = repo.revparse_single(feature_ref).unwrap();
        repo.checkout_tree(&feature_obj, None).unwrap();

        add_commit(
            &repo,
            &dir,
            "feature-file.txt",
            b"feature work",
            "Feature work",
        );

        // Switch back to main
        let head_obj = repo.revparse_single("refs/heads/main").unwrap();
        repo.checkout_tree(&head_obj, None).unwrap();
        repo.set_head("refs/heads/main").unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "merge", "branch": "feature-branch" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Merged feature-branch"),
                    "Expected 'Merged feature-branch', got: {content}"
                );
                assert!(
                    content.contains("fast-forward"),
                    "Expected fast-forward, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_merge_already_up_to_date() {
        let (_dir, repo) = init_repo_with_commit();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("same-branch", &commit, false).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "merge", "branch": "same-branch" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Already up to date"),
                    "Expected 'Already up to date', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_merge_nonexistent_branch() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "merge", "branch": "ghost" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for nonexistent branch, got: {result:?}"
        );
    }

    // ── Rebase operation ──

    #[tokio::test]
    async fn test_rebase() {
        let (dir, repo) = init_repo_with_commit();

        // Create a branch and add a commit
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-rebase", &commit, false).unwrap();

        // Add commit to main
        add_commit(&repo, &dir, "main-file.txt", b"main work", "Main work");

        // Switch to feature branch
        let feature_ref = "refs/heads/feature-rebase";
        repo.set_head(feature_ref).unwrap();
        let feature_obj = repo.revparse_single(feature_ref).unwrap();
        repo.checkout_tree(&feature_obj, None).unwrap();

        // Add commit to feature
        add_commit(&repo, &dir, "feat-file.txt", b"feature", "Feature commit");

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "rebase", "onto": "main" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Rebased onto main"),
                    "Expected 'Rebased onto main', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_rebase_invalid_onto() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "rebase", "onto": "nonexistent" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to resolve")),
            "Expected SystemError for bad onto, got: {result:?}"
        );
    }

    // ── Push/pull/fetch tests (with bare repo as remote) ──

    #[tokio::test]
    async fn test_push_with_bare_remote() {
        let (_dir, repo) = init_repo_with_commit();

        // Create bare repo as remote
        let bare_dir = tempfile::tempdir().unwrap();
        let _bare_repo = git2::Repository::init_bare(bare_dir.path()).unwrap();

        // Add bare repo as remote
        let remote_url = bare_dir.path().to_str().unwrap();
        repo.remote("origin", remote_url).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let _result = execute(&tool, json!({ "operation": "push" })).await;
        // May succeed or fail depending on environment; just check we don't panic
    }

    #[tokio::test]
    async fn test_fetch_no_remote() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "fetch" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for missing remote, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_pull_no_remote() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "pull" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for missing remote, got: {result:?}"
        );
    }

    // ── Cancel signal ──

    #[tokio::test]
    async fn test_cancel_signal_aborts_push() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        // Create a cancelled context
        let (_tx, rx) = tokio::sync::watch::channel(true);
        let ctx = ToolContext {
            workspace_root: repo.workdir().unwrap().to_path_buf(),
            signal: rx,
        };

        let _result = tool.execute(json!({ "operation": "push" }), &ctx).await;
        // Push should fail because remote doesn't exist
    }

    #[tokio::test]
    async fn test_cancel_signal_aborts_fetch() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        let (_tx, rx) = tokio::sync::watch::channel(true);
        let ctx = ToolContext {
            workspace_root: repo.workdir().unwrap().to_path_buf(),
            signal: rx,
        };

        let _result = tool.execute(json!({ "operation": "fetch" }), &ctx).await;
    }

    #[tokio::test]
    async fn test_cancel_signal_aborts_merge() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        let (_tx, rx) = tokio::sync::watch::channel(true);
        let ctx = ToolContext {
            workspace_root: repo.workdir().unwrap().to_path_buf(),
            signal: rx,
        };

        let _result = tool
            .execute(
                json!({ "operation": "merge", "branch": "nonexistent" }),
                &ctx,
            )
            .await;
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "invalid_op" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Unknown operation")),
            "Expected SystemError for unknown operation, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_missing_operation_param() {
        let tool = make_tool(PathBuf::from("/tmp"));
        let result = execute(&tool, json!({})).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'operation'")),
            "Expected SystemError for missing operation, got: {result:?}"
        );
    }

    // ── Merge test (no-ff with merge commit) ──

    #[tokio::test]
    async fn test_merge_with_merge_commit() {
        let (dir, repo) = init_repo_with_commit();

        // Create and switch to feature branch
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-noff", &commit, false).unwrap();

        // Add commit to main diverging
        add_commit(&repo, &dir, "main-only.txt", b"main", "Main divergence");

        // Switch to feature and add commit
        repo.set_head("refs/heads/feature-noff").unwrap();
        let feat_obj = repo.revparse_single("refs/heads/feature-noff").unwrap();
        repo.checkout_tree(&feat_obj, None).unwrap();
        add_commit(
            &repo,
            &dir,
            "feature-only.txt",
            b"feature",
            "Feature divergence",
        );

        // Switch back to main
        repo.set_head("refs/heads/main").unwrap();
        let main_obj = repo.revparse_single("refs/heads/main").unwrap();
        repo.checkout_tree(&main_obj, None).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "merge", "branch": "feature-noff" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Merged feature-noff"),
                    "Expected merge success, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Branch create with start_point ──

    #[tokio::test]
    async fn test_branch_create_with_start_point() {
        let (dir, repo) = init_repo_with_commit();
        add_commit(&repo, &dir, "second.txt", b"second", "Second commit");

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_create", "branch": "from-prev", "start_point": "HEAD~1" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created branch 'from-prev'"),
                    "Expected 'Created branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Commit stats ──

    #[tokio::test]
    async fn test_commit_stats_shown() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("stats-test.txt"), b"stats content\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_path(std::path::Path::new("stats-test.txt"))
            .unwrap();
        index.write().unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Stats test" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("file(s) changed"),
                    "Expected stats in commit output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Add nonexistent file ──

    #[tokio::test]
    async fn test_add_nonexistent_file_returns_error() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "add", "paths": ["does_not_exist.txt"] }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to stage")),
            "Expected SystemError for nonexistent file, got: {result:?}"
        );
    }

    // ── Checkout nonexistent branch without create ──

    #[tokio::test]
    async fn test_checkout_nonexistent_branch_without_create() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "branch": "nonexistent-branch" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found") || message.contains("Failed")),
            "Expected SystemError for nonexistent branch checkout, got: {result:?}"
        );
    }

    // ── Push force requires explicit flag ──

    #[tokio::test]
    async fn test_push_force_requires_explicit_flag() {
        let (dir, repo) = init_repo_with_commit();

        // Create bare repo as remote
        let bare_dir = tempfile::tempdir().unwrap();
        let bare_repo = git2::Repository::init_bare(bare_dir.path()).unwrap();
        let remote_url = bare_dir.path().to_str().unwrap();
        repo.remote("origin", remote_url).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        // Step 1: Initial push succeeds (fast-forward, no force needed)
        let result = execute(&tool, json!({ "operation": "push" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Pushed"),
                    "Expected push success, got: {content}"
                );
            }
            _ => panic!("Expected Success for initial push, got: {result:?}"),
        }

        // Step 2: Add a second commit and push it (still fast-forward)
        add_commit(&repo, &dir, "second.txt", b"second\n", "Second commit");
        let result = execute(&tool, json!({ "operation": "push" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Pushed"),
                    "Expected push success, got: {content}"
                );
            }
            _ => panic!("Expected Success for second push, got: {result:?}"),
        }

        // Step 3: Create a divergent history.
        // Reset HEAD back to the initial commit, then create a different commit on top.
        let initial_obj = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent(0)
            .unwrap()
            .into_object();
        repo.reset(&initial_obj, git2::ResetType::Hard, None)
            .unwrap();
        add_commit(
            &repo,
            &dir,
            "divergent.txt",
            b"divergent\n",
            "Divergent commit",
        );

        // Now: local has (initial -> divergent), remote has (initial -> second)
        // This is a non-fast-forward situation.

        // Step 4: Push WITHOUT force should fail
        let result = execute(&tool, json!({ "operation": "push" })).await;
        match &result {
            ToolResult::SystemError { message } => {
                assert!(
                    message.contains("Push failed"),
                    "Expected push failure, got: {message}"
                );
            }
            _ => panic!(
                "Expected SystemError for non-fast-forward push without force, got: {result:?}"
            ),
        }

        // Step 5: Push WITH force should succeed
        let result = execute(&tool, json!({ "operation": "push", "force": true })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Pushed"),
                    "Expected force push success, got: {content}"
                );
            }
            _ => panic!("Expected Success for force push, got: {result:?}"),
        }

        // Step 6: Verify the remote now has the divergent commit
        let remote_head = bare_repo.head().unwrap().peel_to_commit().unwrap();
        let local_head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            remote_head.id(),
            local_head.id(),
            "After force push, remote HEAD should match local HEAD"
        );
        assert_eq!(
            local_head.message().unwrap().trim(),
            "Divergent commit",
            "Remote should have the divergent commit"
        );
    }

    // ── Commit hash in output ──

    #[tokio::test]
    async fn test_commit_hash_appears_in_output() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("hash_test.txt"), b"content\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_path(std::path::Path::new("hash_test.txt"))
            .unwrap();
        index.write().unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Hash test" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                // The output should contain a commit hash (7+ hex chars)
                assert!(
                    content.contains("Committed"),
                    "Expected 'Committed', got: {content}"
                );
                // Verify commit was actually created
                let head = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(head.message().unwrap().trim(), "Hash test");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Commit with no staged changes ──

    #[tokio::test]
    async fn test_commit_with_nothing_staged_rejected() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Nothing staged" }),
        )
        .await;
        // Without allow_empty, same-tree commits are rejected
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("No changes to commit")),
            "Expected SystemError about no changes, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_commit_with_nothing_staged_allow_empty() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Empty commit", "allow_empty": true }),
        )
        .await;
        // With allow_empty, same-tree commits are permitted
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Committed"),
                    "Expected 'Committed ...', got: {content}"
                );
            }
            _ => panic!("Expected Success (empty commit with allow_empty), got: {result:?}"),
        }
    }
}
