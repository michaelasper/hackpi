use crate::config::VcsConfig;
use crate::github_api::GitHubClient;
use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::sync::Mutex;

pub mod branches;
pub mod ci;
pub mod comments;
pub mod pr_workflow;

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

    /// Get the workspace root path.
    pub(crate) fn workspace_root(&self) -> &PathBuf {
        &self.workspace_root
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
                        "pr_create", "pr_list", "pr_merge", "pr_checkout",
                        "issue_create", "issue_list", "issue_close", "issue_comment",
                        "label_add", "label_list",
                        "release_create", "release_list"
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
                    "description": "Body text for PR, issue, comment, or release"
                },
                "draft": {
                    "type": "boolean",
                    "description": "Create PR or release as draft"
                },
                "prerelease": {
                    "type": "boolean",
                    "description": "Create release as prerelease"
                },
                "name": {
                    "type": "string",
                    "description": "Release name (defaults to tag_name if omitted)"
                },
                "tag_name": {
                    "type": "string",
                    "description": "Tag name for release creation"
                },
                "state": {
                    "type": "string",
                    "enum": ["open", "closed", "all"],
                    "description": "Filter by state for list operations"
                },
                "number": {
                    "type": "integer",
                    "description": "PR or issue number for merge/close/comment/checkout/label operations"
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Labels for issue creation or label_add"
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
            "pr_create" => self.handle_pr_create(client, &params, &owner, &repo).await,
            "pr_list" => self.handle_pr_list(client, &params, &owner, &repo).await,
            "pr_merge" => self.handle_pr_merge(client, &params, &owner, &repo).await,
            "pr_checkout" => {
                self.handle_pr_checkout(client, &params, &owner, &repo)
                    .await
            }
            "issue_create" => {
                self.handle_issue_create(client, &params, &owner, &repo)
                    .await
            }
            "issue_list" => self.handle_issue_list(client, &params, &owner, &repo).await,
            "issue_close" => {
                self.handle_issue_close(client, &params, &owner, &repo)
                    .await
            }
            "issue_comment" => {
                self.handle_issue_comment(client, &params, &owner, &repo)
                    .await
            }
            "label_add" => self.handle_label_add(client, &params, &owner, &repo).await,
            "label_list" => self.handle_label_list(client, &params, &owner, &repo).await,
            "release_create" => {
                self.handle_release_create(client, &params, &owner, &repo)
                    .await
            }
            "release_list" => {
                self.handle_release_list(client, &params, &owner, &repo)
                    .await
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
    pub(crate) fn test_config() -> VcsConfig {
        VcsConfig {
            github_token: Some("test-token".into()),
            github_token_source: TokenSource::EnvVar("HACKPI_GITHUB_TOKEN".into()),
            github_base_url: String::new(), // Will be set per-test
            default_remote: "origin".into(),
            default_branch: "main".into(),
        }
    }

    pub(crate) fn test_ctx() -> ToolContext {
        let (_tx, rx) = watch::channel(false);
        ToolContext {
            workspace_root: std::env::temp_dir(),
            signal: rx,
        }
    }

    /// Setup token validation mock on a server
    pub(crate) async fn setup_token_mock(server: &MockServer) {
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
}
