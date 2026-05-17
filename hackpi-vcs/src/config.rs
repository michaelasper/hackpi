use std::path::Path;

#[derive(Debug, Clone)]
pub struct VcsConfig {
    pub github_token: Option<String>,
    pub github_token_source: TokenSource,
    pub github_base_url: String,
    pub default_remote: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenSource {
    None,
    EnvVar(String), // variable name
    ConfigFile,     // .hackpi/config.toml (future)
}

impl VcsConfig {
    /// Load config from environment and detect git remote defaults.
    pub fn from_env(_workspace_root: &Path) -> Self {
        // Token resolution: HACKPI_GITHUB_TOKEN → GITHUB_TOKEN → None
        let (github_token, github_token_source) =
            if let Ok(token) = std::env::var("HACKPI_GITHUB_TOKEN") {
                (
                    Some(token),
                    TokenSource::EnvVar("HACKPI_GITHUB_TOKEN".into()),
                )
            } else if let Ok(token) = std::env::var("GITHUB_TOKEN") {
                (Some(token), TokenSource::EnvVar("GITHUB_TOKEN".into()))
            } else {
                (None, TokenSource::None)
            };

        Self {
            github_token,
            github_token_source,
            github_base_url: "https://api.github.com".into(),
            default_remote: "origin".into(),
            default_branch: "main".into(),
        }
    }

    /// Parse a GitHub URL into (owner, repo).
    /// Supports git@github.com:owner/repo.git and https://github.com/owner/repo.git
    pub fn parse_github_url(url: &str) -> Option<(String, String)> {
        let url = url.trim();

        // SSH format: git@github.com:owner/repo.git
        if let Some(path) = url.strip_prefix("git@github.com:") {
            let path = path.strip_suffix(".git").unwrap_or(path);
            let parts: Vec<&str> = path.split('/').collect();
            if parts.len() == 2 {
                return Some((parts[0].to_string(), parts[1].to_string()));
            }
        }

        // HTTPS format: https://github.com/owner/repo.git
        if let Some(path) = url.strip_prefix("https://github.com/") {
            let path = path.strip_suffix(".git").unwrap_or(path);
            let parts: Vec<&str> = path.split('/').collect();
            if parts.len() >= 2 {
                return Some((
                    parts[parts.len() - 2].to_string(),
                    parts[parts.len() - 1].to_string(),
                ));
            }
        }

        None
    }

    /// Infer owner/repo from the current git remote.
    pub fn infer_owner_repo(workspace_root: &Path) -> Option<(String, String)> {
        let repo = git2::Repository::discover(workspace_root).ok()?;
        let remote = repo.find_remote("origin").ok()?;
        let url = remote.url()?;
        Self::parse_github_url(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-var-dependent tests to prevent race conditions.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env_clean<F>(f: F)
    where
        F: FnOnce(),
    {
        let _lock = ENV_LOCK.lock().unwrap();
        let orig_hackpi = std::env::var_os("HACKPI_GITHUB_TOKEN");
        let orig_github = std::env::var_os("GITHUB_TOKEN");

        std::env::remove_var("HACKPI_GITHUB_TOKEN");
        std::env::remove_var("GITHUB_TOKEN");

        f();

        match orig_hackpi {
            Some(v) => std::env::set_var("HACKPI_GITHUB_TOKEN", v),
            None => std::env::remove_var("HACKPI_GITHUB_TOKEN"),
        }
        match orig_github {
            Some(v) => std::env::set_var("GITHUB_TOKEN", v),
            None => std::env::remove_var("GITHUB_TOKEN"),
        }
    }

    #[test]
    fn test_from_env_loads_hackpi_github_token() {
        with_env_clean(|| {
            std::env::set_var("HACKPI_GITHUB_TOKEN", "hackpi-token");
            std::env::set_var("GITHUB_TOKEN", "gh-token");

            let config = VcsConfig::from_env(Path::new("/tmp"));

            assert_eq!(config.github_token.as_deref(), Some("hackpi-token"));
            assert_eq!(
                config.github_token_source,
                TokenSource::EnvVar("HACKPI_GITHUB_TOKEN".into())
            );
        });
    }

    #[test]
    fn test_from_env_falls_back_to_github_token() {
        with_env_clean(|| {
            std::env::set_var("GITHUB_TOKEN", "gh-token");

            let config = VcsConfig::from_env(Path::new("/tmp"));

            assert_eq!(config.github_token.as_deref(), Some("gh-token"));
            assert_eq!(
                config.github_token_source,
                TokenSource::EnvVar("GITHUB_TOKEN".into())
            );
        });
    }

    #[test]
    fn test_from_env_no_token() {
        with_env_clean(|| {
            let config = VcsConfig::from_env(Path::new("/tmp"));

            assert!(config.github_token.is_none());
            assert_eq!(config.github_token_source, TokenSource::None);
        });
    }

    #[test]
    fn test_defaults() {
        with_env_clean(|| {
            let config = VcsConfig::from_env(Path::new("/tmp"));

            assert_eq!(config.github_base_url, "https://api.github.com");
            assert_eq!(config.default_remote, "origin");
            assert_eq!(config.default_branch, "main");
        });
    }

    #[test]
    fn test_parse_github_url_ssh_format() {
        let result = VcsConfig::parse_github_url("git@github.com:owner/repo.git");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_url_ssh_format_without_git_suffix() {
        let result = VcsConfig::parse_github_url("git@github.com:owner/repo");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_url_https_format() {
        let result = VcsConfig::parse_github_url("https://github.com/owner/repo.git");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_url_https_format_without_git_suffix() {
        let result = VcsConfig::parse_github_url("https://github.com/owner/repo");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_parse_github_url_invalid_url_returns_none() {
        assert_eq!(VcsConfig::parse_github_url(""), None);
        assert_eq!(VcsConfig::parse_github_url("not-a-github-url"), None);
        assert_eq!(
            VcsConfig::parse_github_url("https://gitlab.com/owner/repo"),
            None
        );
    }

    #[test]
    fn test_parse_github_url_trimmed_whitespace() {
        let result = VcsConfig::parse_github_url("  https://github.com/owner/repo.git  ");
        assert_eq!(result, Some(("owner".to_string(), "repo".to_string())));
    }

    #[test]
    fn test_infer_owner_repo_in_non_git_directory() {
        let temp_dir = std::env::temp_dir();
        let result = VcsConfig::infer_owner_repo(&temp_dir);
        // Just verify it doesn't panic
        let _ = result;
    }
}
