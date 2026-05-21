use crate::github_api::types::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ── GitHub Client ──

pub struct GitHubClient {
    pub(crate) client: reqwest::Client,
    pub(crate) base_url: String,
    token: String,
    token_validated: AtomicBool,
}

impl GitHubClient {
    pub fn new(token: &str, base_url: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent("hackpi-vcs/0.1")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            token_validated: AtomicBool::new(false),
        })
    }

    pub fn auth_header(&self) -> String {
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
}

// ── Error helpers ──

pub(crate) async fn handle_response<T: serde::de::DeserializeOwned>(
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

pub(crate) async fn rate_limit_error(resp: reqwest::Response) -> String {
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    let body = resp.text().await.unwrap_or_default();
    format!("Rate limited (retry after {retry_after}s): {body}")
}

// ── Remote URL helpers ──

/// Parse `(owner, repo)` from a GitHub remote URL.
///
/// Supports the following URL formats:
/// - `https://github.com/owner/repo.git`
/// - `https://github.com/owner/repo`
/// - `git@github.com:owner/repo.git`
/// - `git@github.com:owner/repo`
/// - `ssh://git@github.com/owner/repo.git`
///
/// Returns `None` for non-GitHub URLs (e.g. local file paths).
pub(crate) fn parse_github_owner_repo(url: &str) -> Option<(String, String)> {
    let url = url.strip_suffix(".git").unwrap_or(url);

    if let Some(path) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
    {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_string(), parts[1..].join("/")));
        }
    }

    if let Some(path) = url.strip_prefix("git@github.com:") {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_string(), parts[1..].join("/")));
        }
    }

    None
}

// ── Pagination helpers ──

/// Fetch all pages from a paginated GitHub API endpoint, up to max_results.
pub(crate) async fn fetch_all_pages<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    auth_header: &str,
    max_results: usize,
) -> Result<Vec<T>, String> {
    let mut all_results = Vec::new();
    let mut next_url = Some(url.to_string());

    while let Some(url) = next_url.take() {
        let resp = client
            .get(&url)
            .header("Authorization", auth_header)
            .send()
            .await
            .map_err(|e| format!("Network error: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            if status.as_u16() == 403 || status.as_u16() == 429 {
                return Err(rate_limit_error(resp).await);
            } else if status.as_u16() == 404 {
                return Err("Repository not found. Check visibility and permissions.".to_string());
            } else {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("GitHub API error ({}): {}", status, body));
            }
        }

        // Parse Link header for pagination
        let link_header = resp
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Parse response body
        let page: Vec<T> = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        let remaining = max_results.saturating_sub(all_results.len());
        let take = page.len().min(remaining);
        all_results.extend(page.into_iter().take(take));

        if all_results.len() >= max_results {
            break;
        }

        // Check for next page
        if let Some(link) = link_header {
            next_url = parse_next_link(&link);
        }
    }

    Ok(all_results)
}

pub(crate) fn parse_next_link(link_header: &str) -> Option<String> {
    // Link header format: <https://api.github.com/...?page=2>; rel="next", <...>; rel="last"
    for part in link_header.split(',') {
        if part.contains("rel=\"next\"") {
            if let Some(start) = part.find('<') {
                if let Some(end) = part.find('>') {
                    return Some(part[start + 1..end].to_string());
                }
            }
        }
    }
    None
}
