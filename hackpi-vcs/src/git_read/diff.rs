use hackpi_core::tools::ToolResult;

use super::format_timestamp;
use super::MAX_DIFF_BYTES;

pub(crate) fn cmd_diff(repo: &git2::Repository) -> ToolResult {
    let tree = match repo.head().ok().and_then(|h| h.peel_to_tree().ok()) {
        Some(t) => t,
        None => {
            // No commits yet — diff against empty tree
            return ToolResult::Success {
                content: String::new(),
            };
        }
    };

    let diff = match repo.diff_tree_to_workdir(Some(&tree), None) {
        Ok(d) => d,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to compute diff: {e}"),
            }
        }
    };

    format_diff(diff)
}

pub(crate) fn cmd_diff_staged(repo: &git2::Repository) -> ToolResult {
    let tree = match repo.head().ok().and_then(|h| h.peel_to_tree().ok()) {
        Some(t) => t,
        None => {
            return ToolResult::Success {
                content: String::new(),
            };
        }
    };

    let diff = match repo.diff_tree_to_index(Some(&tree), None, None) {
        Ok(d) => d,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to compute staged diff: {e}"),
            }
        }
    };

    format_diff(diff)
}

pub(crate) fn format_diff(diff: git2::Diff) -> ToolResult {
    let mut output: Vec<u8> = Vec::new();
    if diff
        .print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let content = line.content();
            output.extend_from_slice(content);
            true
        })
        .is_err()
    {
        return ToolResult::SystemError {
            message: "Failed to format diff output.".into(),
        };
    }

    let content = String::from_utf8_lossy(&output).to_string();

    if content.len() > MAX_DIFF_BYTES {
        let truncated: String = content.chars().take(MAX_DIFF_BYTES).collect();
        ToolResult::Success {
            content: format!(
                "{truncated}\n\n[Diff truncated at {MAX_DIFF_BYTES} bytes. Use a more specific query to narrow results.]"
            ),
        }
    } else {
        ToolResult::Success { content }
    }
}

pub(crate) fn cmd_show(repo: &git2::Repository, revision: &str) -> ToolResult {
    let obj = match repo.revparse_single(revision) {
        Ok(o) => o,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to resolve revision '{revision}': {e}"),
            }
        }
    };

    let commit = match obj.peel_to_commit() {
        Ok(c) => c,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("'{revision}' is not a commit: {e}"),
            }
        }
    };

    let oid = commit.id();
    let author = commit.author();
    let committer = commit.committer();
    let message = commit.message().unwrap_or("");
    let time = commit.time();
    let date = format_timestamp(time.seconds());

    let mut output = String::new();
    use std::fmt::Write;

    writeln!(output, "commit {oid}").ok();
    writeln!(
        output,
        "Author: {} <{}>",
        author.name().unwrap_or(""),
        author.email().unwrap_or("")
    )
    .ok();
    writeln!(
        output,
        "Committer: {} <{}>",
        committer.name().unwrap_or(""),
        committer.email().unwrap_or("")
    )
    .ok();
    writeln!(output, "Date:   {date}").ok();
    writeln!(output).ok();
    writeln!(output, "    {}", message.trim()).ok();
    writeln!(output).ok();

    // Get the diff for this commit
    let commit_tree = match commit.tree() {
        Ok(t) => t,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to get commit tree: {e}"),
            }
        }
    };

    let parent_tree = commit.parents().next().and_then(|p| p.tree().ok());

    let diff = match repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), None) {
        Ok(d) => d,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to compute commit diff: {e}"),
            }
        }
    };

    let mut diff_output: Vec<u8> = Vec::new();
    if diff
        .print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let content = line.content();
            diff_output.extend_from_slice(content);
            true
        })
        .is_err()
    {
        return ToolResult::SystemError {
            message: "Failed to format diff output.".into(),
        };
    }

    let diff_str = String::from_utf8_lossy(&diff_output);

    if output.len() + diff_str.len() > MAX_DIFF_BYTES {
        let remaining = MAX_DIFF_BYTES.saturating_sub(output.len());
        let truncated: String = diff_str.chars().take(remaining).collect();
        write!(output, "{truncated}").ok();
        writeln!(
            output,
            "\n[Diff truncated at {MAX_DIFF_BYTES} bytes total.]"
        )
        .ok();
    } else {
        write!(output, "{diff_str}").ok();
    }

    ToolResult::Success { content: output }
}
