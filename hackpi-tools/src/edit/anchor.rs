use std::fmt::Write;

use super::hash::{line_hash, HASH_CHARS};

fn looks_like_read_prefix(s: &str) -> bool {
    // Matches read output prefix like "8#VR:" or "  8#VR:"
    let s = s.trim();
    let (num_part, rest) = match s.split_once('#') {
        Some((n, r)) => (n, r),
        None => return false,
    };
    if num_part.parse::<usize>().is_err() {
        return false;
    }
    if rest.len() < 3 {
        return false;
    }
    let hash_chars = &rest.as_bytes()[..2];
    let colon = rest.as_bytes()[2];
    if colon != b':' {
        return false;
    }
    hash_chars.iter().all(|c| HASH_CHARS.contains(c))
}

pub(crate) fn contains_patch_markers(lines: &[String]) -> bool {
    for line in lines {
        let trimmed = line.trim();
        if looks_like_read_prefix(trimmed) {
            return true;
        }
        if trimmed.starts_with("+ ") || trimmed.starts_with("- ") {
            return true;
        }
    }
    false
}

pub(crate) fn resolve_anchor(anchor: &str, lines: &[String]) -> Option<usize> {
    let (line_str, hash) = anchor.split_once('#')?;
    let lineno: usize = line_str.parse().ok()?;
    if lineno == 0 || lineno > lines.len() {
        return None;
    }
    let actual = &lines[lineno - 1];
    let expected_hash = line_hash(actual, lineno);
    if expected_hash == hash {
        Some(lineno - 1)
    } else {
        None
    }
}

pub(crate) fn resolve_anchor_range(anchor: &str, lines: &[String]) -> Option<(usize, usize)> {
    let (range_str, hash) = anchor.split_once('#')?;
    let (start_str, end_str) = range_str.split_once('-')?;
    let start: usize = start_str.parse().ok()?;
    let end: usize = end_str.parse().ok()?;
    if start == 0 || end == 0 || start > end || end > lines.len() {
        return None;
    }
    let actual_start = &lines[start - 1];
    let start_hash = line_hash(actual_start, start);
    let actual_end = &lines[end - 1];
    let end_hash = line_hash(actual_end, end);
    let computed = format!("{start_hash}{end_hash}");
    if computed.as_str() != hash.chars().take(4).collect::<String>() {
        return None;
    }
    Some((start - 1, end))
}

pub(crate) fn make_updated_anchors(lines: &[String], start_line: usize, end_line: usize) -> String {
    let mut block = String::new();
    writeln!(block, "--- Updated anchors ---").ok();
    for i in start_line..end_line.min(lines.len()) {
        if let Some(line) = lines.get(i) {
            writeln!(block, "{}#{}:{}", i + 1, line_hash(line, i + 1), line).ok();
        }
    }
    block
}

pub(crate) fn generate_anchor_hint(lines: &[String], failed_anchor: &str) -> String {
    let (line_str, _) = failed_anchor.split_once('#').unwrap_or((failed_anchor, ""));
    let requested_line: usize = line_str.parse().unwrap_or(0);
    let mut hint = String::new();
    writeln!(hint, "Current state of lines around line {requested_line}:").ok();
    let start = requested_line.saturating_sub(2).max(1);
    let end = (requested_line + 2).min(lines.len());
    for i in start..=end {
        let idx = i - 1;
        if let Some(line) = lines.get(idx) {
            writeln!(hint, "{}#{}:{}", i, line_hash(line, i), line).ok();
        }
    }
    hint
}
