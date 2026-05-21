use hackpi_core::tools::ToolResult;

use super::format_timestamp;

pub(crate) fn cmd_log(repo: &git2::Repository, count: u32) -> ToolResult {
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
