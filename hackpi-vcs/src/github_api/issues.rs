use crate::github_api::client::{fetch_all_pages, handle_response, rate_limit_error, GitHubClient};
use crate::github_api::types::*;

impl GitHubClient {
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

        fetch_all_pages(&self.client, &url, &self.auth_header(), 100).await
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

    // ── Labels ──

    pub async fn label_add(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: Vec<String>,
    ) -> Result<Vec<LabelInfo>, String> {
        let url = format!(
            "{}/repos/{}/{}/issues/{}/labels",
            self.base_url, owner, repo, issue_number
        );

        let params = serde_json::json!({
            "labels": labels,
        });

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

    pub async fn label_list(&self, owner: &str, repo: &str) -> Result<Vec<LabelInfo>, String> {
        let url = format!("{}/repos/{}/{}/labels", self.base_url, owner, repo);
        fetch_all_pages(&self.client, &url, &self.auth_header(), 100).await
    }
}
