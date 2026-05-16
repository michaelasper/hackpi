use serde_json::Value;

pub(crate) enum EditOp {
    Replace {
        pos: String,
        end: Option<String>,
        lines: Vec<String>,
    },
    Append {
        pos: Option<String>,
        lines: Vec<String>,
    },
    Prepend {
        pos: Option<String>,
        lines: Vec<String>,
    },
    ReplaceText {
        old_text: String,
        new_text: String,
    },
}

pub(crate) struct AppliedEdit {
    pub anchor_text: String,
    pub old_snippet: Vec<String>,
    pub new_snippet: Vec<String>,
    pub start_line: usize,
    pub end_line: usize,
}

pub(crate) fn deserialize_edit_ops(edits: &[Value]) -> Result<Vec<EditOp>, String> {
    let mut ops = Vec::new();
    for edit in edits {
        let op = edit.get("op").and_then(|v| v.as_str()).unwrap_or("");
        match op {
            "replace" => {
                let pos = edit
                    .get("pos")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let lines = edit
                    .get("lines")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let end = edit
                    .get("end")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                ops.push(EditOp::Replace { pos, end, lines });
            }
            "append" => {
                let pos = edit
                    .get("pos")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let lines = edit
                    .get("lines")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                ops.push(EditOp::Append { pos, lines });
            }
            "prepend" => {
                let pos = edit
                    .get("pos")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let lines = edit
                    .get("lines")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                ops.push(EditOp::Prepend { pos, lines });
            }
            "replace_text" => {
                let old_text = edit
                    .get("oldText")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let new_text = edit
                    .get("newText")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                ops.push(EditOp::ReplaceText { old_text, new_text });
            }
            _ => return Err(format!("Unknown edit operation: '{op}'.")),
        }
    }
    Ok(ops)
}

pub(crate) fn op_anchor_line(op: &EditOp, lines: &[String]) -> Option<usize> {
    match op {
        EditOp::Replace { pos, .. } => super::anchor::resolve_anchor(pos, lines),
        EditOp::Append { pos: Some(p), .. } => super::anchor::resolve_anchor(p, lines),
        EditOp::Append { pos: None, .. } => Some(lines.len()),
        EditOp::Prepend { pos: Some(p), .. } => super::anchor::resolve_anchor(p, lines),
        EditOp::Prepend { pos: None, .. } => Some(0),
        EditOp::ReplaceText { .. } => Some(0),
    }
}
