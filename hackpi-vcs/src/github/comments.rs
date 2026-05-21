use crate::github_api::GitHubClient;
use hackpi_core::tools::ToolResult;
use serde_json::Value;

impl super::GitHubTool {
    pub(super) async fn handle_issue_create(
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
                    message: "Missing 'title' parameter for issue_create.".into(),
                }
            }
        };
        let body = params.get("body").and_then(|v| v.as_str());
        let labels: Option<Vec<&str>> = params
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect());

        match client.issue_create(owner, repo, title, body, labels).await {
            Ok(issue) => ToolResult::Success {
                content: format!(
                    "Created issue #{}: {}\n{}\nState: {}",
                    issue.number, issue.title, issue.html_url, issue.state
                ),
            },
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_issue_list(
        &self,
        client: &GitHubClient,
        params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        let state = params.get("state").and_then(|v| v.as_str());
        match client.issue_list(owner, repo, state).await {
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

    pub(super) async fn handle_issue_close(
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
                    message: "Missing 'number' parameter for issue_close.".into(),
                }
            }
        };
        match client.issue_close(owner, repo, number).await {
            Ok(issue) => ToolResult::Success {
                content: format!("Closed issue #{}: {}", issue.number, issue.title),
            },
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_issue_comment(
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
        match client.issue_comment(owner, repo, number, body).await {
            Ok(()) => ToolResult::Success {
                content: format!("Commented on issue #{}", number),
            },
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_label_add(
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
                    message: "Missing 'number' parameter for label_add.".into(),
                }
            }
        };
        let labels: Vec<String> = params
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if labels.is_empty() {
            return ToolResult::SystemError {
                message: "Missing 'labels' parameter for label_add.".into(),
            };
        }
        match client.label_add(owner, repo, number, labels).await {
            Ok(added) => {
                let label_names: Vec<&str> = added.iter().map(|l| l.name.as_str()).collect();
                ToolResult::Success {
                    content: format!("Added labels: {} to #{}", label_names.join(", "), number),
                }
            }
            Err(e) => ToolResult::SystemError { message: e },
        }
    }

    pub(super) async fn handle_label_list(
        &self,
        client: &GitHubClient,
        _params: &Value,
        owner: &str,
        repo: &str,
    ) -> ToolResult {
        match client.label_list(owner, repo).await {
            Ok(labels) => {
                if labels.is_empty() {
                    return ToolResult::Success {
                        content: "No labels found.".into(),
                    };
                }
                let mut output = String::new();
                for (i, label) in labels.iter().enumerate() {
                    use std::fmt::Write;
                    let color_str = label
                        .color
                        .as_deref()
                        .map(|c| format!(" (#{c})"))
                        .unwrap_or_default();
                    let _ = writeln!(output, "{}. {}{}", i + 1, label.name, color_str);
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

    // ── label_add ──

    #[tokio::test]
    async fn test_label_add_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/15/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "bug", "color": "d73a4a", "description": "Bug report"},
                {"name": "feature", "color": "0e8a16", "description": "Feature request"}
            ])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "label_add",
                    "owner": "owner",
                    "repo": "repo",
                    "number": 15,
                    "labels": ["bug", "feature"]
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Added labels"),
                    "Expected 'Added labels' in output, got: {content}"
                );
                assert!(
                    content.contains("bug"),
                    "Expected 'bug' in output, got: {content}"
                );
                assert!(
                    content.contains("feature"),
                    "Expected 'feature' in output, got: {content}"
                );
                assert!(
                    content.contains("#15"),
                    "Expected issue number in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── label_list ──

    #[tokio::test]
    async fn test_label_list_success() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "bug", "color": "d73a4a", "description": "Bug report"},
                {"name": "feature", "color": "0e8a16", "description": "Feature request"}
            ])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "label_list",
                    "owner": "owner",
                    "repo": "repo"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("bug"),
                    "Expected 'bug' in output, got: {content}"
                );
                assert!(
                    content.contains("feature"),
                    "Expected 'feature' in output, got: {content}"
                );
                assert!(
                    content.contains("#d73a4a"),
                    "Expected color in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_label_list_empty() {
        let server = MockServer::start().await;
        let mut config = test_config();
        config.github_base_url = server.uri();
        setup_token_mock(&server).await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let tool = GitHubTool::new(PathBuf::from("/tmp"), config);
        let result = tool
            .execute(
                json!({
                    "operation": "label_list",
                    "owner": "owner",
                    "repo": "repo"
                }),
                &test_ctx(),
            )
            .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("No labels found"),
                    "Expected empty message, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }
}
