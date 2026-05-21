pub mod client;
pub mod issues;
pub mod prs;
pub mod releases;
pub mod types;

pub use client::GitHubClient;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_api::client::{fetch_all_pages, parse_github_owner_repo, parse_next_link};
    use std::time::Duration;
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

    // ── Pagination helpers ──

    #[test]
    fn test_parse_next_link_found() {
        let link = r#"<https://api.github.com/repos/owner/repo/labels?page=2>; rel="next", <https://api.github.com/repos/owner/repo/labels?page=3>; rel="last""#;
        let result = parse_next_link(link);
        assert_eq!(
            result,
            Some("https://api.github.com/repos/owner/repo/labels?page=2".to_string())
        );
    }

    #[test]
    fn test_parse_next_link_not_found() {
        let link = r#"<https://api.github.com/repos/owner/repo/labels?page=1>; rel="last""#;
        let result = parse_next_link(link);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_next_link_empty() {
        let result = parse_next_link("");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_next_link_no_rel() {
        let link = r#"<https://api.github.com/repos/owner/repo/labels?page=2>"#;
        let result = parse_next_link(link);
        assert_eq!(result, None);
    }

    // ── label_add ──

    #[tokio::test]
    async fn test_label_add_single() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/42/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "bug", "color": "d73a4a", "description": "Bug report"}
            ])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .label_add("owner", "repo", 42, vec!["bug".into()])
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let labels = result.unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].name, "bug");
        assert_eq!(labels[0].color, Some("d73a4a".to_string()));
    }

    #[tokio::test]
    async fn test_label_add_multiple() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues/42/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "bug", "color": "d73a4a", "description": "Bug report"},
                {"name": "feature", "color": "0e8a16", "description": "Feature request"}
            ])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .label_add("owner", "repo", 42, vec!["bug".into(), "feature".into()])
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let labels = result.unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "bug");
        assert_eq!(labels[1].name, "feature");
    }

    // ── label_list ──

    #[tokio::test]
    async fn test_label_list_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "bug", "color": "d73a4a", "description": "Bug report"},
                {"name": "feature", "color": "0e8a16", "description": "Feature request"}
            ])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.label_list("owner", "repo").await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let labels = result.unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "bug");
        assert_eq!(labels[1].name, "feature");
    }

    #[tokio::test]
    async fn test_label_list_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.label_list("owner", "repo").await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let labels = result.unwrap();
        assert!(labels.is_empty());
    }

    // ── release_create ──

    #[tokio::test]
    async fn test_release_create_success() {
        let server = MockServer::start().await;
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
        let client = make_client(&server);

        let result = client
            .release_create(
                "owner",
                "repo",
                "v1.0.0",
                "Release v1.0.0",
                Some("First release"),
                false,
                false,
            )
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let release = result.unwrap();
        assert_eq!(release.tag_name, "v1.0.0");
        assert_eq!(release.name, "Release v1.0.0");
        assert!(!release.draft);
        assert!(!release.prerelease);
    }

    #[tokio::test]
    async fn test_release_create_draft() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "tag_name": "v2.0.0-beta",
                "name": "Beta",
                "body": null,
                "draft": true,
                "prerelease": false,
                "html_url": "https://github.com/owner/repo/releases/tag/v2.0.0-beta",
                "created_at": "2024-01-01T00:00:00Z",
                "published_at": null
            })))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .release_create("owner", "repo", "v2.0.0-beta", "Beta", None, true, false)
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let release = result.unwrap();
        assert_eq!(release.tag_name, "v2.0.0-beta");
        assert!(release.draft);
        assert!(!release.prerelease);
        assert!(release.body.is_none());
    }

    #[tokio::test]
    async fn test_release_create_prerelease() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "tag_name": "v1.1.0-rc1",
                "name": "Release Candidate 1",
                "body": "RC",
                "draft": false,
                "prerelease": true,
                "html_url": "https://github.com/owner/repo/releases/tag/v1.1.0-rc1",
                "created_at": "2024-01-01T00:00:00Z",
                "published_at": null
            })))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client
            .release_create(
                "owner",
                "repo",
                "v1.1.0-rc1",
                "Release Candidate 1",
                Some("RC"),
                false,
                true,
            )
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let release = result.unwrap();
        assert_eq!(release.tag_name, "v1.1.0-rc1");
        assert!(!release.draft);
        assert!(release.prerelease);
    }

    // ── release_list ──

    #[tokio::test]
    async fn test_release_list_success() {
        let server = MockServer::start().await;
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
        let client = make_client(&server);

        let result = client.release_list("owner", "repo").await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let releases = result.unwrap();
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].tag_name, "v2.0.0");
        assert_eq!(releases[1].tag_name, "v1.0.0");
    }

    #[tokio::test]
    async fn test_release_list_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        let client = make_client(&server);

        let result = client.release_list("owner", "repo").await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let releases = result.unwrap();
        assert!(releases.is_empty());
    }

    // ── parse_github_owner_repo ──

    #[test]
    fn test_parse_github_owner_repo_https() {
        let result = parse_github_owner_repo("https://github.com/owner/repo.git");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_owner_repo_https_no_dotgit() {
        let result = parse_github_owner_repo("https://github.com/owner/repo");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_owner_repo_ssh() {
        let result = parse_github_owner_repo("git@github.com:owner/repo.git");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_owner_repo_ssh_no_dotgit() {
        let result = parse_github_owner_repo("git@github.com:owner/repo");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_owner_repo_ssh_url_scheme() {
        let result = parse_github_owner_repo("ssh://git@github.com/owner/repo.git");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_owner_repo_nested_path() {
        let result = parse_github_owner_repo("https://github.com/org/team/subproject.git");
        assert_eq!(
            result,
            Some(("org".to_string(), "team/subproject".to_string()))
        );
    }

    #[test]
    fn test_parse_github_owner_repo_non_github() {
        let result = parse_github_owner_repo("https://gitlab.com/owner/repo.git");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_github_owner_repo_local_path() {
        let result = parse_github_owner_repo("/tmp/some-repo");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_github_owner_repo_file_url() {
        let result = parse_github_owner_repo("file:///tmp/some-repo");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_github_owner_repo_empty() {
        let result = parse_github_owner_repo("");
        assert_eq!(result, None);
    }

    // ── pr_checkout ──

    #[tokio::test]
    async fn test_pr_checkout_success() {
        let server = MockServer::start().await;
        // Mock the GET /pulls/{number} endpoint
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
        let client = make_client(&server);

        // Create a bare repo to act as the "remote" origin
        let remote_dir = tempfile::tempdir().unwrap();
        let bare_repo = git2::Repository::init_bare(remote_dir.path()).unwrap();

        // Create an initial commit in the bare repo
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

        // Create a PR ref in the remote: refs/pull/42/head
        bare_repo
            .reference("refs/pull/42/head", initial_oid, true, "Create PR ref")
            .unwrap();

        // Create a local repo with the bare repo as origin
        let tmp = tempfile::tempdir().unwrap();
        let local_repo = git2::Repository::init(tmp.path()).unwrap();

        // Create initial commit in local repo
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
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

        // Set remote origin to point to the bare repo
        local_repo
            .remote("origin", remote_dir.path().to_str().unwrap())
            .unwrap();

        let result = client.pr_checkout("owner", "repo", 42, tmp.path()).await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let msg = result.unwrap();
        assert!(
            msg.contains("42"),
            "Expected PR number in output, got: {msg}"
        );
        assert!(
            msg.contains("feature-x"),
            "Expected branch name in output, got: {msg}"
        );

        // Verify the branch was created
        let branch = local_repo.find_branch("pr-42-feature-x", git2::BranchType::Local);
        assert!(branch.is_ok(), "Expected branch pr-42-feature-x to exist");
    }

    #[tokio::test]
    async fn test_pr_checkout_updates_wrong_origin() {
        let server = MockServer::start().await;
        // Mock the GET /pulls/{number} endpoint
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
        let client = make_client(&server);

        // Create a bare repo that has the PR ref (the "correct" remote)
        let correct_remote_dir = tempfile::tempdir().unwrap();
        let correct_bare = git2::Repository::init_bare(correct_remote_dir.path()).unwrap();

        let sig = git2::Signature::now("test", "test@test.com").unwrap();
        let tree_id = {
            let mut index = correct_bare.index().unwrap();
            let oid = index.write_tree().unwrap();
            correct_bare.find_tree(oid).unwrap().id()
        };
        let initial_oid = correct_bare
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                "initial",
                &correct_bare.find_tree(tree_id).unwrap(),
                &[],
            )
            .unwrap();

        // Create the PR ref in the correct repo
        correct_bare
            .reference("refs/pull/42/head", initial_oid, true, "Create PR ref")
            .unwrap();

        // Create a local repo with origin pointing to a *different* GitHub repo
        let tmp = tempfile::tempdir().unwrap();
        let local_repo = git2::Repository::init(tmp.path()).unwrap();

        let sig = git2::Signature::now("test", "test@test.com").unwrap();
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

        // Set origin to point to a WRONG GitHub repo URL
        local_repo
            .remote("origin", "https://github.com/wrong/other-repo.git")
            .unwrap();

        // pr_checkout should detect the mismatch and update origin to owner/repo
        let result = client.pr_checkout("owner", "repo", 42, tmp.path()).await;

        // The fetch will fail because the updated URL points to github.com, not to
        // our local bare repo. But this test proves the code detects the mismatch
        // and attempts to use the correct URL — the error should reference the
        // updated URL, not the wrong one.
        assert!(result.is_err(), "Expected fetch to fail after URL update");
        let err = result.unwrap_err();
        assert!(
            err.contains("owner/repo") || err.contains("Failed to fetch"),
            "Expected error to reference the correct repo, got: {err}"
        );

        // Prove origin URL was updated from wrong/other-repo to owner/repo
        let origin = local_repo.find_remote("origin").unwrap();
        let origin_url = origin.url().unwrap();
        assert!(
            origin_url.contains("owner/repo"),
            "Expected origin URL to be updated to owner/repo, got: {origin_url}"
        );
        assert!(
            !origin_url.contains("wrong"),
            "Expected origin URL to no longer point to wrong/other-repo, got: {origin_url}"
        );
    }

    #[tokio::test]
    async fn test_pr_checkout_preserves_matching_non_github_origin() {
        // When origin points to a non-GitHub URL (e.g. local file path) that
        // serves the correct repo data, pr_checkout should NOT update it.
        let server = MockServer::start().await;
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
        let client = make_client(&server);

        // Create a bare repo to act as the remote
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
            .reference("refs/pull/42/head", initial_oid, true, "Create PR ref")
            .unwrap();

        // Local repo with origin pointing to a local path (non-GitHub URL)
        let tmp = tempfile::tempdir().unwrap();
        let local_repo = git2::Repository::init(tmp.path()).unwrap();
        let sig = git2::Signature::now("test", "test@test.com").unwrap();
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

        // This should succeed — non-GitHub URLs are not treated as mismatches
        let result = client.pr_checkout("owner", "repo", 42, tmp.path()).await;

        assert!(
            result.is_ok(),
            "Expected Ok for non-GitHub origin, got: {result:?}"
        );

        // Verify the branch was created
        let branch = local_repo.find_branch("pr-42-feature-x", git2::BranchType::Local);
        assert!(branch.is_ok(), "Expected branch pr-42-feature-x to exist");
    }

    // ── Pagination integration ──

    #[tokio::test]
    async fn test_fetch_all_pages_caps_at_max() {
        let server = MockServer::start().await;
        let page1_link = format!(
            r#"<{}/repos/owner/repo/labels?page=2>; rel="next", <{}/repos/owner/repo/labels?page=2>; rel="last""#,
            server.uri(),
            server.uri()
        );
        let page2_link = format!(
            r#"<{}/repos/owner/repo/labels?page=3>; rel="next", <{}/repos/owner/repo/labels?page=3>; rel="last""#,
            server.uri(),
            server.uri()
        );

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", page1_link.as_str())
                    .set_body_json(serde_json::json!([
                        {"name": "bug", "color": "red"},
                        {"name": "feature", "color": "green"},
                    ])),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", page2_link.as_str())
                    .set_body_json(serde_json::json!([
                        {"name": "enhancement", "color": "blue"},
                        {"name": "documentation", "color": "yellow"},
                    ])),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .and(query_param("page", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "urgent", "color": "orange"},
            ])))
            .mount(&server)
            .await;

        let client = make_client(&server);
        let url = format!("{}/repos/owner/repo/labels", server.uri());

        let result = fetch_all_pages::<LabelInfo>(
            &client.client,
            &url,
            &client.auth_header(),
            3, // cap at 3
        )
        .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let labels = result.unwrap();
        assert_eq!(labels.len(), 3, "Should be capped at 3");
    }

    #[tokio::test]
    async fn test_fetch_all_pages_no_pagination() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/labels"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"name": "bug", "color": "red"},
                {"name": "feature", "color": "green"},
            ])))
            .mount(&server)
            .await;

        let client = make_client(&server);
        let url = format!("{}/repos/owner/repo/labels", server.uri());

        let result =
            fetch_all_pages::<LabelInfo>(&client.client, &url, &client.auth_header(), 100).await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let labels = result.unwrap();
        assert_eq!(labels.len(), 2);
    }

    // ── Draft PR creation ──

    #[tokio::test]
    async fn test_pr_create_draft() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 55,
                "title": "Draft PR",
                "state": "open",
                "html_url": "https://github.com/owner/repo/pull/55",
                "body": "Draft description",
                "head": { "label": "owner:draft-feature", "ref": "draft-feature", "sha": "abc123" },
                "base": { "label": "owner:main", "ref": "main", "sha": "def456" },
                "draft": true,
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
                "Draft PR",
                "draft-feature",
                "main",
                Some("Draft description"),
                Some(true), // draft = true
            )
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let pr = result.unwrap();
        assert_eq!(pr.number, 55);
        assert_eq!(pr.title, "Draft PR");
    }

    // ── Issue list with pagination ──

    #[tokio::test]
    async fn test_issue_list_with_pagination() {
        let server = MockServer::start().await;

        // Page 2 (mount first so it has lower priority — wiremock uses last-mounted-first)
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/issues"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 2,
                    "title": "Issue Two",
                    "state": "open",
                    "html_url": "https://github.com/owner/repo/issues/2",
                    "body": null,
                    "labels": [],
                    "created_at": "2024-01-02T00:00:00Z",
                    "updated_at": "2024-01-02T00:00:00Z"
                }
            ])))
            .mount(&server)
            .await;

        // Page 1 with Link header pointing to page 2 (mounted second = higher priority)
        let page1_link = format!(
            r#"<{}/repos/owner/repo/issues?page=2>; rel="next""#,
            server.uri()
        );
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/issues"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", page1_link.as_str())
                    .set_body_json(serde_json::json!([
                        {
                            "number": 1,
                            "title": "Issue One",
                            "state": "open",
                            "html_url": "https://github.com/owner/repo/issues/1",
                            "body": null,
                            "labels": [],
                            "created_at": "2024-01-01T00:00:00Z",
                            "updated_at": "2024-01-01T00:00:00Z"
                        }
                    ])),
            )
            .mount(&server)
            .await;

        let client = make_client(&server);

        let result = client.issue_list("owner", "repo", None).await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let issues = result.unwrap();
        assert_eq!(issues.len(), 2, "Expected 2 issues across pages");
        assert_eq!(issues[0].number, 1);
        assert_eq!(issues[1].number, 2);
    }

    // ── PR list with pagination ──

    #[tokio::test]
    async fn test_pr_list_with_pagination() {
        let server = MockServer::start().await;

        // Page 2
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "number": 2,
                    "title": "PR Two",
                    "state": "closed",
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

        // Page 1
        let page1_link = format!(
            r#"<{}/repos/owner/repo/pulls?page=2>; rel="next""#,
            server.uri()
        );
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/pulls"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", page1_link.as_str())
                    .set_body_json(serde_json::json!([
                        {
                            "number": 1,
                            "title": "PR One",
                            "state": "open",
                            "html_url": "https://github.com/owner/repo/pull/1",
                            "body": null,
                            "head": { "label": "owner:f1", "ref": "f1", "sha": "a" },
                            "base": { "label": "owner:main", "ref": "main", "sha": "b" },
                            "draft": false,
                            "created_at": "2024-01-01T00:00:00Z",
                            "updated_at": "2024-01-01T00:00:00Z",
                            "user": { "login": "user1" }
                        }
                    ])),
            )
            .mount(&server)
            .await;

        let client = make_client(&server);
        let result = client.pr_list("owner", "repo", None).await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let prs = result.unwrap();
        assert_eq!(prs.len(), 2, "Expected 2 PRs across pages");
    }

    // ── Network timeout handling ──

    #[tokio::test]
    async fn test_timeout_handling() {
        // Use a non-routable address — the client's built-in connect_timeout (10s)
        // will fire rather than hanging indefinitely.
        let client = GitHubClient::new("test-token", "http://192.0.2.1:1").unwrap();

        // Wrap in a generous safety net so the test suite never hangs even if
        // the client-level timeout is somehow misconfigured.
        let result = tokio::time::timeout(Duration::from_secs(15), client.validate_token()).await;

        match result {
            Ok(inner) => {
                // The client's built-in timeout should produce an error.
                assert!(
                    inner.is_err(),
                    "Expected client timeout error, got: {inner:?}"
                );
            }
            Err(_) => {
                // Outer safety-net timeout — client-level timeout likely misconfigured.
                panic!(
                    "Client-level timeout did not fire within 15s — check connect_timeout setting"
                );
            }
        }
    }

    // ── Issue create with labels verification ──

    #[tokio::test]
    async fn test_issue_create_with_labels_in_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/owner/repo/issues"))
            // Verify the request body contains the labels array
            .and(wiremock::matchers::body_json(serde_json::json!({
                "title": "Labeled Issue",
                "body": "desc",
                "labels": ["bug", "priority:high"]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "number": 200,
                "title": "Labeled Issue",
                "state": "open",
                "html_url": "https://github.com/owner/repo/issues/200",
                "body": "desc",
                "labels": [
                    { "name": "bug", "color": "d73a4a" },
                    { "name": "priority:high", "color": "b60205" }
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
                "Labeled Issue",
                Some("desc"),
                Some(vec!["bug", "priority:high"]),
            )
            .await;

        assert!(result.is_ok(), "Expected Ok, got: {result:?}");
        let issue = result.unwrap();
        assert_eq!(issue.number, 200);
        assert_eq!(issue.labels.len(), 2);
        assert_eq!(issue.labels[0].name, "bug");
        assert_eq!(issue.labels[1].name, "priority:high");
    }
}
