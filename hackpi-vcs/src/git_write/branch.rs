use hackpi_core::tools::ToolResult;
use serde_json::Value;

pub(super) fn cmd_checkout(repo: &git2::Repository, params: &Value) -> ToolResult {
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

pub(super) fn cmd_branch_create(repo: &git2::Repository, params: &Value) -> ToolResult {
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

pub(super) fn cmd_branch_delete(repo: &git2::Repository, params: &Value) -> ToolResult {
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
