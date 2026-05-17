use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;

const MAX_DIFF_BYTES: usize = 256 * 1024;
const DEFAULT_LOG_COUNT: u32 = 20;
const MAX_LOG_COUNT: u32 = 100;

pub struct GitReadTool {
    workspace_root: PathBuf,
}

impl GitReadTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn open_repo(&self) -> Result<git2::Repository, ToolResult> {
        git2::Repository::discover(&self.workspace_root).map_err(|_| ToolResult::SystemError {
            message: "Not a git repository (or any of the parent directories)".into(),
        })
    }
}

#[async_trait]
impl Tool for GitReadTool {
    fn name(&self) -> &str {
        "git_read"
    }

    fn description(&self) -> &str {
        "Read-only git operations: status, diff, log, branches, remotes, show"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["status", "diff", "diff_staged", "log", "branch_list", "remote_list", "show"],
                    "description": "The git read operation to perform"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of log entries (default: 20, max: 100)"
                },
                "revision": {
                    "type": "string",
                    "description": "Revision for 'show' operation (HEAD, branch name, or hash)"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let operation = match params.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'operation' parameter.".into(),
                }
            }
        };

        let repo = match self.open_repo() {
            Ok(r) => r,
            Err(e) => return e,
        };

        match operation {
            "status" => cmd_status(&repo),
            "diff" => cmd_diff(&repo),
            "diff_staged" => cmd_diff_staged(&repo),
            "log" => {
                let count = params
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(DEFAULT_LOG_COUNT)
                    .min(MAX_LOG_COUNT);
                cmd_log(&repo, count)
            }
            "branch_list" => cmd_branch_list(&repo),
            "remote_list" => cmd_remote_list(&repo),
            "show" => {
                let revision = params
                    .get("revision")
                    .and_then(|v| v.as_str())
                    .unwrap_or("HEAD");
                cmd_show(&repo, revision)
            }
            _ => ToolResult::SystemError {
                message: format!("Unknown operation: {operation}"),
            },
        }
    }
}

/// Convert git2 status flags to a porcelain-format XY character pair.
///
/// Matches `git status --porcelain` output:
/// - `??` for untracked files, `!!` for ignored files
/// - Index status in first column, working tree status in second
fn status_xy(status: git2::Status) -> (char, char) {
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

fn cmd_status(repo: &git2::Repository) -> ToolResult {
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

fn cmd_diff(repo: &git2::Repository) -> ToolResult {
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

fn cmd_diff_staged(repo: &git2::Repository) -> ToolResult {
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

fn format_diff(diff: git2::Diff) -> ToolResult {
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

fn cmd_log(repo: &git2::Repository, count: u32) -> ToolResult {
    let mut revwalk = match repo.revwalk() {
        Ok(w) => w,
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Failed to create revision walk: {e}"),
            }
        }
    };

    if revwalk.push_head().is_err() {
        // No commits yet
        return ToolResult::Success {
            content: String::new(),
        };
    }

    revwalk.set_sorting(git2::Sort::TIME).ok();

    let mut output = String::new();
    let mut n = 0u32;
    for oid_result in revwalk {
        if n >= count {
            break;
        }
        let oid = match oid_result {
            Ok(id) => id,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let short_hash = &oid.to_string()[..7.min(oid.to_string().len())];
        let author = commit.author();
        let name = author.name().unwrap_or("<unknown>");
        let time = author.when();
        let date = format_timestamp(time.seconds());
        let summary = commit.summary().unwrap_or("").to_string();

        use std::fmt::Write;
        writeln!(output, "{short_hash} {name} ({date}) {summary}").ok();
        n += 1;
    }

    ToolResult::Success { content: output }
}

fn cmd_branch_list(repo: &git2::Repository) -> ToolResult {
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

fn cmd_remote_list(repo: &git2::Repository) -> ToolResult {
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

fn cmd_show(repo: &git2::Repository, revision: &str) -> ToolResult {
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

/// Convert a Unix timestamp to YYYY-MM-DD date string.
fn format_timestamp(seconds: i64) -> String {
    // Civil date algorithm by Howard Hinnant
    let z = seconds / 86400 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
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
        // Write initial file
        let path = dir.path().join("README.md");
        std::fs::write(&path, b"# Test\n").unwrap();
        // Stage, persist index, and commit
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

    fn make_tool(workspace_root: PathBuf) -> GitReadTool {
        GitReadTool::new(workspace_root)
    }

    fn test_ctx() -> ToolContext {
        ToolContext {
            workspace_root: std::env::temp_dir(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        }
    }

    async fn execute(tool: &GitReadTool, params: Value) -> ToolResult {
        tool.execute(params, &test_ctx()).await
    }

    // ── Basic tool metadata ──

    #[tokio::test]
    async fn test_name() {
        let tool = make_tool(PathBuf::from("/tmp"));
        assert_eq!(tool.name(), "git_read");
    }

    #[tokio::test]
    async fn test_description() {
        let tool = make_tool(PathBuf::from("/tmp"));
        assert!(tool.description().contains("status"));
        assert!(tool.description().contains("diff"));
        assert!(tool.description().contains("log"));
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
        let result = execute(&tool, json!({ "operation": "status" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Not a git repository")),
            "Expected SystemError about missing repo, got: {result:?}"
        );
    }

    // ── Status ──

    #[tokio::test]
    async fn test_status_clean_repo() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "status" })).await;
        assert!(
            matches!(&result, ToolResult::Success { content } if content.is_empty()),
            "Expected empty status for clean repo, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_status_unstaged_modification() {
        let (dir, repo) = init_repo_with_commit();
        // Modify file without staging
        std::fs::write(dir.path().join("README.md"), b"# Modified\n").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "status" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains(" M README.md"),
                    "Expected ' M README.md' for unstaged modification, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_status_staged_modification() {
        let (dir, repo) = init_repo_with_commit();
        // Modify and stage
        std::fs::write(dir.path().join("README.md"), b"# Staged\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        index.write().unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "status" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("M  README.md"),
                    "Expected 'M  README.md' for staged modification, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_status_untracked_file() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("new_file.txt"), b"new\n").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "status" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("?? new_file.txt"),
                    "Expected '?? new_file.txt', got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Diff ──

    #[tokio::test]
    async fn test_diff_unstaged_changes() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("README.md"), b"# Modified\n").unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "diff" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("-"),
                    "diff should contain removed lines, got: {content}"
                );
                assert!(
                    content.contains("+"),
                    "diff should contain added lines, got: {content}"
                );
                assert!(
                    content.contains("README.md"),
                    "diff should reference README.md, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_diff_no_changes() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "diff" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.is_empty(), "Expected empty diff, got: {content}");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Diff staged ──

    #[tokio::test]
    async fn test_diff_staged_changes() {
        let (dir, repo) = init_repo_with_commit();
        std::fs::write(dir.path().join("README.md"), b"# Staged Change\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        index.write().unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "diff_staged" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(!content.is_empty(), "Expected non-empty staged diff");
                assert!(
                    content.contains("README.md"),
                    "diff_staged should reference README.md, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_diff_staged_no_changes() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "diff_staged" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.is_empty(),
                    "Expected empty staged diff, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Log ──

    #[tokio::test]
    async fn test_log_single_commit() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "log" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Initial commit"),
                    "Log should contain commit message, got: {content}"
                );
                assert!(
                    content.contains("Test User"),
                    "Log should contain author, got: {content}"
                );
                // Should have exactly 1 line
                assert_eq!(
                    content.lines().count(),
                    1,
                    "Expected 1 log entry, got: {content:?}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_log_multiple_commits() {
        let (dir, repo) = init_repo_with_commit();
        add_commit(&repo, &dir, "file2.txt", b"second", "Second commit");
        add_commit(&repo, &dir, "file3.txt", b"third", "Third commit");
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "log", "count": 2 })).await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(
                    content.lines().count(),
                    2,
                    "Expected 2 log entries, got: {content:?}"
                );
                assert!(
                    content.lines().next().unwrap().contains("Third commit"),
                    "Most recent commit should be first, got: {content:?}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_log_default_count() {
        let (dir, repo) = init_repo_with_commit();
        for i in 0..30 {
            add_commit(
                &repo,
                &dir,
                &format!("file{i}.txt"),
                b"data",
                &format!("Commit {i}"),
            );
        }
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "log" })).await;
        match &result {
            ToolResult::Success { content } => {
                // Default is 20
                assert_eq!(
                    content.lines().count(),
                    20,
                    "Expected default 20 log entries, got: {content:?}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_log_clamps_at_max() {
        let (dir, repo) = init_repo_with_commit();
        for i in 0..150 {
            add_commit(
                &repo,
                &dir,
                &format!("file{i}.txt"),
                b"data",
                &format!("Commit {i}"),
            );
        }
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "log", "count": 999 })).await;
        match &result {
            ToolResult::Success { content } => {
                // Should be clamped to 100
                assert_eq!(
                    content.lines().count(),
                    100,
                    "Expected clamped to 100 log entries, got: {}",
                    content.lines().count()
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_log_no_commits() {
        let (dir, _repo) = init_repo(); // No commits
        let tool = make_tool(dir.path().to_path_buf());
        let result = execute(&tool, json!({ "operation": "log" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.is_empty(),
                    "Expected empty log for no commits, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Branch list ──

    #[tokio::test]
    async fn test_branch_list_single_branch() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "branch_list" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("* main") || content.contains("* master"),
                    "Expected current branch marked with *, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_branch_list_multiple_branches() {
        let (_dir, repo) = init_repo_with_commit();
        // Create another branch
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature-x", &commit, false).unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "branch_list" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("feature-x"),
                    "Expected feature-x in branch list, got: {content}"
                );
                assert!(
                    content.contains("main") || content.contains("master"),
                    "Expected main branch in list, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Remote list ──

    #[tokio::test]
    async fn test_remote_list_no_remotes() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "remote_list" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.is_empty(),
                    "Expected empty remote list, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_remote_list_with_remote() {
        let (_dir, repo) = init_repo_with_commit();
        repo.remote("origin", "https://github.com/owner/repo.git")
            .unwrap();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "remote_list" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("origin"),
                    "Expected origin in remote list, got: {content}"
                );
                assert!(
                    content.contains("https://github.com/owner/repo.git"),
                    "Expected remote URL, got: {content}"
                );
                assert!(
                    content.contains("\t"),
                    "Expected tab-separated format, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Show ──

    #[tokio::test]
    async fn test_show_head() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(&tool, json!({ "operation": "show", "revision": "HEAD" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Initial commit"),
                    "Show should contain commit message, got: {content}"
                );
                assert!(
                    content.contains("commit "),
                    "Show should contain commit hash, got: {content}"
                );
                assert!(
                    content.contains("Author:"),
                    "Show should contain Author, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_show_invalid_revision() {
        let (_dir, repo) = init_repo_with_commit();
        let tool = make_tool(repo.workdir().unwrap().to_path_buf());
        let result = execute(
            &tool,
            json!({ "operation": "show", "revision": "nonexistent" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Failed to resolve revision")),
            "Expected SystemError for bad revision, got: {result:?}"
        );
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
}
