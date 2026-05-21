use hackpi_core::tools::ToolResult;

/// Convert git2 status flags to a porcelain-format XY character pair.
///
/// Matches `git status --porcelain` output:
/// - `??` for untracked files, `!!` for ignored files
/// - Index status in first column, working tree status in second
pub(crate) fn status_xy(status: git2::Status) -> (char, char) {
    use git2::Status;

    // Untracked: no index changes, WT_NEW without IGNORED
    if status.contains(Status::WT_NEW) && !status.contains(Status::IGNORED) {
        // Check if there are any index flags too — pure untracked has no index flags
        let has_index = status.intersects(
            Status::INDEX_NEW
                | Status::INDEX_MODIFIED
                | Status::INDEX_DELETED
                | Status::INDEX_RENAMED
                | Status::INDEX_TYPECHANGE,
        );
        if !has_index {
            return ('?', '?');
        }
    }

    // Ignored: both columns are '!'
    if status.contains(Status::IGNORED) {
        return ('!', '!');
    }

    let x = if status.contains(Status::INDEX_NEW) {
        'A'
    } else if status.contains(Status::INDEX_MODIFIED) {
        'M'
    } else if status.contains(Status::INDEX_DELETED) {
        'D'
    } else if status.contains(Status::INDEX_RENAMED) {
        'R'
    } else if status.contains(Status::INDEX_TYPECHANGE) {
        'T'
    } else {
        ' '
    };

    let y = if status.contains(Status::WT_MODIFIED) {
        'M'
    } else if status.contains(Status::WT_DELETED) {
        'D'
    } else if status.contains(Status::WT_RENAMED) {
        'R'
    } else if status.contains(Status::WT_TYPECHANGE) {
        'T'
    } else if status.contains(Status::WT_NEW) {
        // WT_NEW with index flags (e.g., file was staged then modified)
        '?'
    } else {
        ' '
    };

    (x, y)
}

pub(crate) fn cmd_status(repo: &git2::Repository) -> ToolResult {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .include_ignored(true)
        .recurse_untracked_dirs(true);

    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to get status: {e}"),
            }
        }
    };

    let mut output = String::new();
    for entry in statuses.iter() {
        let status = entry.status();
        let (x, y) = status_xy(status);
        let path = entry.path().unwrap_or("<unknown>");
        use std::fmt::Write;
        writeln!(output, "{x}{y} {path}").ok();
    }

    ToolResult::Success { content: output }
}

pub(crate) fn cmd_branch_list(repo: &git2::Repository) -> ToolResult {
    let head = repo.head().ok();

    let mut output = String::new();

    // Local branches
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for branch_result in branches {
            let (branch, _type) = match branch_result {
                Ok(b) => b,
                Err(_) => continue,
            };
            let name = match branch.name() {
                Ok(Some(n)) => n.to_string(),
                _ => continue,
            };
            let marker = if let Some(ref head_ref) = head {
                let head_name = head_ref.shorthand().unwrap_or("");
                if head_name == name.as_str() {
                    '*'
                } else {
                    ' '
                }
            } else {
                ' '
            };
            use std::fmt::Write;
            writeln!(output, "{marker} {name}").ok();
        }
    }

    // Remote branches
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Remote)) {
        for branch_result in branches {
            let (branch, _type) = match branch_result {
                Ok(b) => b,
                Err(_) => continue,
            };
            let name = match branch.name() {
                Ok(Some(n)) => n.to_string(),
                _ => continue,
            };
            // Skip the HEAD pointer remote
            if name.contains("/HEAD") {
                continue;
            }
            use std::fmt::Write;
            writeln!(output, "  {name}").ok();
        }
    }

    ToolResult::Success { content: output }
}

pub(crate) fn cmd_remote_list(repo: &git2::Repository) -> ToolResult {
    let remotes = match repo.remotes() {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to list remotes: {e}"),
            }
        }
    };

    let mut output = String::new();
    for name in remotes.iter().flatten() {
        let url = repo
            .find_remote(name)
            .ok()
            .and_then(|r| r.url().map(|u| u.to_string()))
            .unwrap_or_default();
        use std::fmt::Write;
        writeln!(output, "{name}\t{url}").ok();
    }

    ToolResult::Success { content: output }
}
