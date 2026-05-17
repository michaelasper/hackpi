use crate::config::VcsConfig;
use crate::github_api::GitHubClient;
use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::sync::Mutex;

pub struct GitHubTool {
    workspace_root: PathBuf,
    client: Mutex<Option<GitHubClient>>,
    config: VcsConfig,
}

impl GitHubTool {
    pub fn new(workspace_root: PathBuf, config: VcsConfig) -> Self {
        Self {
            workspace_root,
            client: Mutex::new(None),
            config,
        }
    }

    /// Ensure the client is created and token is validated.
    /// Returns a guard that can be used to access the client.
    async fn ensure_client(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, Option<GitHubClient>>, ToolResult> {
        let mut guard = self.client.lock().await;
        if guard.is_some() {
            return Ok(guard);
        }

        let token = self
            .config
            .github_token
            .as_ref()
            .ok_or_else(|| ToolResult::SystemError {
                message: format!(
                    "GitHub authentication failed.\n\
                    Set HACKPI_GITHUB_TOKEN or GITHUB_TOKEN env var.\n\
                    Current token source: {:?}",
                    self.config.github_token_source
                ),
            })?;

        let client = GitHubClient::new(token, &self.config.github_base_url).map_err(|e| {
            ToolResult::SystemError {
                message: format!("Failed to create GitHub client: {e}"),
            }
        })?;

        // Validate token on first use
        client
            .validate_token()
            .await
            .map_err(|e| ToolResult::SystemError {
                message: format!(
                    "GitHub token validation failed: {e}\n\
                    Check that your token is valid and has the required permissions.\n\
                    Token source: {:?}",
                    self.config.github_token_source
                ),
            })?;

        *guard = Some(client);
        Ok(guard)
    }

    /// Infer owner and repo from the git remote URL, falling back to params if provided.
    fn infer_owner_repo(&self, params: &Value) -> Result<(String, String), ToolResult> {
        // If explicitly provided in params, use those
        if let (Some(owner), Some(repo)) = (
            params.get("owner").and_then(|v| v.as_str()),
            params.get("repo").and_then(|v| v.as_str()),
        ) {
            return Ok((owner.to_string(), repo.to_string()));
        }

        // Otherwise, infer from git remote
        VcsConfig::infer_owner_repo(&self.workspace_root).ok_or_else(|| ToolResult::SystemError {
            message: "Could not determine owner/repo. Provide 'owner' and 'repo' parameters, \
                          or run from a git repository with an 'origin' remote pointing to GitHub."
                .into(),
        })
    }
}

#[async_trait]
impl Tool for GitHubTool {
    fn name(&self) -> &str {
        "github"
    }

    fn description(&self) -> &str {
        "GitHub operations: PRs, issues, and comments"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "pr_create", "pr_list", "pr_merge",
                        "issue_create", "issue_list", "issue_close", "issue_comment"
                    ],
                    "description": "The GitHub operation to perform"
                },
                "owner": {
                    "type": "string",
                    "description": "Repository owner (inferred from git remote if omitted)"
                },
                "repo": {
                    "type": "string",
                    "description": "Repository name (inferred from git remote if omitted)"
                },
                "title": {
                    "type": "string",
                    "description": "Title for PR or issue creation"
                },
                "head": {
                    "type": "string",
                    "description": "Head branch for PR (e.g. feature-branch)"
                },
                "base": {
                    "type": "string",
                    "description": "Base branch for PR (e.g. main)"
                },
                "body": {
                    "type": "string",
                    "description": "Body text for PR, issue, or comment"
                },
                "draft": {
                    "type": "boolean",
                    "description": "Create PR as draft"
                },
                "state": {
                    "type": "string",
                    "enum": ["open", "closed", "all"],
                    "description": "Filter by state for list operations"
                },
                "number": {
                    "type": "integer",
                    "description": "PR or issue number for merge/close/comment operations"
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Labels for issue creation"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let operation = match params.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'operation' parameter.".into(),
                }
            }
        };

        // Get or create the client (locks mutex internally)
        let guard = match self.ensure_client().await {
            Ok(g) => g,
            Err(e) => return e,
        };

        let client = match guard.as_ref() {
            Some(c) => c,
            None => {
                return ToolResult::SystemError {
                    message: "GitHub client not initialized.".into(),
                }
            }
        };

        // Infer owner/repo
        let (owner, repo) = match self.infer_owner_repo(&params) {
            Ok(pair) => pair,
            Err(e) => return e,
        };

        match operation {
            "pr_create" => {
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
                    .pr_create(&owner, &repo, title, head, base, body, draft)
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

            "pr_list" => {
                let state = params.get("state").and_then(|v| v.as_str());
                match client.pr_list(&owner, &repo, state).await {
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

            "pr_merge" => {
                let number = match params.get("number").and_then(|v| v.as_u64()) {
                    Some(n) => n,
                    None => {
                        return ToolResult::SystemError {
                            message: "Missing 'number' parameter for pr_merge.".into(),
                        }
                    }
                };
                match client.pr_merge(&owner, &repo, number).await {
                    Ok(()) => ToolResult::Success {
                        content: format!("Merged PR #{}", number),
                    },
                    Err(e) => ToolResult::SystemError { message: e },
                }
            }

            "issue_create" => {
                let title = match params.get("title").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => {
                        return ToolResult::SystemError {
                            message: "Missing 'title' parameter for issue_create.".into(),
                        }
                    }
                };
                let body = params.get("body").and_then(|v| v.as_str());
                let labels: Option<Vec<&str>> = params
                    .get("labels")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

                match client
                    .issue_create(&owner, &repo, title, body, labels)
                    .await
                {
                    Ok(issue) => ToolResult::Success {
                        content: format!(
                            "Created issue #{}: {}\n{}\nState: {}",
                            issue.number, issue.title, issue.html_url, issue.state
                        ),
                    },
                    Err(e) => ToolResult::SystemError { message: e },
                }
            }

            "issue_list" => {
                let state = params.get("state").and_then(|v| v.as_str());
                match client.issue_list(&owner, &repo, state).await {
                    Ok(issues) => {
                        if issues.is_empty() {
                            return ToolResult::Success {
                                content: "No issues found.".into(),
                            };
                        }
                        let mut output = String::new();
                        for issue in &issues {
                            use std::fmt::Write;
                            let label_str = if issue.labels.is_empty() {
                                String::new()
                            } else {
                                let names: Vec<&str> =
                                    issue.labels.iter().map(|l| l.name.as_str()).collect();
                                format!(" [{}]", names.join(", "))
                            };
                            let _ = writeln!(
                                output,
                                "#{} {}{} ({})",
                                issue.number, issue.title, label_str, issue.state
                            );
                            let _ = writeln!(output, "   {}", issue.html_url);
                        }
                        ToolResult::Success { content: output }
                    }
                    Err(e) => ToolResult::SystemError { message: e },
                }
            }

            "issue_close" => {
                let number = match params.get("number").and_then(|v| v.as_u64()) {
                    Some(n) => n,
                    None => {
                        return ToolResult::SystemError {
                            message: "Missing 'number' parameter for issue_close.".into(),
                        }
                    }
                };
                match client.issue_close(&owner, &repo, number).await {
                    Ok(issue) => ToolResult::Success {
                        content: format!("Closed issue #{}: {}", issue.number, issue.title),
                    },
                    Err(e) => ToolResult::SystemError { message: e },
                }
            }

            "issue_comment" => {
                let number = match params.get("number").and_then(|v| v.as_u64()) {
                    Some(n) => n,
                    None => {
                        return ToolResult::SystemError {
                            message: "Missing 'number' parameter for issue_comment.".into(),
                        }
                    }
                };
                let body = match params.get("body").and_then(|v| v.as_str()) {
                    Some(b) => b,
                    None => {
                        return ToolResult::SystemError {
                            message: "Missing 'body' parameter for issue_comment.".into(),
                        }
                    }
                };
                match client.issue_comment(&owner, &repo, number, body).await {
                    Ok(()) => ToolResult::Success {
                        content: format!("Commented on issue #{}", number),
                    },
                    Err(e) => ToolResult::SystemError { message: e },
                }
            }

            _ => ToolResult::SystemError {
                message: format!("Unknown operation: {operation}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TokenSource;
    use hackpi_core::tools::ToolContext;
    use serde_json::json;
    use tokio::sync::watch;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create a VcsConfig with a test token
    fn test_config() -> VcsConfig {
        VcsConfig {
            github_token: Some("test-token".into()),
            github_token_source: TokenSource::EnvVar("HACKPI_GITHUB_TOKEN".into()),
            github_base_url: String::new(), // Will be set per-test
            default_remote: "origin".into(),
            default_branch: "main".into(),
        }
    }

    fn test_ctx() -> ToolContext {
        let (_tx, rx) = watch::channel(false);
        ToolContext {
            workspace_root: std::env::temp_dir(),
            conversation_id: String::new(),
            signal: rx,
        }
    }

    /// Setup token validation mock on a server
    async fn setup_token_mock(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "login": "testuser",
                "id": 12345,
                "type": "User"
            })))
            .mount(server)
            .await;
    }

    // ── Basic metadata ──

    #[test]
    fn test_name() {
        let config = test_config();
        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        assert_eq!(tool.name(), "github");
    }

    #[test]
    fn test_description() {
        let config = test_config();
        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        assert!(tool.description().contains("GitHub"));
        assert!(tool.description().contains("PRs"));
        assert!(tool.description().contains("issues"));
    }

    #[test]
    fn test_input_schema() {
        let config = test_config();
        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let schema = tool.input_schema();
        assert_eq!(schema.get("additionalProperties"), Some(&json!(false)));
        assert!(schema.get("properties").unwrap().get("operation").is_some());
    }

    // ── Missing operation ──

    #[tokio::test]
    async fn test_missing_operation() {
        let config = test_config();
        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool.execute(json!({}), &test_ctx()).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'operation'")),
            "Expected SystemError for missing operation, got: {result:?}"
        );
    }

    // ── Missing token ──

    #[tokio::test]
    async fn test_missing_token_returns_system_error() {
        let config = VcsConfig {
            github_token: None,
            github_token_source: TokenSource::None,
            github_base_url: "https://api.github.com".into(),
            default_remote: "origin".into(),
            default_branch: "main".into(),
        };
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
            ToolResult::SystemError { message } => {
                assert!(
                    message.contains("GitHub authentication failed"),
                    "Expected auth error, got: {message}"
                );
                assert!(
                    message.contains("HACKPI_GITHUB_TOKEN"),
                    "Expected HACKPI_GITHUB_TOKEN hint, got: {message}"
                );
            }
            _ => panic!("Expected SystemError, got: {result:?}"),
        }
    }

    // ── No owner/repo and no git remote ──

    #[tokio::test]
    async fn test_missing_owner_repo_no_git_remote() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        let tmp = tempfile::tempdir().unwrap();
        let tool = GitHubTool::new(tmp.path().to_path_buf(), config);
        let result = tool
            .execute(json!({ "operation": "pr_list" }), &test_ctx())
            .await;
        match &result {
            ToolResult::SystemError { message } => {
                assert!(
                    message.contains("Could not determine owner/repo"),
                    "Expected owner/repo error, got: {message}"
                );
            }
            _ => panic!("Expected SystemError, got: {result:?}"),
        }
    }

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

    // ── issue_create ──

    #[tokio::test]
    async fn test_issue_create_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 10,
                "title": "Test Issue",
                "state": "open",
                "html_url": "https://github.com/owner/repo/issues/10",
                "body": "desc",
                "labels": [{"name": "bug", "color": "red"}],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "issue_create",
                    "owner": "owner",
                    "repo": "repo",
                    "title": "Test Issue",
                    "body": "desc",
                    "labels": ["bug"]
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created issue #10"),
                    "Expected 'Created issue #10', got: {content}"
                );
                assert!(
                    content.contains("Test Issue"),
                    "Expected title in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── issue_list ──

    #[tokio::test]
    async fn test_issue_list_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/issues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 1, "title": "Bug report", "state": "open",
                    "html_url": "https://github.com/owner/repo/issues/1",
                    "body": null, "labels": [{"name": "bug", "color": "red"}],
                    "created_at": "2024-01-01T00:00:00Z",
                    "updated_at": "2024-01-01T00:00:00Z"
                },
                {
                    "number": 2, "title": "Feature request", "state": "closed",
                    "html_url": "https://github.com/owner/repo/issues/2",
                    "body": null, "labels": [],
                    "created_at": "2024-01-02T00:00:00Z",
                    "updated_at": "2024-01-02T00:00:00Z"
                }
            ])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "issue_list",
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
                    "Expected issue #1 in output, got: {content}"
                );
                assert!(
                    content.contains("Bug report"),
                    "Expected 'Bug report' in output, got: {content}"
                );
                assert!(
                    content.contains("[bug]"),
                    "Expected labels in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_issue_list_empty() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/issues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "issue_list",
                    "owner": "owner",
                    "repo": "repo"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("No issues found"),
                    "Expected empty message, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── issue_close ──

    #[tokio::test]
    async fn test_issue_close_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("PATCH"))
            .and(path("/repos/owner/repo/issues/5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "number": 5,
                "title": "Issue to Close",
                "state": "closed",
                "html_url": "https://github.com/owner/repo/issues/5",
                "body": null,
                "labels": [],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-02T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "issue_close",
                    "owner": "owner",
                    "repo": "repo",
                    "number": 5
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Closed issue #5"),
                    "Expected 'Closed issue #5', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── issue_comment ──

    #[tokio::test]
    async fn test_issue_comment_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/5/comments"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "issue_comment",
                    "owner": "owner",
                    "repo": "repo",
                    "number": 5,
                    "body": "Looks good!"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Commented on issue #5"),
                    "Expected 'Commented on issue #5', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_issue_comment_missing_body() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "issue_comment",
                    "owner": "owner",
                    "repo": "repo",
                    "number": 5
                    // Missing body
                }),
                &test_ctx(),
            )
            .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'body'")),
            "Expected SystemError for missing body, got: {result:?}"
        );
    }
}
