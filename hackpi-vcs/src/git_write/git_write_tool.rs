use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;

use super::branch::{cmd_branch_create, cmd_branch_delete, cmd_checkout};
use super::history::{cmd_commit, cmd_merge, cmd_rebase, cmd_reset};
use super::remote::{cmd_fetch, cmd_pull, cmd_push};
use super::staging::{cmd_add, cmd_stash, cmd_stash_pop};

pub struct GitWriteTool {
    workspace_root: PathBuf,
}

impl GitWriteTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub(crate) fn open_repo(&self) -> Result<git2::Repository, ToolResult> {
        git2::Repository::discover(&self.workspace_root).map_err(|_| ToolResult::SystemError {
            message: "Not a git repository (or any of the parent directories)".into(),
        })
    }

    /// Create remote callbacks for push/pull/fetch with SSH agent and HTTPS fallback.
    ///
    /// Credential resolution order:
    /// 1. SSH agent (for SSH_KEY credential types)
    /// 2. `GIT_USERNAME` / `GIT_PASSWORD` environment variables (for HTTPS)
    /// 3. Clear error — never falls through to `Cred::default()` which would
    ///    prompt on stdin and hang the TUI in raw mode.
    pub(crate) fn create_remote_callbacks<'a>() -> git2::RemoteCallbacks<'a> {
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, allowed_types| {
            if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                return git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"));
            }

            if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                let username = std::env::var("GIT_USERNAME")
                    .ok()
                    .or_else(|| username_from_url.map(|s| s.to_string()));
                let password = std::env::var("GIT_PASSWORD").ok();

                if let (Some(user), Some(pass)) = (username, password) {
                    return git2::Cred::userpass_plaintext(&user, &pass);
                }

                return Err(git2::Error::from_str(
                    "No credentials available. Set GIT_USERNAME and GIT_PASSWORD \
                     environment variables for HTTPS authentication, or use SSH agent.",
                ));
            }

            Err(git2::Error::from_str("No authentication method available"))
        });
        callbacks
    }

    /// Check whether the cancel signal has been set.
    pub(crate) fn is_cancelled(ctx: &ToolContext) -> bool {
        *ctx.signal.borrow()
    }

    /// Compute a short commit stats string like "3 files changed, 12 insertions(+), 5 deletions(-)"
    pub(crate) fn commit_stats(repo: &git2::Repository, commit: &git2::Commit) -> String {
        let tree = match commit.tree() {
            Ok(t) => t,
            Err(_) => return String::new(),
        };
        let parent_tree = commit.parents().next().and_then(|p| p.tree().ok());

        let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) {
            Ok(d) => d,
            Err(_) => return String::new(),
        };

        let stats = match diff.stats() {
            Ok(s) => s,
            Err(_) => return String::new(),
        };

        let files = stats.files_changed();
        let insertions = stats.insertions();
        let deletions = stats.deletions();

        format!("{files} file(s) changed, {insertions} insertion(s)(+), {deletions} deletion(s)(-)")
    }
}

#[async_trait]
impl Tool for GitWriteTool {
    fn name(&self) -> &str {
        "git_write"
    }

    fn description(&self) -> &str {
        "Mutating git operations: add, commit, push, pull, fetch, checkout, branch_create, branch_delete, merge, rebase, stash, stash_pop, reset"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "add", "commit", "push", "pull", "fetch",
                        "checkout", "branch_create", "branch_delete",
                        "merge", "rebase", "stash", "stash_pop", "reset"
                    ],
                    "description": "The git write operation to perform"
                },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "File paths for add/checkout operations"
                },
                "all": {
                    "type": "boolean",
                    "description": "Stage all changes (for add operation)"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (for commit) or stash message (for stash)"
                },
                "remote": {
                    "type": "string",
                    "description": "Remote name (for push/pull/fetch, default: origin)"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch name (for checkout/branch_create/branch_delete/merge)"
                },
                "force": {
                    "type": "boolean",
                    "description": "Force push (for push operation)"
                },
                "create": {
                    "type": "boolean",
                    "description": "Create branch when checking out (for checkout operation)"
                },
                "start_point": {
                    "type": "string",
                    "description": "Start point for branch_create (default: HEAD)"
                },
                "onto": {
                    "type": "string",
                    "description": "Target for rebase operation"
                },
                "index": {
                    "type": "integer",
                    "description": "Stash index for stash_pop (default: 0)"
                },
                "revision": {
                    "type": "string",
                    "description": "Revision for reset operation (default: HEAD)"
                },
                "mode": {
                    "type": "string",
                    "enum": ["soft", "mixed", "hard"],
                    "description": "Reset mode for reset operation (default: mixed)"
                },
                "allow_empty": {
                    "type": "boolean",
                    "description": "Allow empty commits (same tree as parent) for commit operation"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let operation = match params.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'operation' parameter.".into(),
                }
            }
        };

        let mut repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return e,
        };

        match operation {
            "add" => cmd_add(&repo, &params),
            "commit" => cmd_commit(&repo, &params),
            "push" => cmd_push(&repo, &params, ctx),
            "pull" => cmd_pull(&repo, &params, ctx),
            "fetch" => cmd_fetch(&repo, &params, ctx),
            "checkout" => cmd_checkout(&repo, &params),
            "branch_create" => cmd_branch_create(&repo, &params),
            "branch_delete" => cmd_branch_delete(&repo, &params),
            "merge" => cmd_merge(&repo, &params, ctx),
            "rebase" => cmd_rebase(&repo, &params, ctx),
            "stash" => cmd_stash(&mut repo, &params),
            "stash_pop" => cmd_stash_pop(&mut repo, &params),
            "reset" => cmd_reset(&repo, &params),
            _ => ToolResult::SystemError {
                message: format!("Unknown operation: {operation}"),
            },
        }
    }
}
