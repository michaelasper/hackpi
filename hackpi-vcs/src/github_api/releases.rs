use crate::github_api::client::{fetch_all_pages, handle_response, GitHubClient};
use crate::github_api::types::*;

impl GitHubClient {
    #[allow(clippy::too_many_arguments)]
    pub async fn release_create(
        &self,
        owner: &str,
        repo: &str,
        tag_name: &str,
        name: &str,
        body: Option<&str>,
        draft: bool,
        prerelease: bool,
    ) -> Result<ReleaseInfo, String> {
        let url = format!("{}/repos/{}/{}/releases", self.base_url, owner, repo);

        let mut params = serde_json::json!({
            "tag_name": tag_name,
            "name": name,
            "draft": draft,
            "prerelease": prerelease,
        });
        if let Some(b) = body {
            params["body"] = serde_json::Value::String(b.to_string());
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

    pub async fn release_list(&self, owner: &str, repo: &str) -> Result<Vec<ReleaseInfo>, String> {
        let url = format!("{}/repos/{}/{}/releases", self.base_url, owner, repo);
        fetch_all_pages(&self.client, &url, &self.auth_header(), 100).await
    }
}
