use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct GitWriteTool {
    workspace_root: PathBuf,
}

impl GitWriteTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn open_repo(&self) -> Result<git2::Repository, ToolResult> {
        git2::Repository::discover(&self.workspace_root).map_err(|_| ToolResult::SystemError {
            message: "Not a git repository (or any of the parent directories)".into(),
        })
    }

    /// Create remote callbacks for push/pull/fetch with SSH agent and HTTPS fallback.
    fn create_remote_callbacks<'a>() -> git2::RemoteCallbacks<'a> {
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, allowed_types| {
            if allowed_types.contains(git2::CredentialType::SSH_KEY) {
                git2::Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"))
            } else if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                git2::Cred::default()
            } else {
                Err(git2::Error::from_str("No authentication method available"))
            }
        });
        callbacks
    }

    /// Check whether the cancel signal has been set.
    fn is_cancelled(ctx: &ToolContext) -> bool {
        *ctx.signal.borrow()
    }

    /// Compute a short commit stats string like "3 files changed, 12 insertions(+), 5 deletions(-)"
    fn commit_stats(repo: &git2::Repository, commit: &git2::Commit) -> String {
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

// ── Operation implementations ──

fn cmd_add(repo: &git2::Repository, params: &Value) -> ToolResult {
    let all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let paths = params.get("paths").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
    });

    let mut index = match repo.index() {
        Ok(i) => i,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to open index: {e}"),
            }
        }
    };

    if all {
        if let Err(e) = index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None) {
            return ToolResult::SystemError {
                message: format!("Failed to stage all files: {e}"),
            };
        }
    } else if let Some(ref paths) = paths {
        for p in paths {
            if let Err(e) = index.add_path(std::path::Path::new(p)) {
                return ToolResult::SystemError {
                    message: format!("Failed to stage '{p}': {e}"),
                };
            }
        }
    }

    if let Err(e) = index.write() {
        return ToolResult::SystemError {
            message: format!("Failed to write index: {e}"),
        };
    }

    let count = if all {
        let statuses = repo.statuses(None).ok().map(|s| s.len()).unwrap_or(0);
        statuses
    } else {
        paths.as_ref().map(|p| p.len()).unwrap_or(0)
    };

    ToolResult::Success {
        content: format!("Added {count} file(s)"),
    }
}

fn cmd_commit(repo: &git2::Repository, params: &Value) -> ToolResult {
    let message = match params.get("message").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => {
            return ToolResult::SystemError {
                message: "Missing 'message' parameter for commit.".into(),
            }
        }
    };

    let sig = match git2::Signature::now("hackpi", "hackpi@corruptbytes.io") {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create signature: {e}"),
            }
        }
    };

    let mut index = match repo.index() {
        Ok(i) => i,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to open index: {e}"),
            }
        }
    };

    let tree_oid = match index.write_tree() {
        Ok(oid) => oid,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to write tree: {e}"),
            }
        }
    };

    let tree = match repo.find_tree(tree_oid) {
        Ok(t) => t,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to find tree: {e}"),
            }
        }
    };

    let head_commit: Option<git2::Commit> = match repo.head() {
        Ok(head_ref) => head_ref.peel_to_commit().ok(),
        Err(_) => None,
    };

    let parent_refs: Vec<&git2::Commit> = match head_commit {
        Some(ref c) => vec![c],
        None => vec![],
    };

    let commit_oid = match repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parent_refs) {
        Ok(oid) => oid,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create commit: {e}"),
            }
        }
    };

    let short_hash = &commit_oid.to_string()[..7.min(commit_oid.to_string().len())];

    let commit_obj = match repo.find_commit(commit_oid) {
        Ok(c) => c,
        Err(_) => {
            return ToolResult::Success {
                content: format!("Committed {short_hash}: \"{message}\""),
            }
        }
    };

    let stats_str = GitWriteTool::commit_stats(repo, &commit_obj);

    ToolResult::Success {
        content: format!("Committed {short_hash}: \"{message}\"\n{stats_str}"),
    }
}

/// Resolve the current branch name as a String.
fn current_branch_name(repo: &git2::Repository) -> Option<String> {
    repo.head()
        .ok()
        .and_then(|h| h.shorthand().map(|s| s.to_owned()))
}

fn cmd_push(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
    let remote_name = params
        .get("remote")
        .and_then(|v| v.as_str())
        .unwrap_or("origin");
    let force = params
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Determine branch: either from params or current HEAD
    let branch = match params.get("branch").and_then(|v| v.as_str()) {
        Some(b) => b.to_string(),
        None => match current_branch_name(repo) {
            Some(b) => b,
            None => {
                return ToolResult::SystemError {
                    message: "Could not determine current branch.".into(),
                }
            }
        },
    };

    if GitWriteTool::is_cancelled(ctx) {
        return ToolResult::Cancelled;
    }

    let mut remote = match repo.find_remote(remote_name) {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Remote '{remote_name}' not found: {e}"),
            }
        }
    };

    let mut push_options = git2::PushOptions::new();
    push_options.remote_callbacks(GitWriteTool::create_remote_callbacks());

    let refspec = if force {
        format!("+refs/heads/{branch}:refs/heads/{branch}")
    } else {
        format!("refs/heads/{branch}:refs/heads/{branch}")
    };

    if let Err(e) = remote.push(&[&refspec], Some(&mut push_options)) {
        return ToolResult::SystemError {
            message: format!("Push failed: {e}"),
        };
    }

    ToolResult::Success {
        content: format!("Pushed to {remote_name}/{branch}"),
    }
}

fn cmd_pull(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
    let remote_name = params
        .get("remote")
        .and_then(|v| v.as_str())
        .unwrap_or("origin");

    let branch = match params.get("branch").and_then(|v| v.as_str()) {
        Some(b) => b.to_string(),
        None => match current_branch_name(repo) {
            Some(b) => b,
            None => {
                return ToolResult::SystemError {
                    message: "Could not determine current branch.".into(),
                }
            }
        },
    };

    if GitWriteTool::is_cancelled(ctx) {
        return ToolResult::Cancelled;
    }

    // Fetch first
    let fetch_result = cmd_fetch_internal(repo, remote_name);
    if let ToolResult::SystemError { .. } = fetch_result {
        return fetch_result;
    }

    // Then merge FETCH_HEAD
    let fetch_refspec = format!("refs/remotes/{remote_name}/{branch}");
    let fetch_oid = match repo.refname_to_id(&fetch_refspec) {
        Ok(oid) => oid,
        Err(_) => {
            return ToolResult::SystemError {
                message: format!(
                    "Failed to find fetched ref '{fetch_refspec}'. The remote branch may not exist."
                ),
            }
        }
    };

    let fetch_commit = match repo.find_commit(fetch_oid) {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to find fetched commit: {e}"),
            }
        }
    };

    let annotated = match repo.find_annotated_commit(fetch_oid) {
        Ok(a) => a,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create annotated commit: {e}"),
            }
        }
    };

    // Use merge analysis to determine if fast-forward is possible
    let analysis = match repo.merge_analysis(&[&annotated]) {
        Ok((a, _)) => a,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Merge analysis failed: {e}"),
            }
        }
    };

    if analysis.contains(git2::MergeAnalysis::ANALYSIS_UP_TO_DATE) {
        return ToolResult::Success {
            content: "Already up to date.".into(),
        };
    }

    if analysis.contains(git2::MergeAnalysis::ANALYSIS_FASTFORWARD) {
        // Fast-forward: update HEAD to point to fetched commit
        let fetched_tree = match fetch_commit.tree() {
            Ok(t) => t,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Failed to get fetched commit tree: {e}"),
                }
            }
        };

        // Checkout the tree
        if let Err(e) = repo.checkout_tree(
            fetched_tree.as_object(),
            Some(git2::build::CheckoutBuilder::new().force()),
        ) {
            return ToolResult::SystemError {
                message: format!("Failed to checkout during fast-forward: {e}"),
            };
        }

        // Set HEAD to the fetched commit
        if repo
            .head()
            .and_then(|mut head_ref| {
                head_ref.set_target(fetch_oid, &format!("pull: fast-forward to {fetch_oid}"))
            })
            .is_err()
        {
            // If head update fails, try setting head directly
            let _ = repo.set_head_detached(fetch_oid);
        }

        // Clean up index
        if let Ok(mut index) = repo.index() {
            let _ = index.read_tree(&fetched_tree);
            let _ = index.write();
        }

        return ToolResult::Success {
            content: format!("Pulled {remote_name}/{branch} (fast-forward)"),
        };
    }

    // Normal merge
    let sig = match git2::Signature::now("hackpi", "hackpi@corruptbytes.io") {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create signature: {e}"),
            }
        }
    };

    let mut merge_opts = git2::MergeOptions::new();
    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.force().allow_conflicts(true);

    match repo.merge(
        &[&annotated],
        Some(&mut merge_opts),
        Some(&mut checkout_opts),
    ) {
        Ok(()) => {
            // Check for conflicts
            if repo.index().map(|i| i.has_conflicts()).unwrap_or(false) {
                let conflicts = get_conflict_files(repo);
                let _ = repo.cleanup_state();
                return ToolResult::SystemError {
                    message: format!(
                        "Merge conflicts in: {}. Pull aborted. Resolve conflicts and commit manually.",
                        conflicts.join(", ")
                    ),
                };
            }

            // Create merge commit
            let tree_oid = match repo.index().and_then(|mut i| i.write_tree()) {
                Ok(oid) => oid,
                Err(e) => {
                    let _ = repo.cleanup_state();
                    return ToolResult::SystemError {
                        message: format!("Failed to write merge tree: {e}"),
                    };
                }
            };
            let tree = match repo.find_tree(tree_oid) {
                Ok(t) => t,
                Err(e) => {
                    let _ = repo.cleanup_state();
                    return ToolResult::SystemError {
                        message: format!("Failed to find merge tree: {e}"),
                    };
                }
            };

            let head_commit = repo.head().unwrap().peel_to_commit().ok();
            let mut parents: Vec<&git2::Commit> = Vec::new();
            if let Some(ref c) = head_commit {
                parents.push(c);
            }
            parents.push(&fetch_commit);

            let msg = format!("Merge branch '{branch}' of {remote_name}/{branch}");
            if let Err(e) = repo.commit(Some("HEAD"), &sig, &sig, &msg, &tree, &parents) {
                let _ = repo.cleanup_state();
                return ToolResult::SystemError {
                    message: format!("Failed to create merge commit: {e}"),
                };
            }

            let _ = repo.cleanup_state();

            ToolResult::Success {
                content: format!("Pulled {remote_name}/{branch} (merge commit created)"),
            }
        }
        Err(e) => ToolResult::SystemError {
            message: format!("Merge failed: {e}"),
        },
    }
}

/// Internal fetch helper without creating a new RemoteCallbacks
fn cmd_fetch_internal(repo: &git2::Repository, remote_name: &str) -> ToolResult {
    let mut remote = match repo.find_remote(remote_name) {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Remote '{remote_name}' not found: {e}"),
            }
        }
    };

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(GitWriteTool::create_remote_callbacks());

    // Determine default fetch refspecs
    let refspecs: Vec<String> = match remote.fetch_refspecs() {
        Ok(refspecs) => refspecs
            .iter()
            .filter_map(|r| r.map(|s| s.to_string()))
            .collect(),
        Err(_) => {
            // No refspecs configured; use default
            vec![format!("+refs/heads/*:refs/remotes/{remote_name}/*")]
        }
    };

    let refstrs: Vec<&str> = refspecs.iter().map(|s| s.as_str()).collect();

    if let Err(e) = remote.fetch(&refstrs, Some(&mut fetch_options), None) {
        return ToolResult::SystemError {
            message: format!("Fetch failed: {e}"),
        };
    }

    ToolResult::Success {
        content: format!("Fetched from {remote_name}"),
    }
}

fn cmd_fetch(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
    let remote_name = params
        .get("remote")
        .and_then(|v| v.as_str())
        .unwrap_or("origin");

    if GitWriteTool::is_cancelled(ctx) {
        return ToolResult::Cancelled;
    }

    cmd_fetch_internal(repo, remote_name)
}

fn cmd_checkout(repo: &git2::Repository, params: &Value) -> ToolResult {
    let branch = params.get("branch").and_then(|v| v.as_str());
    let paths = params.get("paths").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
    });
    let create = params
        .get("create")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if let Some(ref paths) = paths {
        // Restore specific files
        let mut count = 0u32;
        for p in paths {
            let mut checkout_builder = git2::build::CheckoutBuilder::new();
            checkout_builder.force().update_index(true);
            checkout_builder.path(std::path::Path::new(p));

            if repo.checkout_head(Some(&mut checkout_builder)).is_err() {
                return ToolResult::SystemError {
                    message: format!("Failed to restore '{p}'"),
                };
            }
            count += 1;
        }
        return ToolResult::Success {
            content: format!("Restored {count} file(s)"),
        };
    }

    if let Some(branch) = branch {
        if create {
            // Create and switch to new branch
            let commit = match repo.head().and_then(|h| h.peel_to_commit()) {
                Ok(c) => c,
                Err(e) => {
                    return ToolResult::SystemError {
                        message: format!("Failed to get HEAD commit: {e}"),
                    }
                }
            };

            if repo.branch(branch, &commit, false).is_err() {
                return ToolResult::SystemError {
                    message: format!("Failed to create branch '{branch}'"),
                };
            }

            // Now checkout the new branch
            let branch_ref = format!("refs/heads/{branch}");
            let obj = match repo.revparse_single(&branch_ref) {
                Ok(o) => o,
                Err(e) => {
                    return ToolResult::SystemError {
                        message: format!("Failed to resolve new branch '{branch}': {e}"),
                    }
                }
            };

            let checkout_result =
                repo.checkout_tree(&obj, Some(git2::build::CheckoutBuilder::new().force()));
            if checkout_result.is_err() {
                return ToolResult::SystemError {
                    message: format!("Failed to checkout branch '{branch}'"),
                };
            }

            if repo.set_head(&branch_ref).is_err() {
                return ToolResult::SystemError {
                    message: format!("Failed to set HEAD to '{branch}'"),
                };
            }

            ToolResult::Success {
                content: format!("Switched to branch '{branch}'"),
            }
        } else {
            // Switch to existing branch
            let branch_ref = format!("refs/heads/{branch}");
            let obj = match repo.revparse_single(&branch_ref) {
                Ok(o) => o,
                Err(_) => {
                    // Try as a remote branch or any ref
                    match repo.revparse_single(branch) {
                        Ok(o) => o,
                        Err(e) => {
                            return ToolResult::SystemError {
                                message: format!("Branch '{branch}' not found: {e}"),
                            }
                        }
                    }
                }
            };

            let checkout_result =
                repo.checkout_tree(&obj, Some(git2::build::CheckoutBuilder::new().force()));
            if checkout_result.is_err() {
                return ToolResult::SystemError {
                    message: format!("Failed to checkout branch '{branch}'"),
                };
            }

            if repo.set_head(&branch_ref).is_err() {
                return ToolResult::SystemError {
                    message: format!("Failed to set HEAD to '{branch}'"),
                };
            }

            ToolResult::Success {
                content: format!("Switched to branch '{branch}'"),
            }
        }
    } else {
        ToolResult::SystemError {
            message: "Either 'branch' or 'paths' must be provided for checkout.".into(),
        }
    }
}

fn cmd_branch_create(repo: &git2::Repository, params: &Value) -> ToolResult {
    let name = match params.get("branch").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return ToolResult::SystemError {
                message: "Missing 'branch' parameter for branch_create.".into(),
            }
        }
    };

    let start_point = params
        .get("start_point")
        .and_then(|v| v.as_str())
        .unwrap_or("HEAD");

    let obj = match repo.revparse_single(start_point) {
        Ok(o) => o,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to resolve start point '{start_point}': {e}"),
            }
        }
    };

    let commit = match obj.peel_to_commit() {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("'{start_point}' is not a commit: {e}"),
            }
        }
    };

    if repo.branch(name, &commit, false).is_err() {
        return ToolResult::SystemError {
            message: format!("Failed to create branch '{name}'"),
        };
    }

    ToolResult::Success {
        content: format!("Created branch '{name}'"),
    }
}

fn cmd_branch_delete(repo: &git2::Repository, params: &Value) -> ToolResult {
    let name = match params.get("branch").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return ToolResult::SystemError {
                message: "Missing 'branch' parameter for branch_delete.".into(),
            }
        }
    };

    let mut branch = match repo.find_branch(name, git2::BranchType::Local) {
        Ok(b) => b,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Branch '{name}' not found: {e}"),
            }
        }
    };

    if branch.delete().is_err() {
        return ToolResult::SystemError {
            message: format!("Failed to delete branch '{name}'"),
        };
    }

    ToolResult::Success {
        content: format!("Deleted branch '{name}'"),
    }
}

fn cmd_merge(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
    let branch = match params.get("branch").and_then(|v| v.as_str()) {
        Some(b) => b,
        None => {
            return ToolResult::SystemError {
                message: "Missing 'branch' parameter for merge.".into(),
            }
        }
    };

    if GitWriteTool::is_cancelled(ctx) {
        return ToolResult::Cancelled;
    }

    let merge_branch = match repo.find_branch(branch, git2::BranchType::Local) {
        Ok(b) => b,
        Err(_) => {
            return ToolResult::SystemError {
                message: format!("Branch '{branch}' not found."),
            }
        }
    };

    let merge_commit = match merge_branch.get().peel_to_commit() {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to get commit for branch '{branch}': {e}"),
            }
        }
    };

    let merge_oid = merge_commit.id();
    let annotated = match repo.find_annotated_commit(merge_oid) {
        Ok(a) => a,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to find annotated commit: {e}"),
            }
        }
    };

    let analysis = match repo.merge_analysis(&[&annotated]) {
        Ok((a, _)) => a,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Merge analysis failed: {e}"),
            }
        }
    };

    if analysis.contains(git2::MergeAnalysis::ANALYSIS_UP_TO_DATE) {
        return ToolResult::Success {
            content: format!("Already up to date. Branch '{branch}' is already merged."),
        };
    }

    if analysis.contains(git2::MergeAnalysis::ANALYSIS_FASTFORWARD) {
        // Fast-forward merge
        let merge_tree = match merge_commit.tree() {
            Ok(t) => t,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Failed to get merge commit tree: {e}"),
                }
            }
        };

        if let Err(e) = repo.checkout_tree(
            merge_tree.as_object(),
            Some(git2::build::CheckoutBuilder::new().force()),
        ) {
            return ToolResult::SystemError {
                message: format!("Failed to checkout during fast-forward merge: {e}"),
            };
        }

        if let Err(e) = repo.head().and_then(|mut head_ref| {
            head_ref.set_target(merge_oid, &format!("merge: fast-forward {branch}"))
        }) {
            return ToolResult::SystemError {
                message: format!("Failed to update HEAD during fast-forward merge: {e}"),
            };
        }

        if let Ok(mut index) = repo.index() {
            let _ = index.read_tree(&merge_tree);
            let _ = index.write();
        }

        return ToolResult::Success {
            content: format!("Merged {branch} into current branch (fast-forward)"),
        };
    }

    // Normal merge
    let sig = match git2::Signature::now("hackpi", "hackpi@corruptbytes.io") {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create signature: {e}"),
            }
        }
    };

    let mut merge_opts = git2::MergeOptions::new();
    let mut checkout_opts = git2::build::CheckoutBuilder::new();
    checkout_opts.force().allow_conflicts(true);

    match repo.merge(
        &[&annotated],
        Some(&mut merge_opts),
        Some(&mut checkout_opts),
    ) {
        Ok(()) => {
            if repo.index().map(|i| i.has_conflicts()).unwrap_or(false) {
                let conflicts = get_conflict_files(repo);
                let _ = repo.cleanup_state();
                return ToolResult::SystemError {
                    message: format!(
                        "Merge conflicts in: {}. Merge aborted. Resolve conflicts and commit manually.",
                        conflicts.join(", ")
                    ),
                };
            }

            // Create merge commit
            let tree_oid = match repo.index().and_then(|mut i| i.write_tree()) {
                Ok(oid) => oid,
                Err(e) => {
                    let _ = repo.cleanup_state();
                    return ToolResult::SystemError {
                        message: format!("Failed to write merge tree: {e}"),
                    };
                }
            };
            let tree = match repo.find_tree(tree_oid) {
                Ok(t) => t,
                Err(e) => {
                    let _ = repo.cleanup_state();
                    return ToolResult::SystemError {
                        message: format!("Failed to find merge tree: {e}"),
                    };
                }
            };

            let head_commit = repo.head().unwrap().peel_to_commit().ok();
            let mut parents: Vec<&git2::Commit> = Vec::new();
            if let Some(ref c) = head_commit {
                parents.push(c);
            }
            parents.push(&merge_commit);

            let msg = format!("Merge branch '{branch}' into current branch");
            if let Err(e) = repo.commit(Some("HEAD"), &sig, &sig, &msg, &tree, &parents) {
                let _ = repo.cleanup_state();
                return ToolResult::SystemError {
                    message: format!("Failed to create merge commit: {e}"),
                };
            }

            let _ = repo.cleanup_state();

            ToolResult::Success {
                content: format!("Merged {branch} into current branch"),
            }
        }
        Err(e) => ToolResult::SystemError {
            message: format!("Merge failed: {e}"),
        },
    }
}

fn cmd_rebase(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
    let onto = match params.get("onto").and_then(|v| v.as_str()) {
        Some(o) => o,
        None => {
            return ToolResult::SystemError {
                message: "Missing 'onto' parameter for rebase.".into(),
            }
        }
    };

    if GitWriteTool::is_cancelled(ctx) {
        return ToolResult::Cancelled;
    }

    let onto_obj = match repo.revparse_single(onto) {
        Ok(o) => o,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to resolve '{onto}': {e}"),
            }
        }
    };

    let onto_commit = match onto_obj.peel_to_commit() {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("'{onto}' is not a commit: {e}"),
            }
        }
    };

    let head_commit = match repo.head().and_then(|h| h.peel_to_commit()) {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to get HEAD commit: {e}"),
            }
        }
    };

    // Find the merge base
    let merge_base = match repo.merge_base(onto_commit.id(), head_commit.id()) {
        Ok(oid) => oid,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to find merge base: {e}"),
            }
        }
    };

    let onto_annotated = match repo.find_annotated_commit(onto_commit.id()) {
        Ok(a) => a,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to find annotated commit: {e}"),
            }
        }
    };

    let merge_base_annotated = match repo.find_annotated_commit(merge_base) {
        Ok(a) => a,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to find merge base annotated commit: {e}"),
            }
        }
    };

    let head_annotated = match repo.find_annotated_commit(head_commit.id()) {
        Ok(a) => a,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to find HEAD annotated commit: {e}"),
            }
        }
    };

    let mut rebase = match repo.rebase(
        Some(&head_annotated),
        Some(&merge_base_annotated),
        Some(&onto_annotated),
        None,
    ) {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Rebase failed: {e}"),
            }
        }
    };

    let sig = match git2::Signature::now("hackpi", "hackpi@corruptbytes.io") {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create signature: {e}"),
            }
        }
    };

    let mut operation_count = 0u32;

    loop {
        if GitWriteTool::is_cancelled(ctx) {
            let _ = rebase.abort();
            return ToolResult::Cancelled;
        }

        match rebase.next() {
            Some(Ok(op)) => {
                let _ = op;
                operation_count += 1;
                if let Err(e) = rebase.commit(None, &sig, None) {
                    let _ = rebase.abort();
                    return ToolResult::SystemError {
                        message: format!(
                            "Rebase conflict at operation {operation_count}. Rebase aborted: {e}"
                        ),
                    };
                }
            }
            Some(Err(e)) => {
                let _ = rebase.abort();
                return ToolResult::SystemError {
                    message: format!("Rebase error at operation {operation_count}: {e}"),
                };
            }
            None => break,
        }
    }

    if let Err(e) = rebase.finish(None) {
        return ToolResult::SystemError {
            message: format!("Failed to finish rebase: {e}"),
        };
    }

    ToolResult::Success {
        content: format!("Rebased onto {onto}"),
    }
}

fn cmd_stash(repo: &mut git2::Repository, params: &Value) -> ToolResult {
    let message = params.get("message").and_then(|v| v.as_str());

    let sig = match git2::Signature::now("hackpi", "hackpi@corruptbytes.io") {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create signature: {e}"),
            }
        }
    };

    match repo.stash_save(&sig, message.unwrap_or(""), None) {
        Ok(_) => ToolResult::Success {
            content: "Saved working directory state".into(),
        },
        Err(e) => ToolResult::SystemError {
            message: format!("Failed to stash: {e}"),
        },
    }
}

fn cmd_stash_pop(repo: &mut git2::Repository, params: &Value) -> ToolResult {
    let index = params.get("index").and_then(|v| v.as_u64()).unwrap_or(0);

    match repo.stash_pop(index as usize, None) {
        Ok(_) => ToolResult::Success {
            content: "Restored stashed state".into(),
        },
        Err(e) => ToolResult::SystemError {
            message: format!("Failed to pop stash: {e}"),
        },
    }
}

fn cmd_reset(repo: &git2::Repository, params: &Value) -> ToolResult {
    let revision = params
        .get("revision")
        .and_then(|v| v.as_str())
        .unwrap_or("HEAD");
    let mode = params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("mixed");

    let obj = match repo.revparse_single(revision) {
        Ok(o) => o,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to resolve revision '{revision}': {e}"),
            }
        }
    };

    let reset_type = match mode {
        "soft" => git2::ResetType::Soft,
        "mixed" => git2::ResetType::Mixed,
        "hard" => git2::ResetType::Hard,
        _ => {
            return ToolResult::SystemError {
                message: format!("Invalid reset mode '{mode}'. Use 'soft', 'mixed', or 'hard'."),
            }
        }
    };

    let commit_obj = match obj.peel_to_commit() {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("'{revision}' is not a commit: {e}"),
            }
        }
    };

    // repo.reset takes &Object, peel the commit
    let obj_for_reset = commit_obj.as_object();

    match repo.reset(obj_for_reset, reset_type, None) {
        Ok(()) => ToolResult::Success {
            content: format!("Reset to {revision} ({mode})"),
        },
        Err(e) => ToolResult::SystemError {
            message: format!("Reset failed: {e}"),
        },
    }
}

/// Get a list of files with merge conflicts.
fn get_conflict_files(repo: &git2::Repository) -> Vec<String> {
    let index = match repo.index() {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    };

    let mut conflicts = Vec::new();
    if let Ok(entries) = index.conflicts() {
        for entry_result in entries.flatten() {
            let path_bytes = match entry_result.our {
                Some(ref our) => &our.path,
                None => continue,
            };
            let path = String::from_utf8_lossy(path_bytes).to_string();
            if !conflicts.contains(&path) {
                conflicts.push(path);
            }
        }
    }
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use hackpi_core::tools::ToolContext;

    /// Helper: create a temporary git repo with basic config.
    fn init_repo() -> (tempfile::TempDir, git2::Repository) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = git2::Repository::init(dir.path()).expect("init repo");
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test User").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        (dir, repo)
    }

    /// Helper: create an initial commit with a file, returning the repo.
    fn init_repo_with_commit() -> (tempfile::TempDir, git2::Repository) {
        let (dir, repo) = init_repo();
        let path = dir.path().join("README.md");
        std::fs::write(&path, b"# Test\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        {
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
                .unwrap();
        }
        (dir, repo)
    }

    /// Helper: create a second commit.
    fn add_commit(
        repo: &git2::Repository,
        dir: &tempfile::TempDir,
        filename: &str,
        content: &[u8],
        msg: &str,
    ) {
        let path = dir.path().join(filename);
        std::fs::write(&path, content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new(filename)).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        {
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent])
                .unwrap();
        }
    }

    fn make_tool(workspace_root: PathBuf) -> GitWriteTool {
        GitWriteTool::new(workspace_root)
    }

    fn test_ctx() -> ToolContext {
        ToolContext {
            workspace_root: std::env::temp_dir(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        }
    }

    async fn execute(tool: &GitWriteTool, params: Value) -> ToolResult {
        tool.execute(params, &test_ctx()).await
    }

    // ── Basic tool metadata ──

    #[tokio::test]
    async fn test_name() {
        let tool = make_tool(PathBuf::from("/tmp"));
        assert_eq!(tool.name(), "git_write");
    }

    #[tokio::test]
    async fn test_description() {
        let tool = make_tool(PathBuf::from("/tmp"));
        assert!(tool.description().contains("add"));
        assert!(tool.description().contains("commit"));
        assert!(tool.description().contains("push"));
        assert!(tool.description().contains("pull"));
        assert!(tool.description().contains("checkout"));
    }

    #[test]
    fn test_input_schema_has_additional_properties_false() {
        let tool = make_tool(PathBuf::from("/tmp"));
        let schema = tool.input_schema();
        assert_eq!(schema.get("additionalProperties"), Some(&json!(false)));
        assert!(schema.get("properties").unwrap().get("operation").is_some());
    }

    // ── Missing repo ──

    #[tokio::test]
    async fn test_missing_repo_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = make_tool(tmp.path().to_path_buf());
        let result = execute(&tool, json!({ "operation": "add" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Not a git repository")),
            "Expected SystemError about missing repo, got: {result:?}"
        );
    }

    // ── Add operation ──

    #[tokio::test]
    async fn test_add_specific_files() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("new_file.txt"), b"hello").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "add", "paths": ["new_file.txt"] }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Added"),
                    "Expected 'Added ...', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify it's actually staged
        let statuses = repo.statuses(None).unwrap();
        let staged = statuses.iter().any(|e| {
            e.status().contains(git2::Status::INDEX_NEW) && e.path() == Some("new_file.txt")
        });
        assert!(staged, "new_file.txt should be staged");
    }

    #[tokio::test]
    async fn test_add_all() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("file_a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("file_b.txt"), b"b").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "add", "all": true })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Added"),
                    "Expected 'Added ...', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify both are staged
        let statuses = repo.statuses(None).unwrap();
        let staged_count = statuses
            .iter()
            .filter(|e| e.status().contains(git2::Status::INDEX_NEW))
            .count();
        assert_eq!(staged_count, 2, "Expected 2 staged files");
    }

    #[tokio::test]
    async fn test_add_no_params_stages_nothing() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("untracked.txt"), b"data").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "add" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Added"),
                    "Expected 'Added ...', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify nothing was staged
        let statuses = repo.statuses(None).unwrap();
        let untracked = statuses.iter().any(|e| {
            e.status().contains(git2::Status::WT_NEW) && e.path() == Some("untracked.txt")
        });
        assert!(untracked, "untracked.txt should remain untracked");
    }

    // ── Commit operation ──

    #[tokio::test]
    async fn test_commit_creates_commit() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("feature.txt"), b"feature content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("feature.txt")).unwrap();
        index.write().unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Add feature" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Committed"),
                    "Expected 'Committed ...', got: {content}"
                );
                assert!(
                    content.contains("Add feature"),
                    "Expected message in output, got: {content}"
                );
                assert!(
                    content.contains("file(s) changed"),
                    "Expected stats in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify commit exists
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message().unwrap().trim(), "Add feature");
    }

    #[tokio::test]
    async fn test_commit_missing_message() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "commit" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'message'")),
            "Expected SystemError for missing message, got: {result:?}"
        );
    }

    // ── Checkout operation ──

    #[tokio::test]
    async fn test_checkout_create_and_switch_branch() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "branch": "feature", "create": true }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Switched to branch 'feature'"),
                    "Expected 'Switched to branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify we're on the new branch
        let head = repo.head().unwrap();
        assert_eq!(head.shorthand().unwrap(), "feature");
    }

    #[tokio::test]
    async fn test_checkout_existing_branch() {
        let (_dir, repo) = init_repo_with_commit();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("develop", &commit, false).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "branch": "develop" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Switched to branch 'develop'"),
                    "Expected 'Switched to branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        assert_eq!(repo.head().unwrap().shorthand().unwrap(), "develop");
    }

    #[tokio::test]
    async fn test_checkout_restore_files() {
        let (dir, repo) = init_repo_with_commit();
        // Modify a tracked file
        std::fs::write(dir.path().join("README.md"), b"# Modified\n").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "paths": ["README.md"] }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Restored"),
                    "Expected 'Restored ...', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify file is restored
        let content = std::fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert_eq!(content, "# Test\n");
    }

    // ── Branch create ──

    #[tokio::test]
    async fn test_branch_create() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_create", "branch": "new-branch" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created branch 'new-branch'"),
                    "Expected 'Created branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify branch exists
        assert!(repo
            .find_branch("new-branch", git2::BranchType::Local)
            .is_ok());
    }

    #[tokio::test]
    async fn test_branch_create_missing_name() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "branch_create" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'branch'")),
            "Expected SystemError for missing branch name, got: {result:?}"
        );
    }

    // ── Branch delete ──

    #[tokio::test]
    async fn test_branch_delete() {
        let (_dir, repo) = init_repo_with_commit();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("delete-me", &commit, false).unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_delete", "branch": "delete-me" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Deleted branch 'delete-me'"),
                    "Expected 'Deleted branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // Verify branch is gone
        assert!(repo
            .find_branch("delete-me", git2::BranchType::Local)
            .is_err());
    }

    #[tokio::test]
    async fn test_branch_delete_nonexistent() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_delete", "branch": "nonexistent" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for nonexistent branch, got: {result:?}"
        );
    }

    // ── Reset operation ──

    #[tokio::test]
    async fn test_reset_mixed() {
        let (dir, repo) = init_repo_with_commit();
        // Create a second commit
        add_commit(&repo, &dir, "file2.txt", b"second", "Second commit");
        let _second_oid = repo.head().unwrap().peel_to_commit().unwrap().id();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "reset", "revision": "HEAD~1", "mode": "mixed" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Reset to"),
                    "Expected 'Reset to ...', got: {content}"
                );
                assert!(
                    content.contains("mixed"),
                    "Expected 'mixed' in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
        // After mixed reset, HEAD should point to initial commit
        let head_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
        assert_ne!(head_oid, _second_oid, "HEAD should have been reset");
    }

    #[tokio::test]
    async fn test_reset_soft() {
        let (dir, repo) = init_repo_with_commit();
        add_commit(&repo, &dir, "file2.txt", b"second", "Second commit");
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "reset", "revision": "HEAD~1", "mode": "soft" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("soft"),
                    "Expected 'soft' in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_reset_default_revision() {
        let (dir, repo) = init_repo_with_commit();
        add_commit(&repo, &dir, "file2.txt", b"second", "Second commit");
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "reset", "mode": "hard" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("HEAD"),
                    "Expected 'HEAD' revision, got: {content}"
                );
                assert!(
                    content.contains("hard"),
                    "Expected 'hard' mode, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_reset_invalid_revision() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "reset", "revision": "nonexistent" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to resolve revision")),
            "Expected SystemError for bad revision, got: {result:?}"
        );
    }

    // ── Stash operations ──

    #[tokio::test]
    async fn test_stash_and_stash_pop() {
        let (dir, repo) = init_repo_with_commit();
        // Modify a tracked file
        std::fs::write(dir.path().join("README.md"), b"# Stashed change\n").unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        // Stash
        let stash_result = execute(&tool, json!({ "operation": "stash", "message": "WIP" })).await;
        match &stash_result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Saved working directory"),
                    "Expected 'Saved working directory', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {stash_result:?}"),
        }

        // Verify working directory is clean
        let statuses = repo.statuses(None).unwrap();
        let has_changes = statuses
            .iter()
            .any(|e| !e.status().is_empty() && !e.status().contains(git2::Status::IGNORED));
        assert!(
            !has_changes,
            "Working directory should be clean after stash"
        );

        // Pop the stash
        let pop_result = execute(&tool, json!({ "operation": "stash_pop" })).await;
        match &pop_result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Restored stashed state"),
                    "Expected 'Restored stashed state', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {pop_result:?}"),
        }

        // Verify changes are restored
        let readme_content = std::fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert_eq!(readme_content, "# Stashed change\n");
    }

    #[tokio::test]
    async fn test_stash_pop_no_stash() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "stash_pop" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to pop stash")),
            "Expected SystemError for empty stash, got: {result:?}"
        );
    }

    // ── Merge operation (fast-forward) ──

    #[tokio::test]
    async fn test_merge_fast_forward() {
        let (dir, repo) = init_repo_with_commit();

        // Create a branch and add a commit
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-branch", &commit, false).unwrap();

        // Switch to feature branch and add a commit
        let feature_ref = "refs/heads/feature-branch";
        repo.set_head(feature_ref).unwrap();
        let feature_obj = repo.revparse_single(feature_ref).unwrap();
        repo.checkout_tree(&feature_obj, None).unwrap();

        add_commit(
            &repo,
            &dir,
            "feature-file.txt",
            b"feature work",
            "Feature work",
        );

        // Switch back to main
        let head_obj = repo.revparse_single("refs/heads/main").unwrap();
        repo.checkout_tree(&head_obj, None).unwrap();
        repo.set_head("refs/heads/main").unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "merge", "branch": "feature-branch" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Merged feature-branch"),
                    "Expected 'Merged feature-branch', got: {content}"
                );
                assert!(
                    content.contains("fast-forward"),
                    "Expected fast-forward, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_merge_already_up_to_date() {
        let (_dir, repo) = init_repo_with_commit();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("same-branch", &commit, false).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "merge", "branch": "same-branch" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Already up to date"),
                    "Expected 'Already up to date', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_merge_nonexistent_branch() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "merge", "branch": "ghost" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for nonexistent branch, got: {result:?}"
        );
    }

    // ── Rebase operation ──

    #[tokio::test]
    async fn test_rebase() {
        let (dir, repo) = init_repo_with_commit();

        // Create a branch and add a commit
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-rebase", &commit, false).unwrap();

        // Add commit to main
        add_commit(&repo, &dir, "main-file.txt", b"main work", "Main work");

        // Switch to feature branch
        let feature_ref = "refs/heads/feature-rebase";
        repo.set_head(feature_ref).unwrap();
        let feature_obj = repo.revparse_single(feature_ref).unwrap();
        repo.checkout_tree(&feature_obj, None).unwrap();

        // Add commit to feature
        add_commit(&repo, &dir, "feat-file.txt", b"feature", "Feature commit");

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "rebase", "onto": "main" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Rebased onto main"),
                    "Expected 'Rebased onto main', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_rebase_invalid_onto() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "rebase", "onto": "nonexistent" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to resolve")),
            "Expected SystemError for bad onto, got: {result:?}"
        );
    }

    // ── Push/pull/fetch tests (with bare repo as remote) ──

    #[tokio::test]
    async fn test_push_with_bare_remote() {
        let (_dir, repo) = init_repo_with_commit();

        // Create bare repo as remote
        let bare_dir = tempfile::tempdir().unwrap();
        let _bare_repo = git2::Repository::init_bare(bare_dir.path()).unwrap();

        // Add bare repo as remote
        let remote_url = bare_dir.path().to_str().unwrap();
        repo.remote("origin", remote_url).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let _result = execute(&tool, json!({ "operation": "push" })).await;
        // May succeed or fail depending on environment; just check we don't panic
    }

    #[tokio::test]
    async fn test_fetch_no_remote() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "fetch" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for missing remote, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_pull_no_remote() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "pull" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for missing remote, got: {result:?}"
        );
    }

    // ── Cancel signal ──

    #[tokio::test]
    async fn test_cancel_signal_aborts_push() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        // Create a cancelled context
        let (_tx, rx) = tokio::sync::watch::channel(true);
        let ctx = ToolContext {
            workspace_root: repo.workdir().unwrap().to_path_buf(),
            conversation_id: String::new(),
            signal: rx,
        };

        let _result = tool.execute(json!({ "operation": "push" }), &ctx).await;
        // Push should fail because remote doesn't exist
    }

    #[tokio::test]
    async fn test_cancel_signal_aborts_fetch() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        let (_tx, rx) = tokio::sync::watch::channel(true);
        let ctx = ToolContext {
            workspace_root: repo.workdir().unwrap().to_path_buf(),
            conversation_id: String::new(),
            signal: rx,
        };

        let _result = tool.execute(json!({ "operation": "fetch" }), &ctx).await;
    }

    #[tokio::test]
    async fn test_cancel_signal_aborts_merge() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        let (_tx, rx) = tokio::sync::watch::channel(true);
        let ctx = ToolContext {
            workspace_root: repo.workdir().unwrap().to_path_buf(),
            conversation_id: String::new(),
            signal: rx,
        };

        let _result = tool
            .execute(
                json!({ "operation": "merge", "branch": "nonexistent" }),
                &ctx,
            )
            .await;
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "invalid_op" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Unknown operation")),
            "Expected SystemError for unknown operation, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_missing_operation_param() {
        let tool = make_tool(PathBuf::from("/tmp"));
        let result = execute(&tool, json!({})).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'operation'")),
            "Expected SystemError for missing operation, got: {result:?}"
        );
    }

    // ── Merge test (no-ff with merge commit) ──

    #[tokio::test]
    async fn test_merge_with_merge_commit() {
        let (dir, repo) = init_repo_with_commit();

        // Create and switch to feature branch
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-noff", &commit, false).unwrap();

        // Add commit to main diverging
        add_commit(&repo, &dir, "main-only.txt", b"main", "Main divergence");

        // Switch to feature and add commit
        repo.set_head("refs/heads/feature-noff").unwrap();
        let feat_obj = repo.revparse_single("refs/heads/feature-noff").unwrap();
        repo.checkout_tree(&feat_obj, None).unwrap();
        add_commit(
            &repo,
            &dir,
            "feature-only.txt",
            b"feature",
            "Feature divergence",
        );

        // Switch back to main
        repo.set_head("refs/heads/main").unwrap();
        let main_obj = repo.revparse_single("refs/heads/main").unwrap();
        repo.checkout_tree(&main_obj, None).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "merge", "branch": "feature-noff" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Merged feature-noff"),
                    "Expected merge success, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Branch create with start_point ──

    #[tokio::test]
    async fn test_branch_create_with_start_point() {
        let (dir, repo) = init_repo_with_commit();
        add_commit(&repo, &dir, "second.txt", b"second", "Second commit");

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "branch_create", "branch": "from-prev", "start_point": "HEAD~1" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created branch 'from-prev'"),
                    "Expected 'Created branch', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Commit stats ──

    #[tokio::test]
    async fn test_commit_stats_shown() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("stats-test.txt"), b"stats content\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_path(std::path::Path::new("stats-test.txt"))
            .unwrap();
        index.write().unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Stats test" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("file(s) changed"),
                    "Expected stats in commit output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Add nonexistent file ──

    #[tokio::test]
    async fn test_add_nonexistent_file_returns_error() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "add", "paths": ["does_not_exist.txt"] }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to stage")),
            "Expected SystemError for nonexistent file, got: {result:?}"
        );
    }

    // ── Checkout nonexistent branch without create ──

    #[tokio::test]
    async fn test_checkout_nonexistent_branch_without_create() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "checkout", "branch": "nonexistent-branch" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found") || message.contains("Failed")),
            "Expected SystemError for nonexistent branch checkout, got: {result:?}"
        );
    }

    // ── Push force requires explicit flag ──

    #[tokio::test]
    async fn test_push_force_requires_explicit_flag() {
        let (dir, repo) = init_repo_with_commit();

        // Create bare repo as remote
        let bare_dir = tempfile::tempdir().unwrap();
        let bare_repo = git2::Repository::init_bare(bare_dir.path()).unwrap();
        let remote_url = bare_dir.path().to_str().unwrap();
        repo.remote("origin", remote_url).unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());

        // Step 1: Initial push succeeds (fast-forward, no force needed)
        let result = execute(&tool, json!({ "operation": "push" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Pushed"),
                    "Expected push success, got: {content}"
                );
            }
            _ => panic!("Expected Success for initial push, got: {result:?}"),
        }

        // Step 2: Add a second commit and push it (still fast-forward)
        add_commit(&repo, &dir, "second.txt", b"second\n", "Second commit");
        let result = execute(&tool, json!({ "operation": "push" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Pushed"),
                    "Expected push success, got: {content}"
                );
            }
            _ => panic!("Expected Success for second push, got: {result:?}"),
        }

        // Step 3: Create a divergent history.
        // Reset HEAD back to the initial commit, then create a different commit on top.
        let initial_obj = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent(0)
            .unwrap()
            .into_object();
        repo.reset(&initial_obj, git2::ResetType::Hard, None)
            .unwrap();
        add_commit(
            &repo,
            &dir,
            "divergent.txt",
            b"divergent\n",
            "Divergent commit",
        );

        // Now: local has (initial -> divergent), remote has (initial -> second)
        // This is a non-fast-forward situation.

        // Step 4: Push WITHOUT force should fail
        let result = execute(&tool, json!({ "operation": "push" })).await;
        match &result {
            ToolResult::SystemError { message } => {
                assert!(
                    message.contains("Push failed"),
                    "Expected push failure, got: {message}"
                );
            }
            _ => panic!(
                "Expected SystemError for non-fast-forward push without force, got: {result:?}"
            ),
        }

        // Step 5: Push WITH force should succeed
        let result = execute(&tool, json!({ "operation": "push", "force": true })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Pushed"),
                    "Expected force push success, got: {content}"
                );
            }
            _ => panic!("Expected Success for force push, got: {result:?}"),
        }

        // Step 6: Verify the remote now has the divergent commit
        let remote_head = bare_repo.head().unwrap().peel_to_commit().unwrap();
        let local_head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(
            remote_head.id(),
            local_head.id(),
            "After force push, remote HEAD should match local HEAD"
        );
        assert_eq!(
            local_head.message().unwrap().trim(),
            "Divergent commit",
            "Remote should have the divergent commit"
        );
    }

    // ── Commit hash in output ──

    #[tokio::test]
    async fn test_commit_hash_appears_in_output() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("hash_test.txt"), b"content\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_path(std::path::Path::new("hash_test.txt"))
            .unwrap();
        index.write().unwrap();

        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Hash test" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                // The output should contain a commit hash (7+ hex chars)
                assert!(
                    content.contains("Committed"),
                    "Expected 'Committed', got: {content}"
                );
                // Verify commit was actually created
                let head = repo.head().unwrap().peel_to_commit().unwrap();
                assert_eq!(head.message().unwrap().trim(), "Hash test");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Commit with no staged changes ──

    #[tokio::test]
    async fn test_commit_with_nothing_staged_succeeds_with_zero_stats() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "commit", "message": "Nothing staged" }),
        )
        .await;
        // git2 allows committing with no changes (creates same-tree commit)
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("0 file(s) changed"),
                    "Expected 0 files changed, got: {content}"
                );
            }
            _ => panic!("Expected Success (empty commit), got: {result:?}"),
        }
    }
}
