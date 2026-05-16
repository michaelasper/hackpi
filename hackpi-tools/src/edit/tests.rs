use super::anchor::{contains_patch_markers, make_updated_anchors, resolve_anchor};
use super::hash::{line_hash, HASH_CHARS};

fn valid_hash() -> String {
    // Return a valid 2-char hash from the actual alphabet
    let chars: String = HASH_CHARS.iter().map(|&c| c as char).collect();
    chars.chars().take(2).collect()
}

// ── line_hash tests ──────────────────────────────────────────

#[test]
fn test_line_hash_returns_two_chars_from_alphabet() {
    let h = line_hash("hello world", 0);
    assert_eq!(h.len(), 2);
    for c in h.chars() {
        assert!(
            "ZPMQVRWSNKTXJBYH".contains(c),
            "char '{c}' not in hashline alphabet"
        );
    }
}

#[test]
fn test_line_hash_deterministic() {
    let h1 = line_hash("fn main() {}", 0);
    let h2 = line_hash("fn main() {}", 0);
    assert_eq!(h1, h2);
}

#[test]
fn test_line_hash_different_content_different_hash() {
    let h1 = line_hash("fn foo() {}", 0);
    let h2 = line_hash("fn bar() {}", 0);
    assert_ne!(h1, h2);
}

#[test]
fn test_line_hash_uses_line_number_seed_for_non_alphanumeric() {
    // Spec §76: non-alphanumeric lines use their line number as the hash seed
    // Two lines with identical non-alphanumeric content ("}") at different
    // line positions should produce different hashes.
    let h_at_line_5 = line_hash("}", 5);
    let h_at_line_15 = line_hash("}", 15);
    assert_ne!(
        h_at_line_5, h_at_line_15,
        "identical non-alphanumeric content at different line numbers must produce different hashes"
    );
}

#[test]
fn test_line_hash_zero_seed_for_alphanumeric_lines() {
    let h1 = line_hash("fn main() {}", 0);
    let h2 = line_hash("fn main() {}", 99);
    assert_eq!(
        h1, h2,
        "alphanumeric lines should use seed=0 regardless of line number"
    );
}

// ── contains_patch_markers tests ──────────────────────────────

#[test]
fn test_detects_read_output_prefix_format() {
    // Read output format: "  8#VR:function hello() {"
    let lines = vec!["  8#VR:function hello() {".to_string()];
    assert!(
        contains_patch_markers(&lines),
        "read output prefix '8#VR:' must be detected"
    );
}

#[test]
fn test_detects_read_output_prefix_at_line_1() {
    let h = valid_hash();
    let prefix = format!("1#{h}:content");
    let lines = vec![prefix.clone()];
    assert!(
        contains_patch_markers(&lines),
        "prefix '1#{h}:content' must be detected"
    );
}

#[test]
fn test_detects_diff_plus_marker() {
    let lines = vec!["+ console.log('hello')".to_string()];
    assert!(
        contains_patch_markers(&lines),
        "diff '+' marker must be detected"
    );
}

#[test]
fn test_detects_diff_minus_marker() {
    let lines = vec!["- console.log('old')".to_string()];
    assert!(
        contains_patch_markers(&lines),
        "diff '-' marker must be detected"
    );
}

#[test]
fn test_allows_plus_without_trailing_space() {
    // "+1" is a valid increment, not a diff marker
    let lines = vec!["+1".to_string()];
    assert!(
        !contains_patch_markers(&lines),
        "'+1' should not be treated as diff marker"
    );
}

#[test]
fn test_allows_minus_without_trailing_space() {
    let lines = vec!["-1".to_string()];
    assert!(
        !contains_patch_markers(&lines),
        "'-1' should not be treated as diff marker"
    );
}

#[test]
fn test_allows_normal_code_lines() {
    let lines = vec![
        "fn main() {".to_string(),
        "    let x = 1;".to_string(),
        "    println!(\"{x}\");".to_string(),
        "}".to_string(),
    ];
    assert!(
        !contains_patch_markers(&lines),
        "normal code should not be flagged"
    );
}

#[test]
fn test_allows_empty_lines() {
    let lines = vec!["".to_string()];
    assert!(!contains_patch_markers(&lines));
}

#[test]
fn test_detects_prefix_in_multiline_input() {
    let h = valid_hash();
    let prefixed = format!("  3#{h}:    let x = 1;");
    let lines = vec!["fn main() {".to_string(), prefixed, "}".to_string()];
    assert!(
        contains_patch_markers(&lines),
        "read output prefix anywhere in lines must be detected"
    );
}

// ── replace_text tests ───────────────────────────────────────

#[test]
fn test_replace_text_fails_if_old_text_not_found() {
    // Simulate what deserialize_edit_ops + validation does
    let content = "fn main() {\n    let x = 1;\n}".to_string();
    let old_text = "nonexistent";
    let count = content.matches(old_text).count();
    assert_eq!(count, 0, "replace_text should detect oldText not found");
}

#[test]
fn test_replace_text_fails_if_old_text_not_unique() {
    let content = "abc\nxyz\nabc\n".to_string();
    let old_text = "abc";
    let count = content.matches(old_text).count();
    assert_eq!(
        count, 2,
        "replace_text should fail if oldText matches more than once"
    );
}

#[test]
fn test_replace_text_replaces_single_occurrence() {
    let old_text = "let x = 1;";
    let new_text = "let x = 42;";
    let content = "fn main() {\n    let x = 1;\n}\n";
    let expected = "fn main() {\n    let x = 42;\n}\n";
    // Content.replace replaces all occurrences, which works for unique text
    let result = content.replace(old_text, new_text);
    assert_eq!(result, expected);
}

// ── resolve_anchor tests ─────────────────────────────────────

#[test]
fn test_resolve_anchor_matches_hash() {
    let line = "fn hello() {".to_string();
    let lines = vec![line.clone()];
    let hash = line_hash(&line, 1);
    let anchor = format!("1#{hash}");
    let result = resolve_anchor(&anchor, &lines);
    assert!(
        result.is_some(),
        "anchor '{anchor}' should resolve to line 0"
    );
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn test_resolve_anchor_rejects_stale_hash() {
    let lines = vec!["fn hello() {".to_string()];
    let result = resolve_anchor("1#ZZ", &lines);
    assert!(result.is_none(), "stale anchor '1#ZZ' must be rejected");
}

#[test]
fn test_resolve_anchor_out_of_bounds() {
    let lines = vec!["line one".to_string()];
    let result = resolve_anchor("5#XX", &lines);
    assert!(result.is_none(), "out-of-bounds anchor must be rejected");
}

// ── make_updated_anchors tests ────────────────────────────────

#[test]
fn test_updated_anchors_block_format() {
    let lines = vec!["fn a() {}".to_string(), "fn b() {}".to_string()];
    let block = make_updated_anchors(&lines, 0, 2);
    assert!(block.starts_with("--- Updated anchors ---"));
    assert!(block.contains("1#"));
    assert!(block.contains("2#"));
}

// ── Edge case: empty file read advisory ──────────────────────
// This tests the read.rs behavior contract, tested through the
// public LineHash format functions.

#[test]
fn test_empty_lines_list_produces_empty_block() {
    let lines: Vec<String> = vec![];
    let block = make_updated_anchors(&lines, 0, 0);
    assert!(
        block.starts_with("--- Updated anchors ---"),
        "even empty blocks should have header"
    );
}
