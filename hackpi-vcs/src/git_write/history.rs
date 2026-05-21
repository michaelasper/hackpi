use hackpi_core::tools::{ToolContext, ToolResult};
use serde_json::Value;

use super::GitWriteTool;

pub(super) fn cmd_commit(repo: &git2::Repository, params: &Value) -> ToolResult {
    let message = match params.get("message").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => {
            return ToolResult::SystemError {
                message: "Missing 'message' parameter for commit.".into(),
            }
        }
    };

    let allow_empty = params
        .get("allow_empty")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

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

    // Reject same-tree (empty) commits unless allow_empty is explicitly true
    if !allow_empty {
        if let Some(ref parent) = head_commit {
            if let Ok(parent_tree) = parent.tree() {
                if parent_tree.id() == tree_oid {
                    return ToolResult::SystemError {
                        message: "No changes to commit: the index matches HEAD. Use `allow_empty: true` to force an empty commit.".into(),
                    };
                }
            }
        }
    }

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

pub(super) fn cmd_merge(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
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

pub(super) fn cmd_rebase(repo: &git2::Repository, params: &Value, ctx: &ToolContext) -> ToolResult {
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

pub(super) fn cmd_reset(repo: &git2::Repository, params: &Value) -> ToolResult {
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
pub(super) fn get_conflict_files(repo: &git2::Repository) -> Vec<String> {
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
