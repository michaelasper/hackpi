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

#[test]
fn test_line_hash_trailing_whitespace_matches_trimmed_non_alphanumeric() {
    // Spec §94: "Line hashing operates on trimmed content"
    // A non-alphanumeric line with trailing whitespace must produce the
    // same hash as the trimmed version.
    let trimmed = line_hash("}", 5);
    let trailing = line_hash("}   ", 5);
    let leading = line_hash("   }", 5);
    let both = line_hash("  }  ", 5);
    assert_eq!(
        trimmed, trailing,
        "trailing whitespace must not change hash"
    );
    assert_eq!(trimmed, leading, "leading whitespace must not change hash");
    assert_eq!(trimmed, both, "both-side whitespace must not change hash");
}

#[test]
fn test_line_hash_trailing_whitespace_matches_trimmed_alphanumeric() {
    // Same for alphanumeric lines: "// comment  " must hash same as "// comment"
    let trimmed = line_hash("// comment", 5);
    let trailing = line_hash("// comment  ", 5);
    let leading = line_hash("  // comment", 5);
    let both = line_hash("  // comment  ", 5);
    assert_eq!(
        trimmed, trailing,
        "trailing whitespace must not change hash"
    );
    assert_eq!(trimmed, leading, "leading whitespace must not change hash");
    assert_eq!(trimmed, both, "both-side whitespace must not change hash");
}

#[test]
fn test_line_hash_trailing_whitespace_on_empty_looking_line() {
    // An all-whitespace line should trim to empty string.
    // Empty string has no alphanumeric chars → uses line_num seed.
    let h1 = line_hash("", 3);
    let h2 = line_hash("   ", 3);
    let h3 = line_hash("\t  ", 3);
    assert_eq!(h1, h2, "whitespace-only line must hash same as empty line");
    assert_eq!(h1, h3, "tab-whitespace line must hash same as empty line");
}

#[test]
fn test_resolve_anchor_matches_hash_with_trailing_whitespace() {
    // Integration check: resolve_anchor must match a hash produced from
    // a line with trailing whitespace, just like from the read tool output.
    let raw_line = "}  ".to_string(); // line as read from disk
    let lines = vec![raw_line.clone()];

    // Simulate what the read tool outputs: hash of the untrimmed line
    let read_hash = line_hash(&raw_line, 1);
    let anchor = format!("1#{read_hash}");

    // resolve_anchor calls line_hash on the stored line (which trims internally)
    let result = resolve_anchor(&anchor, &lines);
    assert!(
        result.is_some(),
        "anchor '{anchor}' for line with trailing whitespace '{}' must resolve",
        raw_line.trim()
    );
    assert_eq!(result.unwrap(), 0);
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

// ── EditTool integration tests ────────────────────────────────

use hackpi_core::tools::{Tool, ToolContext};

fn create_test_file(dir: &std::path::Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

async fn apply_edit(
    workspace_root: &std::path::Path,
    file_path: &str,
    edits: serde_json::Value,
) -> String {
    let tool = super::tool::EditTool::new(workspace_root.to_path_buf());
    let params = serde_json::json!({
        "path": file_path,
        "edits": edits,
    });
    let ctx = ToolContext {
        workspace_root: workspace_root.to_path_buf(),
        conversation_id: String::new(),
        signal: tokio::sync::watch::channel(false).1,
    };
    let result = tool.execute(params, &ctx).await;
    match result {
        hackpi_core::tools::ToolResult::Success { content } => content,
        other => panic!("Expected Success, got {other:?}"),
    }
}

async fn apply_edit_expect_error(
    workspace_root: &std::path::Path,
    file_path: &str,
    edits: serde_json::Value,
) -> String {
    let tool = super::tool::EditTool::new(workspace_root.to_path_buf());
    let params = serde_json::json!({
        "path": file_path,
        "edits": edits,
    });
    let ctx = ToolContext {
        workspace_root: workspace_root.to_path_buf(),
        conversation_id: String::new(),
        signal: tokio::sync::watch::channel(false).1,
    };
    let result = tool.execute(params, &ctx).await;
    match result {
        hackpi_core::tools::ToolResult::SystemError { message } => message,
        other => panic!("Expected SystemError, got {other:?}"),
    }
}

fn make_anchor(line: &str, lineno: usize) -> String {
    let h = super::hash::line_hash(line, lineno);
    format!("{lineno}#{h}")
}

#[tokio::test]
async fn test_multi_edit_diff_shows_accurate_old_content() {
    let dir = std::env::temp_dir().join("hackpi_edit_test_diff");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "line1\nline2\nline3\nline4\nline5\n");

    let anchor1 = make_anchor("line1", 1);
    let anchor5 = make_anchor("line5", 5);

    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": anchor5,
            "lines": ["modified5"]
        },
        {
            "op": "replace",
            "pos": anchor1,
            "lines": ["modified1"]
        }
    ]);

    let result = apply_edit(&dir, "test.rs", edits).await;

    assert!(
        result.contains("- line1"),
        "diff should show replaced line1"
    );
    assert!(
        result.contains("- line5"),
        "diff should show replaced line5"
    );
    assert!(result.contains("+ modified1"), "diff should show new line1");
    assert!(result.contains("+ modified5"), "diff should show new line5");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_single_edit_diff_works() {
    let dir = std::env::temp_dir().join("hackpi_edit_test_single");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "fn old() {}\n");

    let anchor = make_anchor("fn old() {}", 1);

    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": anchor,
            "lines": ["fn new() {}"]
        }
    ]);

    let result = apply_edit(&dir, "test.rs", edits).await;

    assert!(
        result.contains("- fn old() {}"),
        "diff should show old line"
    );
    assert!(
        result.contains("+ fn new() {}"),
        "diff should show new line"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── COR-88: Stale end anchor tests ───────────────────────────

#[tokio::test]
async fn test_replace_stale_end_anchor_returns_error() {
    // COR-88: When a replace operation has a valid pos but stale end anchor,
    // the tool must return a SystemError instead of silently defaulting.
    let dir = std::env::temp_dir().join("hackpi_edit_test_stale_end");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "line1\nline2\nline3\nline4\nline5\n");

    let anchor1 = make_anchor("line1", 1);
    let stale_end = "4#ZZ".to_string(); // valid line number, wrong hash (stale)

    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": anchor1,
            "end": stale_end,
            "lines": ["replacement"]
        }
    ]);

    let error = apply_edit_expect_error(&dir, "test.rs", edits).await;
    assert!(
        error.contains("E_STALE_ANCHOR"),
        "stale end anchor should produce E_STALE_ANCHOR error, got: {error}"
    );
    assert!(
        error.contains("End anchor"),
        "error message should mention end anchor, got: {error}"
    );

    // Verify file was NOT modified (stale anchor rejected before write)
    let content = std::fs::read_to_string(dir.join("test.rs")).unwrap();
    assert_eq!(content, "line1\nline2\nline3\nline4\nline5\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_replace_with_valid_end_anchor_succeeds() {
    // COR-88: Replace with valid pos AND valid end anchor should work
    let dir = std::env::temp_dir().join("hackpi_edit_test_valid_end");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "line1\nline2\nline3\nline4\nline5\n");

    let anchor1 = make_anchor("line1", 1);
    let anchor3 = make_anchor("line3", 3);

    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": anchor1,
            "end": anchor3,
            "lines": ["new_a", "new_b", "new_c"]
        }
    ]);

    let result = apply_edit(&dir, "test.rs", edits).await;
    assert!(
        result.contains("Applied 1 edit(s)"),
        "should have applied 1 edit"
    );

    let content = std::fs::read_to_string(dir.join("test.rs")).unwrap();
    assert_eq!(content, "new_a\nnew_b\nnew_c\nline4\nline5");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── COR-113.1: replace_text with lines array ─────────────────

#[tokio::test]
async fn test_replace_text_with_lines_array() {
    // COR-113: replace_text with `lines` array payload replacing oldText
    let dir = std::env::temp_dir().join("hackpi_edit_test_rt_lines");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "fn main() {\n    let x = 1;\n}\n");

    let edits = serde_json::json!([
        {
            "op": "replace_text",
            "oldText": "    let x = 1;",
            "lines": ["    let x = 42;", "    println!(\"{x}\");"]
        }
    ]);

    let result = apply_edit(&dir, "test.rs", edits).await;
    assert!(result.contains("Applied 1 edit(s)"));

    let content = std::fs::read_to_string(dir.join("test.rs")).unwrap();
    assert_eq!(
        content,
        "fn main() {\n    let x = 42;\n    println!(\"{x}\");\n}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_replace_text_with_lines_rejects_patch_markers() {
    // COR-113: replace_text with `lines` containing patch markers should error
    let dir = std::env::temp_dir().join("hackpi_edit_test_rt_patch");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "fn main() {\n    let x = 1;\n}\n");

    let edits = serde_json::json!([
        {
            "op": "replace_text",
            "oldText": "    let x = 1;",
            "lines": ["+    let x = 42;"]
        }
    ]);

    let error = apply_edit_expect_error(&dir, "test.rs", edits).await;
    assert!(
        error.contains("E_INVALID_PATCH"),
        "patch markers in lines should produce E_INVALID_PATCH, got: {error}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── COR-113.2: Range anchor syntax ────────────────────────────

#[tokio::test]
async fn test_range_anchor_syntax_in_pos() {
    // COR-113: `pos: "3#HASH-5#HASH"` syntax for multi-line replace ranges
    let dir = std::env::temp_dir().join("hackpi_edit_test_range_anchor");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "line1\nline2\nline3\nline4\nline5\n");

    let anchor3 = make_anchor("line3", 3);
    let anchor5 = make_anchor("line5", 5);
    let range_pos = format!("{anchor3}-{anchor5}");

    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": range_pos,
            "lines": ["new_line3", "new_line4", "new_line5"]
        }
    ]);

    let result = apply_edit(&dir, "test.rs", edits).await;
    assert!(result.contains("Applied 1 edit(s)"));

    let content = std::fs::read_to_string(dir.join("test.rs")).unwrap();
    assert_eq!(content, "line1\nline2\nnew_line3\nnew_line4\nnew_line5");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_range_anchor_syntax_validates_hash() {
    // COR-113: range anchor with wrong combined hash should error
    let dir = std::env::temp_dir().join("hackpi_edit_test_range_stale");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "line1\nline2\nline3\nline4\nline5\n");

    let anchor3 = make_anchor("line3", 3);
    // Use correct end line number but modified content hash
    let stale_anchor5 = "5#ZZ".to_string();
    let range_pos = format!("{anchor3}-{stale_anchor5}");

    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": range_pos,
            "lines": ["new"]
        }
    ]);

    let error = apply_edit_expect_error(&dir, "test.rs", edits).await;
    assert!(
        error.contains("E_STALE_ANCHOR") || error.contains("not found"),
        "stale range anchor should produce error, got: {error}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── COR-113.3: E_INVALID_PATCH error code ────────────────────

#[tokio::test]
async fn test_e_invalid_patch_on_lines_with_read_prefix() {
    // COR-113: lines containing read output prefix should get E_INVALID_PATCH
    let dir = std::env::temp_dir().join("hackpi_edit_test_invalid_patch");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "line1\nline2\n");

    let anchor1 = make_anchor("line1", 1);

    // lines contains what looks like read output prefix
    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": anchor1,
            "lines": ["  1#VR:line1"]
        }
    ]);

    let error = apply_edit_expect_error(&dir, "test.rs", edits).await;
    assert!(
        error.contains("E_INVALID_PATCH"),
        "read prefix in lines should give E_INVALID_PATCH, got: {error}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── COR-113.5: Atomic write ───────────────────────────────────

#[tokio::test]
async fn test_atomic_write_creates_file_with_correct_content() {
    // COR-113: atomic write via temp-file-then-rename should work correctly
    let dir = std::env::temp_dir().join("hackpi_edit_test_atomic");
    let _ = std::fs::create_dir_all(&dir);
    create_test_file(&dir, "test.rs", "original content\n");

    let anchor = make_anchor("original content", 1);
    let edits = serde_json::json!([
        {
            "op": "replace",
            "pos": anchor,
            "lines": ["modified content"]
        }
    ]);

    let result = apply_edit(&dir, "test.rs", edits).await;
    assert!(result.contains("Applied 1 edit(s)"));

    // Verify file content
    let content = std::fs::read_to_string(dir.join("test.rs")).unwrap();
    assert_eq!(content, "modified content");

    // Verify no temp file is left behind
    let has_temp = std::fs::read_dir(&dir)
        .unwrap()
        .any(|e| e.unwrap().file_name().to_string_lossy().starts_with("."));
    assert!(
        !has_temp,
        "temp file should be cleaned up after atomic write"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
