use crate::github_api::client::{
    handle_response, parse_github_owner_repo, rate_limit_error, GitHubClient,
};
use crate::github_api::types::*;

impl GitHubClient {
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

        crate::github_api::client::fetch_all_pages(&self.client, &url, &self.auth_header(), 100)
            .await
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

    // ── PR Checkout ──

    pub async fn pr_checkout(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        checkout_dir: &std::path::Path,
    ) -> Result<String, String> {
        // GET /repos/{owner}/{repo}/pulls/{number} to get PR details
        let url = format!(
            "{}/repos/{}/{}/pulls/{}",
            self.base_url, owner, repo, number
        );

        let pr: PrInfo = handle_response(
            self.client
                .get(&url)
                .header("Authorization", self.auth_header())
                .send()
                .await
                .map_err(|e| format!("Network error: {e}"))?,
        )
        .await?;

        // Use git2 to fetch and checkout the PR head
        let git_repo = git2::Repository::open(checkout_dir)
            .map_err(|e| format!("Failed to open git repository: {e}"))?;

        // Add the remote if it doesn't exist, or verify existing origin matches
        let remote_name = "origin";
        let expected_url = format!("https://github.com/{owner}/{repo}.git");

        let needs_update = git_repo.find_remote(remote_name).ok().is_some_and(|r| {
            let expected_owner_repo = format!("{owner}/{repo}");
            r.url().is_some_and(|url| {
                matches!(parse_github_owner_repo(url), Some((o, r)) if format!("{o}/{r}") != expected_owner_repo)
            })
        });

        let mut remote = if git_repo.find_remote(remote_name).is_ok() {
            if needs_update {
                git_repo
                    .remote_set_url(remote_name, &expected_url)
                    .map_err(|e| format!("Failed to update remote URL: {e}"))?;
            }
            git_repo
                .find_remote(remote_name)
                .map_err(|e| format!("Failed to find remote: {e}"))?
        } else {
            git_repo
                .remote(remote_name, &expected_url)
                .map_err(|e| format!("Failed to create remote: {e}"))?
        };

        // Fetch the PR head ref
        let refspec = format!("+refs/pull/{number}/head:refs/remotes/origin/pr/{number}");
        remote
            .fetch(&[&refspec], None, None)
            .map_err(|e| format!("Failed to fetch PR #{number}: {e}"))?;

        // Create a local branch tracking the PR head
        let branch_name = format!("pr-{number}-{}", pr.head.git_ref);
        let pr_oid = git_repo
            .refname_to_id(&format!("refs/remotes/origin/pr/{number}"))
            .map_err(|e| format!("Failed to resolve PR reference: {e}"))?;

        let pr_commit = git_repo
            .find_commit(pr_oid)
            .map_err(|e| format!("Failed to find PR commit: {e}"))?;

        // Create or reset branch
        if git_repo
            .find_branch(&branch_name, git2::BranchType::Local)
            .is_ok()
        {
            // Branch already exists, reset it via its reference
            let branch = git_repo
                .find_branch(&branch_name, git2::BranchType::Local)
                .unwrap();
            branch
                .into_reference()
                .set_target(pr_commit.id(), "Reset to PR head")
                .map_err(|e| format!("Failed to reset branch: {e}"))?;
        } else {
            git_repo
                .branch(&branch_name, &pr_commit, false)
                .map_err(|e| format!("Failed to create branch: {e}"))?;
        }

        // Checkout the branch
        git_repo
            .set_head(&format!("refs/heads/{branch_name}"))
            .map_err(|e| format!("Failed to set HEAD: {e}"))?;

        git_repo
            .checkout_head(Some(
                git2::build::CheckoutBuilder::default()
                    .force()
                    .remove_untracked(false),
            ))
            .map_err(|e| format!("Failed to checkout: {e}"))?;

        Ok(format!(
            "Checked out PR #{number} ({}) locally",
            pr.head.git_ref
        ))
    }
}
