use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
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

pub struct EditTool {
    workspace_root: std::path::PathBuf,
}

impl EditTool {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit an existing file using LINE#HASH anchors from read output. \
         Supports replace, append, prepend, and replace_text operations."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filePath": {
                    "type": "string",
                    "description": "Path to the file to edit."
                },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "enum": ["replace", "append", "prepend", "replace_text"],
                                "description": "Edit operation type."
                            },
                            "pos": {
                                "type": "string",
                                "description": "LINE#HASH anchor for the target line."
                            },
                            "old_string": {
                                "type": "string",
                                "description": "For replace_text: the exact text to replace."
                            },
                            "new_string": {
                                "type": "string",
                                "description": "The new text to insert or replacement text."
                            }
                        },
                        "required": ["op", "new_string"]
                    },
                    "description": "List of edit operations to apply sequentially."
                }
            },
            "required": ["filePath", "edits"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match params.get("filePath").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'filePath' parameter.".into(),
                }
            }
        };

        let edits = match params.get("edits").and_then(|v| v.as_array()) {
            Some(e) => e,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'edits' parameter.".into(),
                }
            }
        };

        let path = self.workspace_root.join(file_path);

        let original = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Error reading {file_path}: {e}"),
                }
            }
        };

        let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
        let mut total_changes = 0u32;

        for edit in edits {
            let op = edit.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let new_string = edit.get("new_string").and_then(|v| v.as_str()).unwrap_or("");

            match op {
                "replace" => {
                    let pos = edit.get("pos").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(lineno) = resolve_anchor(pos, &lines) {
                        lines[lineno] = new_string.to_string();
                        total_changes += 1;
                    } else {
                        return ToolResult::SystemError {
                            message: format!("Anchor '{pos}' not found in {file_path}. The file may have changed since you read it. Please re-read the file and retry."),
                        };
                    }
                }
                "append" => {
                    let pos = edit.get("pos").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(lineno) = resolve_anchor(pos, &lines) {
                        if lineno + 1 >= lines.len() {
                            lines.push(new_string.to_string());
                        } else {
                            lines.insert(lineno + 1, new_string.to_string());
                        }
                        total_changes += 1;
                    } else {
                        return ToolResult::SystemError {
                            message: format!("Anchor '{pos}' not found in {file_path}. The file may have changed since you read it. Please re-read the file and retry."),
                        };
                    }
                }
                "prepend" => {
                    let pos = edit.get("pos").and_then(|v| v.as_str()).unwrap_or("");
                    if let Some(lineno) = resolve_anchor(pos, &lines) {
                        lines.insert(lineno, new_string.to_string());
                        total_changes += 1;
                    } else {
                        return ToolResult::SystemError {
                            message: format!("Anchor '{pos}' not found in {file_path}. The file may have changed since you read it. Please re-read the file and retry."),
                        };
                    }
                }
                "replace_text" => {
                    let old_string = edit.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                    let content = lines.join("\n");
                    if !content.contains(old_string) {
                        return ToolResult::SystemError {
                            message: format!("replace_text: old_string not found in {file_path}."),
                        };
                    }
                    let new_content = content.replace(old_string, new_string);
                    lines = new_content.lines().map(|l| l.to_string()).collect();
                    total_changes += 1;
                }
                _ => {
                    return ToolResult::SystemError {
                        message: format!("Unknown edit operation: '{op}'."),
                    }
                }
            }
        }

        let result = lines.join("\n");
        if let Err(e) = std::fs::write(&path, &result) {
            return ToolResult::SystemError {
                message: format!("Error writing {file_path}: {e}"),
            };
        }

        ToolResult::Success {
            content: format!("Applied {total_changes} edit(s) to {file_path}"),
        }
    }
}

fn resolve_anchor(anchor: &str, lines: &[String]) -> Option<usize> {
    let (line_str, hash) = anchor.split_once('#')?;
    let lineno: usize = line_str.parse().ok()?;
    if lineno == 0 || lineno > lines.len() {
        return None;
    }
    let actual = &lines[lineno - 1];
    let expected_hash = line_hash(actual);
    if expected_hash == hash {
        Some(lineno - 1)
    } else {
        None
    }
}
