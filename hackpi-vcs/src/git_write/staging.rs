use hackpi_core::tools::ToolResult;
use serde_json::Value;

pub(super) fn cmd_add(repo: &git2::Repository, params: &Value) -> ToolResult {
    let all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let paths = params.get("paths").and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
    });

    // Reject no-op add: must have `all: true` or non-empty `paths`
    if !all {
        let is_empty = match &paths {
            None => true,
            Some(p) => p.is_empty(),
        };
        if is_empty {
            return ToolResult::SystemError {
                message: "No files to stage: provide `all: true` or non-empty `paths`.".into(),
            };
        }
    }

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

pub(super) fn cmd_stash(repo: &mut git2::Repository, params: &Value) -> ToolResult {
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

pub(super) fn cmd_stash_pop(repo: &mut git2::Repository, params: &Value) -> ToolResult {
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
