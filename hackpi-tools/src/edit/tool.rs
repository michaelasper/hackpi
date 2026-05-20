use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::fmt::Write;
use tokio::fs;

/// Try to parse `pos` as a range anchor in the format `start#HASH-end#HASH`.
/// Returns `Some((start_idx, end_idx))` where end_idx is exclusive (1 past the end).
/// Returns `None` if the string doesn't match the range anchor format.
fn resolve_range_anchor(pos: &str, lines: &[String]) -> Option<(usize, usize)> {
    // Match pattern: digits#XX-digits#XX where XX are 2-char hashes
    let (left, right) = pos.split_once('-')?;
    let start_idx = super::anchor::resolve_anchor(left, lines)?;
    let end_idx = super::anchor::resolve_anchor(right, lines)?;
    if start_idx > end_idx {
        return None;
    }
    Some((start_idx, end_idx + 1))
}

use super::anchor::{
    contains_patch_markers, generate_anchor_hint, make_updated_anchors, resolve_anchor,
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
                            },
                            "lines": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "For replace_text: replacement lines array (alternative to newText)."
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

        let canonical =
            match crate::path_jail::resolve_workspace_path(&self.workspace_root, file_path) {
                Ok(p) => p,
                Err(e) => return e,
            };

        let original = match fs::read_to_string(&canonical).await {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Error reading {file_path}: {e}"),
                }
            }
        };

        let lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();
        let had_trailing_newline = original.ends_with('\n');
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
                        if resolve_anchor(end_pos, &lines).is_none() {
                            let hint = generate_anchor_hint(&lines, pos);
                            return ToolResult::SystemError {
                                message: format!(
                                    "[E_STALE_ANCHOR] End anchor '{end_pos}' not found in {file_path}. \
                                     The file may have changed since you read it.\n{hint}"
                                ),
                            };
                        }
                    }
                    if resolve_range_anchor(pos, &lines).is_none()
                        && resolve_anchor(pos, &lines).is_none()
                    {
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
                EditOp::ReplaceText {
                    old_text,
                    lines: replacement_lines,
                    ..
                } => {
                    if let Some(ref rep_lines) = replacement_lines {
                        if contains_patch_markers(rep_lines) {
                            return ToolResult::SystemError {
                                message: format!(
                                    "[E_INVALID_PATCH] Edit rejected: `lines` contains LINE#HASH: prefixes or +/- markers. \
                                     Send plain file content only. File: {file_path}"
                                ),
                            };
                        }
                    }
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
                    let (start_lineno, end_idx) = if let Some((s, e)) =
                        resolve_range_anchor(pos, &lines)
                    {
                        (s, e)
                    } else {
                        let s = resolve_anchor(pos, &lines).unwrap();
                        let e = if let Some(end_pos) = end {
                            match resolve_anchor(end_pos, &lines) {
                                Some(ee) => ee + 1,
                                None => {
                                    return ToolResult::SystemError {
                                        message: format!(
                                            "[E_STALE_ANCHOR] End anchor '{end_pos}' not found in {file_path}. \
                                             The file may have changed since you read it."
                                        ),
                                    }
                                }
                            }
                        } else {
                            s + 1
                        };
                        (s, e)
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
                EditOp::ReplaceText {
                    old_text,
                    new_text,
                    lines,
                } => {
                    let content = current_lines.join("\n");
                    let replacement = match lines {
                        Some(l) => l.join("\n"),
                        None => new_text.clone(),
                    };
                    let new_content = content.replace(old_text, &replacement);
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

        let mut result = current_lines.join("\n");
        if had_trailing_newline {
            result.push('\n');
        }

        let original_perms = std::fs::metadata(&canonical).ok().map(|m| m.permissions());

        // Use tempfile::NamedTempFile with randomized filenames opened with
        // create_new(true) (O_CREAT|O_EXCL) so that pre-existing symlinks at
        // the temp path CANNOT be followed — the OS atomically refuses to
        // open an already-existing path. This prevents the attack where a
        // malicious workspace places a symlink at a predictable temp path
        // (e.g. .<file>.tmp) pointing outside the workspace (COR-168).
        let tmp_dir = canonical.parent().unwrap_or(std::path::Path::new("."));
        let tmp = match tempfile::NamedTempFile::new_in(tmp_dir) {
            Ok(t) => t,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("IO error creating temp file: {e}"),
                }
            }
        };
        let tmp_path = tmp.path().to_path_buf();

        if let Err(e) = fs::write(&tmp_path, result.as_bytes()).await {
            // tmp is dropped here, which cleans up the temp file
            return ToolResult::SystemError {
                message: format!("IO error writing {file_path}: {e}"),
            };
        }

        if let Some(perms) = &original_perms {
            let _ = fs::set_permissions(&tmp_path, perms.clone()).await;
        }

        // persist() atomically renames the temp file to the target path.
        // On failure, the NamedTempFile is consumed and cleaned up on drop.
        if let Err(e) = tmp.persist(&canonical) {
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
                let old_count = ae.old_snippet.len();
                let new_count = ae.new_snippet.len();
                let old_start = ae.start_line + 1;
                let new_start = ae.start_line + 1;
                writeln!(
                    diff,
                    "@@ -{},{} +{},{} @@",
                    old_start, old_count, new_start, new_count
                )
                .ok();
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
    use std::sync::OnceLock;

    fn temp_dir() -> std::path::PathBuf {
        static COUNTER: OnceLock<std::sync::atomic::AtomicU32> = OnceLock::new();
        let c = COUNTER.get_or_init(|| std::sync::atomic::AtomicU32::new(0));
        let id = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hackpi_edit_test_{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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

    #[tokio::test]
    async fn test_edit_absolute_path_is_rejected() {
        let dir = temp_dir();
        std::fs::write(dir.join("test.txt"), b"hello").unwrap();

        let tool = EditTool::new(dir.clone());
        let params = serde_json::json!({
            "path": "/etc/passwd",
            "edits": [{"op": "append", "lines": ["extra"]}]
        });
        let ctx = ToolContext {
            workspace_root: dir.clone(),
            signal: tokio::sync::watch::channel(false).1,
        };

        let result = tool.execute(params, &ctx).await;

        match result {
            ToolResult::SystemError { message } => {
                assert!(
                    message.contains("Absolute path") || message.contains("outside workspace"),
                    "expected security error, got: {message}"
                );
            }
            other => panic!("expected SystemError for absolute path, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_edit_path_traversal_outside_workspace_is_rejected() {
        let dir = temp_dir();
        std::fs::write(dir.join("test.txt"), b"hello").unwrap();

        let tool = EditTool::new(dir.clone());
        let params = serde_json::json!({
            "path": "../outside.txt",
            "edits": [{"op": "append", "lines": ["extra"]}]
        });
        let ctx = ToolContext {
            workspace_root: dir.clone(),
            signal: tokio::sync::watch::channel(false).1,
        };

        let result = tool.execute(params, &ctx).await;

        match result {
            ToolResult::SystemError { message } => {
                assert!(
                    message.contains("outside workspace"),
                    "expected security error, got: {message}"
                );
            }
            other => panic!("expected SystemError for path traversal, got {other:?}"),
        }
    }
}
