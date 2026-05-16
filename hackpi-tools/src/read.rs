use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::path::Path;
use xxhash_rust::xxh32::xxh32;

const HASH_CHARS: &[u8; 16] = b"ZPMQVRWSNKTXJBYH";

fn line_hash(line: &str) -> String {
    let trimmed = line.trim();
    let seed = if trimmed.chars().all(|c| !c.is_alphanumeric()) {
        line.len() as u32
    } else {
        0
    };
    let hash = xxh32(trimmed.as_bytes(), seed);
    let a = HASH_CHARS[(hash >> 4 & 0xF) as usize] as char;
    let b = HASH_CHARS[(hash & 0xF) as usize] as char;
    format!("{a}{b}")
}

const MAX_LINES: usize = 1000;
const INITIAL_DISPLAY: usize = 200;

pub struct ReadTool {
    workspace_root: std::path::PathBuf,
}

impl ReadTool {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read a file or directory. Returns file contents with LINE#HASH: prefixes for editing."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file or directory to read."
                },
                "offset": {
                    "type": "integer",
                    "description": "Start reading from this line number (1-indexed). Default: 1."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to return. Default: all lines."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'filePath' parameter.".into(),
                }
            }
        };

        let path = self.workspace_root.join(file_path);

        if !path.exists() {
            return ToolResult::SystemError {
                message: format!("Path does not exist: {file_path}"),
            };
        }

        if path.is_dir() {
            return read_directory(&path, file_path);
        }

        let is_image = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp")
        );

        if is_image {
            return ToolResult::Success {
                content: format!("[Image: {}] Passed through as attachment.\n", file_path),
            };
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Error reading {file_path}: {e}"),
                }
            }
        };

        let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        if total_lines == 0 {
            return ToolResult::Success {
                content: "[Empty file. Use prepend to add content at the beginning or append to add at the end.]".into(),
            };
        }

        let start = (offset - 1).min(total_lines);
        let end = match limit {
            Some(l) => (start + l).min(total_lines),
            None => total_lines,
        };

        let display_lines = &lines[start..end];

        let mut output = String::new();
        let line_num_width = total_lines.to_string().len();

        if total_lines > MAX_LINES && offset == 1 && limit.is_none() {
            let shown = INITIAL_DISPLAY.min(total_lines);
            let truncated_lines = &lines[..shown];
            for (i, line) in truncated_lines.iter().enumerate() {
                let lnum = i + 1;
                let hash = line_hash(line);
                writeln!(
                    output,
                    "{:>width$}#{hash}:{line}",
                    lnum,
                    width = line_num_width
                )
                .ok();
            }
            output.push_str(&format!(
                "... [truncated: {total_lines} total lines, showing {shown}] ..."
            ));
        } else {
            for (i, line) in display_lines.iter().enumerate() {
                let lnum = start + i + 1;
                let hash = line_hash(line);
                writeln!(
                    output,
                    "{:>width$}#{hash}:{line}",
                    lnum,
                    width = line_num_width
                )
                .ok();
            }
        }

        ToolResult::Success { content: output }
    }
}

use std::fmt::Write;

fn read_directory(path: &Path, display_path: &str) -> ToolResult {
    let mut entries: Vec<_> = match std::fs::read_dir(path) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| {
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let name = e.file_name().to_string_lossy().to_string();
                (name, is_dir)
            })
            .collect(),
        Err(e) => {
            return ToolResult::SystemError {
                message: format!("Error reading {display_path}: {e}"),
            }
        }
    };

    entries.sort_by(|a, b| {
        if a.1 != b.1 {
            b.1.cmp(&a.1)
        } else {
            a.0.cmp(&b.0)
        }
    });

    let mut output = String::new();
    for (name, is_dir) in &entries {
        let prefix = if *is_dir { "dir   " } else { "file  " };
        writeln!(output, "{prefix}{name}").ok();
    }

    ToolResult::Success { content: output }
}
