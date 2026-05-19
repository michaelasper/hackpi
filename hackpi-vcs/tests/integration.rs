//! End-to-end integration tests for VCS tools.
//!
//! These tests exercise the full workflow across multiple tools using temporary
//! git repositories and wiremock for GitHub API mocking.

use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use hackpi_vcs::config::{TokenSource, VcsConfig};
use hackpi_vcs::git_read::GitReadTool;
use hackpi_vcs::git_write::GitWriteTool;
use hackpi_vcs::github::GitHubTool;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_ctx() -> ToolContext {
    ToolContext {
        workspace_root: std::env::temp_dir(),
        signal: tokio::sync::watch::channel(false).1,
    }
}

/// Helper: init a temporary git repo with user config.
fn init_repo() -> (tempfile::TempDir, git2::Repository) {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = git2::Repository::init(dir.path()).expect("init repo");
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Integration Test").unwrap();
    config
        .set_str("user.email", "integration@test.com")
        .unwrap();
    (dir, repo)
}

/// End-to-end: add → commit → status → log
#[tokio::test]
async fn test_e2e_add_commit_status_log() {
    let (dir, repo) = init_repo();

    // Create initial commit
    let initial_path = dir.path().join("README.md");
    std::fs::write(&initial_path, b"# Hello\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    index.write().unwrap();
    let oid = index.write_tree().unwrap();
    let sig = git2::Signature::now("Integration Test", "integration@test.com").unwrap();
    {
        let tree = repo.find_tree(oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();
    }

    let workspace = repo.workdir().unwrap().to_path_buf();
    let write_tool = GitWriteTool::new(workspace.clone());
    let read_tool = GitReadTool::new(workspace.clone());

    // Step 1: Add a new file
    std::fs::write(dir.path().join("feature.txt"), b"feature content\n").unwrap();
    let add_result = write_tool
        .execute(
            json!({ "operation": "add", "paths": ["feature.txt"] }),
            &test_ctx(),
        )
        .await;
    match &add_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("Added"),
                "Expected 'Added', got: {content}"
            );
        }
        _ => panic!("Add should succeed, got: {add_result:?}"),
    }

    // Step 2: Commit the change
    let commit_result = write_tool
        .execute(
            json!({ "operation": "commit", "message": "Add feature" }),
            &test_ctx(),
        )
        .await;
    match &commit_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("Committed"),
                "Expected 'Committed', got: {content}"
            );
            assert!(
                content.contains("Add feature"),
                "Expected commit message in output, got: {content}"
            );
        }
        _ => panic!("Commit should succeed, got: {commit_result:?}"),
    }

    // Step 3: Check status is clean
    let status_result = read_tool
        .execute(json!({ "operation": "status" }), &test_ctx())
        .await;
    match &status_result {
        ToolResult::Success { content } => {
            // After committing, working tree should be clean
            let lines: Vec<&str> = content.lines().collect();
            // Filter out empty lines
            let non_empty: Vec<&&str> = lines.iter().filter(|l| !l.is_empty()).collect();
            assert!(
                non_empty.is_empty(),
                "Expected clean status after commit, got: {content}"
            );
        }
        _ => panic!("Status should succeed, got: {status_result:?}"),
    }

    // Step 4: Verify log shows both commits
    let log_result = read_tool
        .execute(json!({ "operation": "log" }), &test_ctx())
        .await;
    match &log_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("Initial commit"),
                "Log should contain initial commit, got: {content}"
            );
            assert!(
                content.contains("Add feature"),
                "Log should contain new commit, got: {content}"
            );
        }
        _ => panic!("Log should succeed, got: {log_result:?}"),
    }
}

/// End-to-end: add → commit → branch → checkout → commit → log
#[tokio::test]
async fn test_e2e_branching_workflow() {
    let (dir, repo) = init_repo();

    // Create initial commit
    let initial_path = dir.path().join("README.md");
    std::fs::write(&initial_path, b"# Hello\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    index.write().unwrap();
    let oid = index.write_tree().unwrap();
    let sig = git2::Signature::now("Integration Test", "integration@test.com").unwrap();
    {
        let tree = repo.find_tree(oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();
    }

    let workspace = repo.workdir().unwrap().to_path_buf();
    let write_tool = GitWriteTool::new(workspace.clone());
    let read_tool = GitReadTool::new(workspace.clone());

    // Create and switch to feature branch
    let checkout_result = write_tool
        .execute(
            json!({ "operation": "checkout", "branch": "feature-test", "create": true }),
            &test_ctx(),
        )
        .await;
    assert!(
        matches!(&checkout_result, ToolResult::Success { .. }),
        "Checkout should succeed, got: {checkout_result:?}"
    );

    // Make a commit on feature branch
    std::fs::write(dir.path().join("feature-file.txt"), b"feature\n").unwrap();
    write_tool
        .execute(
            json!({ "operation": "add", "paths": ["feature-file.txt"] }),
            &test_ctx(),
        )
        .await;
    write_tool
        .execute(
            json!({ "operation": "commit", "message": "Feature work" }),
            &test_ctx(),
        )
        .await;

    // Verify branch list shows both branches
    let branch_result = read_tool
        .execute(json!({ "operation": "branch_list" }), &test_ctx())
        .await;
    match &branch_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("feature-test"),
                "Expected feature branch, got: {content}"
            );
        }
        _ => panic!("Branch list should succeed, got: {branch_result:?}"),
    }

    // Verify log shows feature commit
    let log_result = read_tool
        .execute(json!({ "operation": "log" }), &test_ctx())
        .await;
    match &log_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("Feature work"),
                "Log should contain feature commit, got: {content}"
            );
        }
        _ => panic!("Log should succeed, got: {log_result:?}"),
    }
}

/// End-to-end with GitHub mock: issue_create → issue_list → pr_create
#[tokio::test]
async fn test_e2e_github_workflow_with_mock() {
    let server = MockServer::start().await;

    // Setup token validation mock
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("Authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "login": "testuser",
            "id": 12345,
            "type": "User"
        })))
        .mount(&server)
        .await;

    // Setup issue_create mock
    Mock::given(method("POST"))
        .and(path("/repos/testorg/testrepo/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 42,
            "title": "Integration test issue",
            "state": "open",
            "html_url": "https://github.com/testorg/testrepo/issues/42",
            "body": "Created from integration test",
            "labels": [{"name": "bug", "color": "d73a4a"}],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;

    // Setup issue_list mock
    Mock::given(method("GET"))
        .and(path("/repos/testorg/testrepo/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 42,
                "title": "Integration test issue",
                "state": "open",
                "html_url": "https://github.com/testorg/testrepo/issues/42",
                "body": "Created from integration test",
                "labels": [{"name": "bug", "color": "d73a4a"}],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z"
            }
        ])))
        .mount(&server)
        .await;

    // Setup pr_create mock
    Mock::given(method("POST"))
        .and(path("/repos/testorg/testrepo/pulls"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 7,
            "title": "Fix integration test issue",
            "state": "open",
            "html_url": "https://github.com/testorg/testrepo/pull/7",
            "body": "Fixes #42",
            "head": { "label": "testorg:fix-branch", "ref": "fix-branch", "sha": "abc123" },
            "base": { "label": "testorg:main", "ref": "main", "sha": "def456" },
            "draft": false,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "user": { "login": "testuser" }
        })))
        .mount(&server)
        .await;

    let config = VcsConfig {
        github_token: Some("test-token".into()),
        github_token_source: TokenSource::EnvVar("HACKPI_GITHUB_TOKEN".into()),
        github_base_url: server.uri(),
        default_remote: "origin".into(),
        default_branch: "main".into(),
    };

    let tool = GitHubTool::new(std::env::temp_dir(), config);

    // Step 1: Create an issue
    let issue_result = tool
        .execute(
            json!({
                "operation": "issue_create",
                "owner": "testorg",
                "repo": "testrepo",
                "title": "Integration test issue",
                "body": "Created from integration test",
                "labels": ["bug"]
            }),
            &test_ctx(),
        )
        .await;
    match &issue_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("#42"),
                "Expected issue #42 in output, got: {content}"
            );
            assert!(
                content.contains("Integration test issue"),
                "Expected title in output, got: {content}"
            );
        }
        _ => panic!("Issue create should succeed, got: {issue_result:?}"),
    }

    // Step 2: List issues
    let list_result = tool
        .execute(
            json!({
                "operation": "issue_list",
                "owner": "testorg",
                "repo": "testrepo"
            }),
            &test_ctx(),
        )
        .await;
    match &list_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("#42"),
                "Expected issue #42 in list, got: {content}"
            );
        }
        _ => panic!("Issue list should succeed, got: {list_result:?}"),
    }

    // Step 3: Create a PR to fix the issue
    let pr_result = tool
        .execute(
            json!({
                "operation": "pr_create",
                "owner": "testorg",
                "repo": "testrepo",
                "title": "Fix integration test issue",
                "head": "fix-branch",
                "base": "main",
                "body": "Fixes #42"
            }),
            &test_ctx(),
        )
        .await;
    match &pr_result {
        ToolResult::Success { content } => {
            assert!(
                content.contains("#7"),
                "Expected PR #7 in output, got: {content}"
            );
            assert!(
                content.contains("Fix integration test issue"),
                "Expected PR title in output, got: {content}"
            );
        }
        _ => panic!("PR create should succeed, got: {pr_result:?}"),
    }
}

/// End-to-end: stash → status clean → stash_pop → changes restored
#[tokio::test]
async fn test_e2e_stash_workflow() {
    let (dir, repo) = init_repo();

    // Create initial commit
    let initial_path = dir.path().join("README.md");
    std::fs::write(&initial_path, b"# Hello\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    index.write().unwrap();
    let oid = index.write_tree().unwrap();
    let sig = git2::Signature::now("Integration Test", "integration@test.com").unwrap();
    {
        let tree = repo.find_tree(oid).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();
    }

    let workspace = repo.workdir().unwrap().to_path_buf();
    let write_tool = GitWriteTool::new(workspace.clone());
    let read_tool = GitReadTool::new(workspace.clone());

    // Modify a tracked file
    std::fs::write(dir.path().join("README.md"), b"# Modified\n").unwrap();

    // Stash the change
    let stash_result = write_tool
        .execute(
            json!({ "operation": "stash", "message": "WIP" }),
            &test_ctx(),
        )
        .await;
    assert!(
        matches!(&stash_result, ToolResult::Success { .. }),
        "Stash should succeed, got: {stash_result:?}"
    );

    // Verify status is clean
    let status_result = read_tool
        .execute(json!({ "operation": "status" }), &test_ctx())
        .await;
    match &status_result {
        ToolResult::Success { content } => {
            let non_empty: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
            assert!(
                non_empty.is_empty(),
                "Expected clean status after stash, got: {content}"
            );
        }
        _ => panic!("Status should succeed, got: {status_result:?}"),
    }

    // Pop the stash
    let pop_result = write_tool
        .execute(json!({ "operation": "stash_pop" }), &test_ctx())
        .await;
    assert!(
        matches!(&pop_result, ToolResult::Success { .. }),
        "Stash pop should succeed, got: {pop_result:?}"
    );

    // Verify changes are restored
    let readme_content = std::fs::read_to_string(dir.path().join("README.md")).unwrap();
    assert_eq!(
        readme_content, "# Modified\n",
        "Changes should be restored after stash pop"
    );
}
