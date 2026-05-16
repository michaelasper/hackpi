use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::fmt::Write;
use tokio::fs;

use super::anchor::{
    contains_patch_markers, generate_anchor_hint, make_updated_anchors, resolve_anchor,
    resolve_anchor_range,
};
use super::hash::line_hash;
use super::ops::{deserialize_edit_ops, op_anchor_line, AppliedEdit, EditOp};

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
                "path": {
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
                            "end": {
                                "type": "string",
                                "description": "LINE#HASH anchor for the end of a range (replace only)."
                            },
                            "lines": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Replacement lines (not LINE#HASH prefixed, not diff +/- marked)."
                            },
                            "oldText": {
                                "type": "string",
                                "description": "For replace_text: the exact text to replace."
                            },
                            "newText": {
                                "type": "string",
                                "description": "For replace_text: the replacement text."
                            }
                        },
                        "required": ["op"],
                        "additionalProperties": false
                    },
                    "description": "List of edit operations. Applied bottom-up on pre-edit snapshot."
                }
            },
            "required": ["path", "edits"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'path' parameter.".into(),
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
        let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());

        let original = match fs::read_to_string(&canonical).await {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Error reading {file_path}: {e}"),
                }
            }
        };

        let lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
        let ops = match deserialize_edit_ops(edits) {
            Ok(o) => o,
            Err(e) => return ToolResult::SystemError { message: e },
        };

        for op in &ops {
            match op {
                EditOp::Replace {
                    pos,
                    end,
                    lines: new_lines,
                } => {
                    if contains_patch_markers(new_lines) {
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_INVALID_PATCH] Edit rejected: `lines` contains LINE#HASH: prefixes or +/- markers. \
                                 Send plain file content only. File: {file_path}"
                            ),
                        };
                    }
                    if let Some(end_pos) = end {
                        if resolve_anchor_range(end_pos, &lines).is_none() {
                            let hint = generate_anchor_hint(&lines, pos);
                            return ToolResult::SystemError {
                                message: format!(
                                    "[E_STALE_ANCHOR] End anchor '{end_pos}' not found in {file_path}. \
                                     The file may have changed since you read it.\n{hint}"
                                ),
                            };
                        }
                    }
                    if resolve_anchor(pos, &lines).is_none() {
                        let hint = generate_anchor_hint(&lines, pos);
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_STALE_ANCHOR] Anchor '{pos}' not found in {file_path}. \
                                 The file may have changed since you read it.\n{hint}"
                            ),
                        };
                    }
                }
                EditOp::Append {
                    pos: Some(pos),
                    lines: new_lines,
                } => {
                    if contains_patch_markers(new_lines) {
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_INVALID_PATCH] Edit rejected: `lines` contains LINE#HASH: prefixes or +/- markers. \
                                 Send plain file content only. File: {file_path}"
                            ),
                        };
                    }
                    if resolve_anchor(pos, &lines).is_none() {
                        let hint = generate_anchor_hint(&lines, pos);
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_STALE_ANCHOR] Anchor '{pos}' not found in {file_path}. \
                                 The file may have changed since you read it.\n{hint}"
                            ),
                        };
                    }
                }
                EditOp::Append {
                    pos: None,
                    lines: new_lines,
                } => {
                    if contains_patch_markers(new_lines) {
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_INVALID_PATCH] Edit rejected: `lines` contains LINE#HASH: prefixes or +/- markers. \
                                 Send plain file content only. File: {file_path}"
                            ),
                        };
                    }
                }
                EditOp::Prepend {
                    pos: Some(pos),
                    lines: new_lines,
                } => {
                    if contains_patch_markers(new_lines) {
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_INVALID_PATCH] Edit rejected: `lines` contains LINE#HASH: prefixes or +/- markers. \
                                 Send plain file content only. File: {file_path}"
                            ),
                        };
                    }
                    if resolve_anchor(pos, &lines).is_none() {
                        let hint = generate_anchor_hint(&lines, pos);
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_STALE_ANCHOR] Anchor '{pos}' not found in {file_path}. \
                                 The file may have changed since you read it.\n{hint}"
                            ),
                        };
                    }
                }
                EditOp::Prepend {
                    pos: None,
                    lines: new_lines,
                } => {
                    if contains_patch_markers(new_lines) {
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_INVALID_PATCH] Edit rejected: `lines` contains LINE#HASH: prefixes or +/- markers. \
                                 Send plain file content only. File: {file_path}"
                            ),
                        };
                    }
                }
                EditOp::ReplaceText { old_text, .. } => {
                    if old_text.is_empty() {
                        return ToolResult::SystemError {
                            message: "replace_text: oldText must not be empty.".into(),
                        };
                    }
                    let haystack = lines.join("\n");
                    let count = haystack.matches(old_text.as_str()).count();
                    if count == 0 {
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_TEXT_NOT_FOUND] replace_text: '{old_text}' not found in {file_path}."
                            ),
                        };
                    }
                    if count > 1 {
                        return ToolResult::SystemError {
                            message: format!(
                                "[E_TEXT_NOT_UNIQUE] replace_text: '{old_text}' matches {count} times in {file_path}. \
                                 Use a more specific string to match exactly one occurrence."
                            ),
                        };
                    }
                }
            }
        }

        let mut indexed_ops: Vec<(usize, &EditOp)> = ops.iter().enumerate().collect();
        indexed_ops.sort_by(|a, b| {
            let line_a = op_anchor_line(a.1, &lines).unwrap_or(usize::MAX);
            let line_b = op_anchor_line(b.1, &lines).unwrap_or(usize::MAX);
            line_b.cmp(&line_a)
        });

        let mut current_lines = lines.clone();
        let mut applied_edits: Vec<AppliedEdit> = Vec::new();

        for (_, op) in &indexed_ops {
            match op {
                EditOp::Replace {
                    pos,
                    end,
                    lines: new_lines,
                } => {
                    let start_lineno = resolve_anchor(pos, &lines).unwrap();
                    let end_idx = if let Some(end_pos) = end {
                        resolve_anchor(end_pos, &lines)
                            .map(|e| e + 1)
                            .unwrap_or(start_lineno + 1)
                    } else {
                        start_lineno + 1
                    };
                    let old_snippet: Vec<String> = lines[start_lineno..end_idx].to_vec();
                    let old_start = start_lineno;

                    let actual_lineno = start_lineno;
                    current_lines.splice(
                        actual_lineno..actual_lineno + (end_idx - start_lineno),
                        new_lines.iter().cloned(),
                    );
                    let new_end = actual_lineno + new_lines.len();
                    let new_snippet = current_lines[actual_lineno..new_end].to_vec();

                    applied_edits.push(AppliedEdit {
                        anchor_text: pos.clone(),
                        old_snippet,
                        new_snippet,
                        start_line: old_start,
                        end_line: new_end,
                    });
                }
                EditOp::Append {
                    pos,
                    lines: new_lines,
                } => {
                    let insert_at = match pos {
                        Some(p) => resolve_anchor(p, &lines).unwrap() + 1,
                        None => current_lines.len(),
                    };
                    let old_snippet: Vec<String> =
                        if insert_at > 0 && insert_at <= current_lines.len() {
                            vec![current_lines[insert_at - 1].clone()]
                        } else {
                            Vec::new()
                        };

                    current_lines.splice(insert_at..insert_at, new_lines.iter().cloned());
                    let new_end = insert_at + new_lines.len();

                    applied_edits.push(AppliedEdit {
                        anchor_text: pos.clone().unwrap_or_default(),
                        old_snippet,
                        new_snippet: current_lines[insert_at..new_end].to_vec(),
                        start_line: insert_at.saturating_sub(1),
                        end_line: new_end,
                    });
                }
                EditOp::Prepend {
                    pos,
                    lines: new_lines,
                } => {
                    let insert_at = match pos {
                        Some(p) => resolve_anchor(p, &lines).unwrap(),
                        None => 0,
                    };
                    let old_snippet: Vec<String> = if insert_at < current_lines.len() {
                        vec![current_lines[insert_at].clone()]
                    } else {
                        Vec::new()
                    };

                    current_lines.splice(insert_at..insert_at, new_lines.iter().cloned());
                    let new_end = insert_at + new_lines.len();

                    applied_edits.push(AppliedEdit {
                        anchor_text: pos.clone().unwrap_or_default(),
                        old_snippet,
                        new_snippet: current_lines[insert_at..new_end].to_vec(),
                        start_line: insert_at,
                        end_line: new_end,
                    });
                }
                EditOp::ReplaceText { old_text, new_text } => {
                    let content = current_lines.join("\n");
                    let new_content = content.replace(old_text, new_text);
                    let new_lines_vec: Vec<String> =
                        new_content.lines().map(|l| l.to_string()).collect();

                    applied_edits.push(AppliedEdit {
                        anchor_text: String::new(),
                        old_snippet: current_lines.clone(),
                        new_snippet: new_lines_vec.clone(),
                        start_line: 0,
                        end_line: new_lines_vec.len(),
                    });

                    current_lines = new_lines_vec;
                }
            }
        }

        let result = current_lines.join("\n");

        let file_name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let tmp_path = canonical.with_file_name(format!(".{file_name}.tmp"));

        let original_perms = std::fs::metadata(&canonical).ok().map(|m| m.permissions());

        if let Err(e) = fs::write(&tmp_path, result.as_bytes()).await {
            return ToolResult::SystemError {
                message: format!("IO error writing {file_path}: {e}"),
            };
        }

        if let Some(perms) = &original_perms {
            let _ = fs::set_permissions(&tmp_path, perms.clone()).await;
        }

        if let Err(e) = fs::rename(&tmp_path, &canonical).await {
            let _ = fs::remove_file(&tmp_path).await;
            return ToolResult::SystemError {
                message: format!("IO error renaming {file_path}: {e}"),
            };
        }

        let mut output = String::new();
        writeln!(
            output,
            "Applied {} edit(s) to {file_path}",
            applied_edits.len()
        )
        .ok();

        for ae in &applied_edits {
            if !ae.old_snippet.is_empty() || !ae.new_snippet.is_empty() {
                let mut diff = String::new();
                writeln!(diff, "Diff preview:").ok();
                for line in &ae.old_snippet {
                    writeln!(diff, "- {line}").ok();
                }
                for line in &ae.new_snippet {
                    writeln!(diff, "+ {line}  #{}", line_hash(line, 0)).ok();
                }
                output.push_str(&diff);
            }
            if !ae.anchor_text.is_empty() && !ae.new_snippet.is_empty() {
                output.push_str(&make_updated_anchors(
                    &current_lines,
                    ae.start_line,
                    ae.end_line,
                ));
            }
        }

        ToolResult::Success { content: output }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_schema_has_additional_properties_false() {
        let tool = EditTool::new(std::path::PathBuf::from("/tmp"));
        let schema = tool.input_schema();
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&serde_json::json!(false)),
            "edit tool top-level schema missing additionalProperties: false"
        );
        let items = &schema["properties"]["edits"]["items"];
        assert_eq!(
            items.get("additionalProperties"),
            Some(&serde_json::json!(false)),
            "edit tool items schema missing additionalProperties: false"
        );
    }
}
