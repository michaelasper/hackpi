use hackpi_core::tools::{ToolContext, ToolResult};
use serde_json::Value;

use super::current_branch_name;
use super::history::get_conflict_files;
use super::GitWriteTool;

pub(super) fn cmd_push(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
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

pub(super) fn cmd_pull(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
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

pub(super) fn cmd_fetch(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
    let remote_name = params
        .get("remote")
        .and_then(|v| v.as_str())
        .unwrap_or("origin");

    if GitWriteTool::is_cancelled(ctx) {
        return ToolResult::Cancelled;
    }

    cmd_fetch_internal(repo, remote_name)
}
