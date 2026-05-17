use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

// ── Response types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub login: String,
    pub id: u64,
    pub avatar_url: Option<String>,
    pub html_url: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub r#type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub body: Option<String>,
    pub head: PrRef,
    pub base: PrRef,
    pub draft: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
    pub user: PrUser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrRef {
    pub label: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrUser {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub html_url: String,
    pub body: Option<String>,
    pub labels: Vec<IssueLabel>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueLabel {
    pub name: String,
    pub color: Option<String>,
}

// ── GitHub Client ──

pub struct GitHubClient {
    client: reqwest::Client,
    base_url: String,
    token: String,
    token_validated: AtomicBool,
}

impl GitHubClient {
    pub fn new(token: &str, base_url: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent("hackpi-vcs/0.1")
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            token_validated: AtomicBool::new(false),
        })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    pub async fn validate_token(&self) -> Result<UserInfo, String> {
        let url = format!("{}/user", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        if resp.status().is_success() {
            let user: UserInfo = resp
                .json()
                .await
                .map_err(|e| format!("Failed to parse response: {e}"))?;
            self.token_validated.store(true, Ordering::Relaxed);
            Ok(user)
        } else if resp.status().as_u16() == 403 || resp.status().as_u16() == 429 {
            Err(rate_limit_error(resp).await)
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(format!("GitHub API error ({}): {}", status, body))
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn pr_create(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        head: &str,
        base: &str,
        body: Option<&str>,
        draft: Option<bool>,
    ) -> Result<PrInfo, String> {
        let url = format!("{}/repos/{}/{}/pulls", self.base_url, owner, repo);
        let mut params = serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
        });
        if let Some(b) = body {
            params["body"] = serde_json::Value::String(b.to_string());
        }
        if let Some(d) = draft {
            params["draft"] = serde_json::Value::Bool(d);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&params)
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        handle_response(resp).await
    }

    pub async fn pr_list(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
    ) -> Result<Vec<PrInfo>, String> {
        let mut url = format!("{}/repos/{}/{}/pulls", self.base_url, owner, repo);
        if let Some(s) = state {
            url = format!("{}?state={}", url, s);
        }

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        handle_response(resp).await
    }

    pub async fn pr_merge(&self, owner: &str, repo: &str, number: u64) -> Result<(), String> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{}/merge",
            self.base_url, owner, repo, number
        );

        let resp = self
            .client
            .put(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else if status.as_u16() == 403 || status.as_u16() == 429 {
            Err(rate_limit_error(resp).await)
        } else if status.as_u16() == 404 {
            Err(format!(
                "Repository '{owner}/{repo}' not found. Check visibility and permissions."
            ))
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("GitHub API error ({}): {}", status, body))
        }
    }

    pub async fn issue_create(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: Option<&str>,
        labels: Option<Vec<&str>>,
    ) -> Result<IssueInfo, String> {
        let url = format!("{}/repos/{}/{}/issues", self.base_url, owner, repo);
        let mut params = serde_json::json!({
            "title": title,
        });
        if let Some(b) = body {
            params["body"] = serde_json::Value::String(b.to_string());
        }
        if let Some(l) = labels {
            let label_values: Vec<serde_json::Value> = l
                .iter()
                .map(|s| serde_json::Value::String(s.to_string()))
                .collect();
            params["labels"] = serde_json::Value::Array(label_values);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&params)
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        handle_response(resp).await
    }

    pub async fn issue_list(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
    ) -> Result<Vec<IssueInfo>, String> {
        let mut url = format!("{}/repos/{}/{}/issues", self.base_url, owner, repo);
        if let Some(s) = state {
            url = format!("{}?state={}", url, s);
        }

        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        handle_response(resp).await
    }

    pub async fn issue_close(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<IssueInfo, String> {
        let url = format!(
            "{}/repos/{}/{}/issues/{}",
            self.base_url, owner, repo, number
        );

        let params = serde_json::json!({
            "state": "closed",
        });

        let resp = self
            .client
            .patch(&url)
            .header("Authorization", self.auth_header())
            .json(&params)
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        handle_response(resp).await
    }

    pub async fn issue_comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), String> {
        let url = format!(
            "{}/repos/{}/{}/issues/{}/comments",
            self.base_url, owner, repo, number
        );

        let params = serde_json::json!({
            "body": body,
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&params)
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else if status.as_u16() == 403 || status.as_u16() == 429 {
            Err(rate_limit_error(resp).await)
        } else if status.as_u16() == 404 {
            Err(format!(
                "Repository '{owner}/{repo}' not found. Check visibility and permissions."
            ))
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(format!("GitHub API error ({}): {}", status, body))
        }
    }
}

// ── Error helpers ──

async fn handle_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, String> {
    let status = resp.status();
    if status.is_success() {
        resp.json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))
    } else if status.as_u16() == 403 || status.as_u16() == 429 {
        Err(rate_limit_error(resp).await)
    } else if status.as_u16() == 404 {
        Err("Repository not found. Check visibility and permissions.".to_string())
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(format!("GitHub API error ({}): {}", status, body))
    }
}

async fn rate_limit_error(resp: reqwest::Response) -> String {
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    let body = resp.text().await.unwrap_or_default();
    format!("Rate limited (retry after {retry_after}s): {body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper: mount a mock that responds to GET /user with a valid UserInfo.
    async fn setup_valid_token_mock(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "login": "testuser",
                "id": 12345,
                "avatar_url": "https://avatars.githubusercontent.com/u/12345",
                "html_url": "https://github.com/testuser",
                "name": "Test User",
                "email": "test@example.com",
                "type": "User"
            })))
            .mount(server)
            .await;
    }

    /// Helper: create a GitHubClient pointing at the mock server.
    fn make_client(server: &MockServer) -> GitHubClient {
        GitHubClient::new("test-token", &server.uri()).unwrap()
    }

    // ── Token validation ──

    #[tokio::test]
    async fn test_validate_token_success() {
        let server = MockServer::start().await;
        setup_valid_token_mock(&server).await;
        let client = make_client(&server);

        let result = client.validate_token().await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let user = result.unwrap();
        assert_eq!(user.login, "testuser");
        assert_eq!(user.id, 12345);
    }

    #[tokio::test]
    async fn test_validate_token_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Bad credentials"))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.validate_token().await;

        assert!(result.is_err(), "Expected Err, got: {result:?}");
        let err = result.unwrap_err();
        assert!(err.contains("401"), "Expected 401 error, got: {err}");
    }

    // ── PR create ──

    #[tokio::test]
    async fn test_pr_create_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 42,
                "title": "Test PR",
                "state": "open",
                "html_url": "https://github.com/owner/repo/pull/42",
                "body": "Description",
                "head": { "label": "owner:feature", "ref": "feature", "sha": "abc123" },
                "base": { "label": "owner:main", "ref": "main", "sha": "def456" },
                "draft": false,
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "user": { "login": "testuser" }
            })))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .pr_create(
                "owner",
                "repo",
                "Test PR",
                "feature",
                "main",
                Some("Description"),
                Some(false),
            )
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let pr = result.unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Test PR");
        assert_eq!(pr.state, "open");
        assert_eq!(pr.head.git_ref, "feature");
        assert_eq!(pr.base.git_ref, "main");
        assert_eq!(pr.user.login, "testuser");
    }

    #[tokio::test]
    async fn test_pr_create_without_optional_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 1,
                "title": "Minimal PR",
                "state": "open",
                "html_url": "https://github.com/owner/repo/pull/1",
                "body": null,
                "head": { "label": "owner:feature", "ref": "feature", "sha": "abc" },
                "base": { "label": "owner:main", "ref": "main", "sha": "def" },
                "draft": null,
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "user": { "login": "testuser" }
            })))
            .mount(&server)
            .await;
        let client = make_client(&server);

        // No body, no draft
        let result = client
            .pr_create("owner", "repo", "Minimal PR", "feature", "main", None, None)
            .await;

        assert!(result.is_ok());
    }

    // ── PR list ──

    #[tokio::test]
    async fn test_pr_list_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 1,
                    "title": "PR One",
                    "state": "open",
                    "html_url": "https://github.com/owner/repo/pull/1",
                    "body": null,
                    "head": { "label": "owner:feature", "ref": "feature", "sha": "a" },
                    "base": { "label": "owner:main", "ref": "main", "sha": "b" },
                    "draft": false,
                    "created_at": "2024-01-01T00:00:00Z",
                    "updated_at": "2024-01-01T00:00:00Z",
                    "user": { "login": "user1" }
                },
                {
                    "number": 2,
                    "title": "PR Two",
                    "state": "closed",
                    "html_url": "https://github.com/owner/repo/pull/2",
                    "body": "Fixed bug",
                    "head": { "label": "owner:fix", "ref": "fix", "sha": "c" },
                    "base": { "label": "owner:main", "ref": "main", "sha": "d" },
                    "draft": false,
                    "created_at": "2024-01-02T00:00:00Z",
                    "updated_at": "2024-01-02T00:00:00Z",
                    "user": { "login": "user2" }
                }
            ])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.pr_list("owner", "repo", None).await;

        assert!(result.is_ok());
        let prs = result.unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[0].number, 1);
        assert_eq!(prs[1].number, 2);
    }

    #[tokio::test]
    async fn test_pr_list_with_state_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .and(query_param("state", "closed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 2,
                    "title": "PR Two",
                    "state": "closed",
                    "html_url": "https://github.com/owner/repo/pull/2",
                    "body": null,
                    "head": { "label": "owner:fix", "ref": "fix", "sha": "c" },
                    "base": { "label": "owner:main", "ref": "main", "sha": "d" },
                    "draft": false,
                    "created_at": "2024-01-02T00:00:00Z",
                    "updated_at": "2024-01-02T00:00:00Z",
                    "user": { "login": "user2" }
                }
            ])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.pr_list("owner", "repo", Some("closed")).await;

        assert!(result.is_ok());
        let prs = result.unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].state, "closed");
    }

    // ── PR merge ──

    #[tokio::test]
    async fn test_pr_merge_success() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/owner/repo/pulls/42/merge"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.pr_merge("owner", "repo", 42).await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn test_pr_merge_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(r"/repos/[^/]+/[^/]+/pulls/\d+/merge"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.pr_merge("owner", "repo", 999).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not found"),
            "Expected 'not found' error, got: {err}"
        );
    }

    // ── Issue create ──

    #[tokio::test]
    async fn test_issue_create_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 100,
                "title": "Test Issue",
                "state": "open",
                "html_url": "https://github.com/owner/repo/issues/100",
                "body": "Issue description",
                "labels": [
                    { "name": "bug", "color": "d73a4a" },
                    { "name": "enhancement", "color": "a2eeef" }
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .issue_create(
                "owner",
                "repo",
                "Test Issue",
                Some("Issue description"),
                Some(vec!["bug", "enhancement"]),
            )
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let issue = result.unwrap();
        assert_eq!(issue.number, 100);
        assert_eq!(issue.title, "Test Issue");
        assert_eq!(issue.state, "open");
        assert_eq!(issue.labels.len(), 2);
        assert_eq!(issue.labels[0].name, "bug");
    }

    #[tokio::test]
    async fn test_issue_create_minimal() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 101,
                "title": "Minimal Issue",
                "state": "open",
                "html_url": "https://github.com/owner/repo/issues/101",
                "body": null,
                "labels": [],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .issue_create("owner", "repo", "Minimal Issue", None, None)
            .await;

        assert!(result.is_ok());
    }

    // ── Issue list ──

    #[tokio::test]
    async fn test_issue_list_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/issues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 1,
                    "title": "Issue One",
                    "state": "open",
                    "html_url": "https://github.com/owner/repo/issues/1",
                    "body": "First issue",
                    "labels": [{"name": "bug", "color": "red"}],
                    "created_at": "2024-01-01T00:00:00Z",
                    "updated_at": "2024-01-01T00:00:00Z"
                },
                {
                    "number": 2,
                    "title": "Issue Two",
                    "state": "closed",
                    "html_url": "https://github.com/owner/repo/issues/2",
                    "body": null,
                    "labels": [],
                    "created_at": "2024-01-02T00:00:00Z",
                    "updated_at": "2024-01-02T00:00:00Z"
                }
            ])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.issue_list("owner", "repo", None).await;

        assert!(result.is_ok());
        let issues = result.unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].number, 1);
        assert_eq!(issues[1].number, 2);
    }

    #[tokio::test]
    async fn test_issue_list_with_state_filter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/issues"))
            .and(query_param("state", "open"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 1,
                    "title": "Open Issue",
                    "state": "open",
                    "html_url": "https://github.com/owner/repo/issues/1",
                    "body": null,
                    "labels": [],
                    "created_at": "2024-01-01T00:00:00Z",
                    "updated_at": "2024-01-01T00:00:00Z"
                }
            ])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.issue_list("owner", "repo", Some("open")).await;

        assert!(result.is_ok());
        let issues = result.unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].state, "open");
    }

    // ── Issue close ──

    #[tokio::test]
    async fn test_issue_close_success() {
        let server = MockServer::start().await;
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
        let client = make_client(&server);

        let result = client.issue_close("owner", "repo", 5).await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let issue = result.unwrap();
        assert_eq!(issue.state, "closed");
    }

    // ── Issue comment ──

    #[tokio::test]
    async fn test_issue_comment_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/5/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_string("{}"))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .issue_comment("owner", "repo", 5, "Looks good!")
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
    }

    // ── Error handling ──

    #[tokio::test]
    async fn test_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "60")
                    .set_body_string("API rate limit exceeded"),
            )
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.validate_token().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("rate limited") || err.contains("Rate limited"),
            "Expected rate limit error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/unknown/missing/pulls"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.pr_list("unknown", "missing", None).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("not found"),
            "Expected 'not found' error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_network_error() {
        // Point client at a non-existent server
        let client = GitHubClient::new("test-token", "http://127.0.0.1:1").unwrap();

        let result = client.validate_token().await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Network error"),
            "Expected Network error, got: {err}"
        );
    }

    // ── Client creation ──

    #[test]
    fn test_new_client_with_base_url_trailing_slash() {
        let client = GitHubClient::new("token", "https://api.github.com/").unwrap();
        assert_eq!(client.base_url, "https://api.github.com");
    }

    #[test]
    fn test_new_client_success() {
        let client = GitHubClient::new("token", "https://api.github.com").unwrap();
        assert_eq!(client.base_url, "https://api.github.com");
    }
}
