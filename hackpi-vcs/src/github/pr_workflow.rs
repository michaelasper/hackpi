use crate::github_api::GitHubClient;
use hackpi_core::tools::ToolResult;
use serde_json::Value;

impl super::GitHubTool {
    pub(super) async fn handle_pr_create(
        &self,
        client: &GitHubClient,
        params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        let title = match params.get("title").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'title' parameter for pr_create.".into(),
                }
            }
        };
        let head = match params.get("head").and_then(|v| v.as_str()) {
            Some(h) => h,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'head' parameter for pr_create.".into(),
                }
            }
        };
        let base = match params.get("base").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'base' parameter for pr_create.".into(),
                }
            }
        };
        let body = params.get("body").and_then(|v| v.as_str());
        let draft = params.get("draft").and_then(|v| v.as_bool());

        match client
            .pr_create(owner, repo, title, head, base, body, draft)
            .await
        {
            Ok(pr) => ToolResult::Success {
                content: format!(
                    "Created PR #{}: {}\n{}\nState: {}",
                    pr.number, pr.title, pr.html_url, pr.state
                ),
            },
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_pr_list(
        &self,
        client: &GitHubClient,
        params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        let state = params.get("state").and_then(|v| v.as_str());
        match client.pr_list(owner, repo, state).await {
            Ok(prs) => {
                if prs.is_empty() {
                    return ToolResult::Success {
                        content: "No pull requests found.".into(),
                    };
                }
                let mut output = String::new();
                for pr in &prs {
                    use std::fmt::Write;
                    let draft_label = if pr.draft.unwrap_or(false) {
                        " [DRAFT]"
                    } else {
                        ""
                    };
                    let _ = writeln!(
                        output,
                        "#{} {} ({}){}",
                        pr.number, pr.title, pr.state, draft_label
                    );
                    let _ = writeln!(output, "   {} by @{}", pr.html_url, pr.user.login);
                }
                ToolResult::Success { content: output }
            }
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_pr_merge(
        &self,
        client: &GitHubClient,
        params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        let number = match params.get("number").and_then(|v| v.as_u64()) {
            Some(n) => n,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'number' parameter for pr_merge.".into(),
                }
            }
        };
        match client.pr_merge(owner, repo, number).await {
            Ok(()) => ToolResult::Success {
                content: format!("Merged PR #{}", number),
            },
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_pr_checkout(
        &self,
        client: &GitHubClient,
        params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        let number = match params.get("number").and_then(|v| v.as_u64()) {
            Some(n) => n,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'number' parameter for pr_checkout.".into(),
                }
            }
        };
        match client
            .pr_checkout(owner, repo, number, self.workspace_root())
            .await
        {
            Ok(msg) => ToolResult::Success { content: msg },
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_release_create(
        &self,
        client: &GitHubClient,
        params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        let tag_name = match params.get("tag_name").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'tag_name' parameter for release_create.".into(),
                }
            }
        };
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(tag_name);
        let body = params.get("body").and_then(|v| v.as_str());
        let draft = params
            .get("draft")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let prerelease = params
            .get("prerelease")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        match client
            .release_create(owner, repo, tag_name, name, body, draft, prerelease)
            .await
        {
            Ok(release) => ToolResult::Success {
                content: format!(
                    "Created release {}\nURL: {}",
                    release.tag_name, release.html_url
                ),
            },
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_release_list(
        &self,
        client: &GitHubClient,
        _params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        match client.release_list(owner, repo).await {
            Ok(releases) => {
                if releases.is_empty() {
                    return ToolResult::Success {
                        content: "No releases found.".into(),
                    };
                }
                let mut output = String::new();
                for release in &releases {
                    use std::fmt::Write;
                    let date = release
                        .published_at
                        .as_deref()
                        .or(Some(&release.created_at))
                        .map(|d| {
                            // Trim to date portion
                            let date_str: String = d.chars().take(10).collect();
                            date_str
                        })
                        .unwrap_or_default();
                    let _ = writeln!(output, "{} ({}): {}", release.tag_name, date, release.name);
                }
                ToolResult::Success { content: output }
            }
            Err(e) => ToolResult::SystemError { message: e },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{setup_token_mock, test_config, test_ctx};
    use super::super::GitHubTool;
    use hackpi_core::tools::{Tool, ToolResult};
    use serde_json::json;
    use std::path::PathBuf;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── pr_create ──

    #[tokio::test]
    async fn test_pr_create_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 1,
                "title": "Test PR",
                "state": "open",
                "html_url": "https://github.com/owner/repo/pull/1",
                "body": "desc",
                "head": { "label": "owner:feature", "ref": "feature", "sha": "a" },
                "base": { "label": "owner:main", "ref": "main", "sha": "b" },
                "draft": false,
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "user": { "login": "testuser" }
            })))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_create",
                    "owner": "owner",
                    "repo": "repo",
                    "title": "Test PR",
                    "head": "feature",
                    "base": "main",
                    "body": "desc",
                    "draft": false
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created PR #1"),
                    "Expected 'Created PR #1', got: {content}"
                );
                assert!(
                    content.contains("Test PR"),
                    "Expected title in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_pr_create_missing_required_params() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_create",
                    "owner": "owner",
                    "repo": "repo"
                    // Missing title, head, base
                }),
                &test_ctx(),
            )
            .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing")),
            "Expected SystemError for missing params, got: {result:?}"
        );
    }

    // ── pr_list ──

    #[tokio::test]
    async fn test_pr_list_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 1, "title": "PR One", "state": "open",
                    "html_url": "https://github.com/owner/repo/pull/1",
                    "body": null,
                    "head": { "label": "owner:f1", "ref": "f1", "sha": "a" },
                    "base": { "label": "owner:main", "ref": "main", "sha": "b" },
                    "draft": false,
                    "created_at": "2024-01-01T00:00:00Z",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "user": { "login": "user1" }
                },
                {
                    "number": 2, "title": "PR Two", "state": "closed",
                    "html_url": "https://github.com/owner/repo/pull/2",
                    "body": null,
                    "head": { "label": "owner:f2", "ref": "f2", "sha": "c" },
                    "base": { "label": "owner:main", "ref": "main", "sha": "d" },
                    "draft": false,
                    "created_at": "2024-01-02T00:00:00Z",
                    "updated_at": "2024-01-02T00:00:00Z",
                    "user": { "login": "user2" }
                }
            ])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_list",
                    "owner": "owner",
                    "repo": "repo"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("#1"),
                    "Expected PR #1 in output, got: {content}"
                );
                assert!(
                    content.contains("#2"),
                    "Expected PR #2 in output, got: {content}"
                );
                assert!(
                    content.contains("PR One"),
                    "Expected 'PR One' in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_pr_list_empty() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_list",
                    "owner": "owner",
                    "repo": "repo"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("No pull requests found"),
                    "Expected empty message, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── pr_merge ──

    #[tokio::test]
    async fn test_pr_merge_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("PUT"))
            .and(path("/repos/owner/repo/pulls/1/merge"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_merge",
                    "owner": "owner",
                    "repo": "repo",
                    "number": 1
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Merged PR #1"),
                    "Expected 'Merged PR #1', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_pr_merge_missing_number() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_merge",
                    "owner": "owner",
                    "repo": "repo"
                    // Missing number
                }),
                &test_ctx(),
            )
            .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'number'")),
            "Expected SystemError for missing number, got: {result:?}"
        );
    }

    // ── pr_checkout ──

    #[tokio::test]
    async fn test_pr_checkout_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        let head_sha = "abc123def4567890123456789012345678901234";
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "number": 42,
                "title": "Feature branch",
                "state": "open",
                "html_url": "https://github.com/owner/repo/pull/42",
                "body": null,
                "head": {
                    "label": "owner:feature-x",
                    "ref": "feature-x",
                    "sha": head_sha
                },
                "base": {
                    "label": "owner:main",
                    "ref": "main",
                    "sha": "789012345678"
                },
                "draft": false,
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "user": { "login": "testuser" }
            })))
            .mount(&server)
            .await;

        // Create a bare repo as remote
        let remote_dir = tempfile::tempdir().unwrap();
        let bare_repo = git2::Repository::init_bare(remote_dir.path()).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let tree_id = {
            let mut index = bare_repo.index().unwrap();
            let oid = index.write_tree().unwrap();
            bare_repo.find_tree(oid).unwrap().id()
        };
        let initial_oid = bare_repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "initial",
                &bare_repo.find_tree(tree_id).unwrap(),
                &[],
            )
            .unwrap();
        bare_repo
            .reference("refs/pull/42/head", initial_oid, true, "PR ref")
            .unwrap();

        // Create local repo
        let tmp = tempfile::tempdir().unwrap();
        let local_repo = git2::Repository::init(tmp.path()).unwrap();
        let tree_id = {
            let mut index = local_repo.index().unwrap();
            let oid = index.write_tree().unwrap();
            local_repo.find_tree(oid).unwrap().id()
        };
        local_repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "initial",
                &local_repo.find_tree(tree_id).unwrap(),
                &[],
            )
            .unwrap();
        local_repo
            .remote("origin", remote_dir.path().to_str().unwrap())
            .unwrap();

        let tool = GitHubTool::new(tmp.path().to_path_buf(), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_checkout",
                    "owner": "owner",
                    "repo": "repo",
                    "number": 42
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("42"),
                    "Expected PR number in output, got: {content}"
                );
                assert!(
                    content.contains("feature-x"),
                    "Expected branch name in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_pr_checkout_missing_number() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "pr_checkout",
                    "owner": "owner",
                    "repo": "repo"
                    // Missing number
                }),
                &test_ctx(),
            )
            .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'number'")),
            "Expected SystemError for missing number, got: {result:?}"
        );
    }

    // ── release_create ──

    #[tokio::test]
    async fn test_release_create_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "tag_name": "v1.0.0",
                "name": "Release v1.0.0",
                "body": "First release",
                "draft": false,
                "prerelease": false,
                "html_url": "https://github.com/owner/repo/releases/tag/v1.0.0",
                "created_at": "2024-01-01T00:00:00Z",
                "published_at": "2024-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "release_create",
                    "owner": "owner",
                    "repo": "repo",
                    "tag_name": "v1.0.0",
                    "name": "Release v1.0.0",
                    "body": "First release",
                    "draft": false,
                    "prerelease": false
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created release v1.0.0"),
                    "Expected 'Created release v1.0.0' in output, got: {content}"
                );
                assert!(
                    content.contains("https://github.com/owner/repo/releases/tag/v1.0.0"),
                    "Expected URL in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_release_create_defaults_name_from_tag_name() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "tag_name": "v2.0.0",
                "name": "v2.0.0",
                "body": null,
                "draft": false,
                "prerelease": false,
                "html_url": "https://github.com/owner/repo/releases/tag/v2.0.0",
                "created_at": "2024-06-01T00:00:00Z",
                "published_at": "2024-06-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        // No "name" parameter — should fall back to tag_name
        let result = tool
            .execute(
                json!({
                    "operation": "release_create",
                    "owner": "owner",
                    "repo": "repo",
                    "tag_name": "v2.0.0",
                    "draft": false,
                    "prerelease": false
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created release v2.0.0"),
                    "Expected 'Created release v2.0.0' in output, got: {content}"
                );
            }
            _ => panic!("Expected Success when name is omitted, got: {result:?}"),
        }
    }

    // ── release_list ──

    #[tokio::test]
    async fn test_release_list_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "tag_name": "v2.0.0",
                    "name": "Version 2",
                    "body": "Major release",
                    "draft": false,
                    "prerelease": false,
                    "html_url": "https://github.com/owner/repo/releases/tag/v2.0.0",
                    "created_at": "2024-02-01T00:00:00Z",
                    "published_at": "2024-02-01T00:00:00Z"
                },
                {
                    "tag_name": "v1.0.0",
                    "name": "Version 1",
                    "body": "Initial release",
                    "draft": false,
                    "prerelease": false,
                    "html_url": "https://github.com/owner/repo/releases/tag/v1.0.0",
                    "created_at": "2024-01-01T00:00:00Z",
                    "published_at": "2024-01-01T00:00:00Z"
                }
            ])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "release_list",
                    "owner": "owner",
                    "repo": "repo"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("v2.0.0"),
                    "Expected v2.0.0 in output, got: {content}"
                );
                assert!(
                    content.contains("v1.0.0"),
                    "Expected v1.0.0 in output, got: {content}"
                );
                assert!(
                    content.contains("Version 2"),
                    "Expected Version 2 in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_release_list_empty() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "release_list",
                    "owner": "owner",
                    "repo": "repo"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("No releases found"),
                    "Expected empty message, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }
}
