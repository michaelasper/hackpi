use async_trait::async_trait;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::SearcherBuilder;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

use crate::chunker::{CodeChunker, GenericChunker, RustChunker};
use crate::search_bm25::Bm25Index;

const MAX_MATCHES: usize = 50;
const MAX_LINE_LENGTH: usize = 500;
const DEFAULT_CONTEXT: usize = 2;
const MAX_CONTEXT: usize = 10;

pub struct SearchGrepTool {
    workspace_root: std::path::PathBuf,
    bm25: std::sync::Mutex<Option<Bm25Index>>,
}

impl SearchGrepTool {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self {
            workspace_root,
            bm25: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl Tool for SearchGrepTool {
    fn name(&self) -> &str {
        "search_grep"
    }

    fn description(&self) -> &str {
        "Searches the codebase for a regex pattern. Returns matching lines with surrounding context."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regular expression to search for."
                },
                "include_glob": {
                    "type": "string",
                    "description": "Optional glob pattern to restrict the search (e.g. 'src/**/*.rs')."
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Number of context lines before and after each match. Max 10. Default 2."
                },
                "use_bm25": {
                    "type": "boolean",
                    "description": "When true, use BM25 ranking to order results by relevance"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let pattern = match params.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'pattern' parameter.".into(),
                }
            }
        };

        let include_glob = params
            .get("include_glob")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let context_lines = params
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_CONTEXT as u64)
            .min(MAX_CONTEXT as u64) as usize;

        let use_bm25 = params
            .get("use_bm25")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let matcher = match RegexMatcher::new(pattern) {
            Ok(m) => m,
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Invalid regex pattern '{pattern}': {e}"),
                }
            }
        };

        let mut builder = SearcherBuilder::new();
        builder
            .line_number(true)
            .after_context(context_lines)
            .before_context(context_lines);

        let mut searcher = builder.build();

        // Collect per-file matches so we can optionally re-rank with BM25
        let mut file_matches: HashMap<std::path::PathBuf, String> = HashMap::new();
        let mut match_count = 0;
        let mut truncated = false;

        let gitignore_patterns = load_gitignore_patterns(&self.workspace_root);

        // Walk the directory tree lazily, searching files as they are discovered
        // instead of collecting all paths into memory first.
        let glob_matcher = include_glob
            .as_ref()
            .and_then(|g| globset::Glob::new(g).ok())
            .map(|g| g.compile_matcher());

        let walk_root = self.workspace_root.clone();
        for entry in walkdir::WalkDir::new(&walk_root)
            .into_iter()
            .filter_entry(|e| filter_entry(e, &gitignore_patterns))
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }

            let file_path = entry.path().to_path_buf();

            // Apply glob filter
            if let Some(ref m) = glob_matcher {
                if !m.is_match(&file_path) {
                    continue;
                }
            }

            if match_count >= MAX_MATCHES {
                truncated = true;
                break;
            }

            let mut file_has_match = false;
            let mut file_output = String::new();

            let result = searcher.search_path(
                &matcher,
                &file_path,
                UTF8(|lnum, line| {
                    if match_count >= MAX_MATCHES {
                        return Ok(false);
                    }

                    if !file_has_match {
                        file_has_match = true;
                    }

                    let line_str = line.trim_end();
                    if line_str.len() > MAX_LINE_LENGTH {
                        let msg = format!(
                            "{}:{}: [line omitted: {} chars — exceeds {} char limit]\n",
                            file_path.display(),
                            lnum,
                            line_str.len(),
                            MAX_LINE_LENGTH
                        );
                        file_output.push_str(&msg);
                        match_count += 1;
                        return Ok(true);
                    }

                    let msg = format!("{}:{}:  {line_str}\n", file_path.display(), lnum);
                    file_output.push_str(&msg);
                    match_count += 1;
                    Ok(true)
                }),
            );

            if let Err(e) = result {
                tracing::warn!("Search error in {}: {e}", file_path.display());
            }

            if file_has_match {
                file_matches.insert(file_path, file_output);
            }
        }

        // Optionally re-rank results using BM25
        if use_bm25 {
            let mut bm25_guard = self.bm25.lock().unwrap();
            let should_rebuild = match &*bm25_guard {
                Some(index) => index.is_stale() || index.is_empty(),
                None => true,
            };

            if should_rebuild {
                let mut new_index = Bm25Index::new();
                let rust_chunker = RustChunker::new();
                let generic_chunker = GenericChunker::new();

                // Walk again to build the BM25 index (infrequent operation)
                let gitignore_patterns = load_gitignore_patterns(&self.workspace_root);
                for entry in walkdir::WalkDir::new(&self.workspace_root)
                    .into_iter()
                    .filter_entry(|e| filter_entry(e, &gitignore_patterns))
                    .flatten()
                {
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let file_path = entry.path();

                    if let Ok(content) = std::fs::read_to_string(file_path) {
                        let is_rust =
                            file_path.extension().map(|e| e == "rs").unwrap_or(false);
                        if is_rust {
                            let chunks = rust_chunker.chunk_file(file_path, &content);
                            if !chunks.is_empty() {
                                new_index.add_chunked_document(file_path, &chunks);
                                continue;
                            }
                        } else {
                            let chunks = generic_chunker.chunk_file(file_path, &content);
                            if !chunks.is_empty() {
                                new_index.add_chunked_document(file_path, &chunks);
                                continue;
                            }
                        }
                        new_index.add_document(file_path, &content);
                    }
                }
                new_index.build();
                *bm25_guard = Some(new_index);
            }

            if let Some(ref index) = *bm25_guard {
                let scored = index.search(pattern, file_matches.len());

                // Helper: convert a chunk path (filepath:type:name) back to its source file path
                fn source_file_path(p: &std::path::Path) -> std::path::PathBuf {
                    let s = p.to_string_lossy();
                    if let Some(colon_pos) = s.rfind(':') {
                        let before_colon = &s[..colon_pos];
                        if let Some(second_colon) = before_colon.rfind(':') {
                            return std::path::PathBuf::from(&s[..second_colon]);
                        }
                    }
                    p.to_path_buf()
                }

                // Build a map: source_path -> (score, chunk_type, chunk_name) for best chunk
                let mut best_chunk: HashMap<
                    std::path::PathBuf,
                    (f64, Option<String>, Option<String>),
                > = HashMap::new();
                for result in &scored {
                    let src = source_file_path(&result.path);
                    let entry = best_chunk.entry(src).or_insert((result.score, None, None));
                    if result.score > entry.0 {
                        *entry = (
                            result.score,
                            result.chunk_type.clone(),
                            result.chunk_name.clone(),
                        );
                    }
                }

                // Sort source paths by their best chunk score
                let mut ranked_sources: Vec<(
                    std::path::PathBuf,
                    (f64, Option<String>, Option<String>),
                )> = best_chunk.into_iter().collect();
                ranked_sources.sort_by(|a, b| {
                    b.1 .0
                        .partial_cmp(&a.1 .0)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                // Build ordered list: scored files first by BM25 score,
                // then unscored files from file_matches
                let mut ordered: Vec<(std::path::PathBuf, String)> = Vec::new();

                // Add scored files first
                for (src_path, _) in &ranked_sources {
                    if let Some(content) = file_matches.remove(src_path) {
                        ordered.push((src_path.clone(), content));
                    }
                }

                // Add remaining (unscored) files
                for (path, content) in file_matches.drain() {
                    ordered.push((path, content));
                }

                // Format output with re-ranked order
                let mut output = String::new();
                let mut first_file = true;
                for (src_path, (_, chunk_type, chunk_name)) in &ranked_sources {
                    let content = ordered
                        .iter()
                        .find(|(p, _)| p == src_path)
                        .map(|(_, c)| c.as_str())
                        .unwrap_or("");

                    if !first_file {
                        output.push('\n');
                    }
                    first_file = false;

                    let header = match (chunk_type, chunk_name) {
                        (Some(typ), Some(name)) if !name.is_empty() => {
                            format!("--- {}: {} {}() ---", src_path.display(), typ, name)
                        }
                        _ => format!("--- {} ---", src_path.display()),
                    };
                    output.push_str(&header);
                    output.push('\n');
                    output.push_str(content);
                }

                // Add remaining (unscored) files without chunk info
                for (path, content) in &ordered {
                    if ranked_sources.iter().any(|(p, _)| p == path) {
                        continue;
                    }
                    if !first_file {
                        output.push('\n');
                    }
                    first_file = false;
                    output.push_str(&format!("--- {} ---\n", path.display()));
                    output.push_str(content);
                }

                if truncated {
                    output.push_str(&format!(
                        "\n[Search truncated. Over {MAX_MATCHES} matches found. Refine your pattern or use include_glob.]"
                    ));
                }

                if output.is_empty() {
                    output = "No matches found.".to_string();
                }

                return ToolResult::Success { content: output };
            }
        }

        // Without BM25: format output in walk order
        let mut output = String::new();
        let mut first_file = true;

        // Determine walk order for output formatting
        let gitignore_patterns = load_gitignore_patterns(&self.workspace_root);
        let walk_root = self.workspace_root.clone();
        for entry in walkdir::WalkDir::new(&walk_root)
            .into_iter()
            .filter_entry(|e| filter_entry(e, &gitignore_patterns))
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }

            // Apply glob filter
            if let Some(ref m) = glob_matcher {
                if !m.is_match(entry.path()) {
                    continue;
                }
            }

            if let Some(content) = file_matches.remove(entry.path()) {
                if !first_file {
                    output.push('\n');
                }
                first_file = false;
                output.push_str(&format!("--- {} ---\n", entry.path().display()));
                output.push_str(&content);
            }
        }

        if truncated {
            output.push_str(&format!(
                "\n[Search truncated. Over {MAX_MATCHES} matches found. Refine your pattern or use include_glob.]"
            ));
        }

        if output.is_empty() {
            output = "No matches found.".to_string();
        }

        ToolResult::Success { content: output }
    }
}

fn load_gitignore_patterns(root: &Path) -> Vec<globset::GlobMatcher> {
    let gitignore_path = root.join(".gitignore");
    let content = match std::fs::read_to_string(&gitignore_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            // If pattern starts with /, anchor to repo root
            let pattern = if let Some(stripped) = line.strip_prefix('/') {
                format!("{stripped}*")
            } else {
                format!("**/{line}")
            };
            globset::Glob::new(&pattern)
                .ok()
                .map(|g| g.compile_matcher())
        })
        .collect()
}

/// Apply the standard filter_entry logic used by all walkdir iterations.
fn filter_entry(
    e: &walkdir::DirEntry,
    gitignore_patterns: &[globset::GlobMatcher],
) -> bool {
    let name = e.file_name().to_str().unwrap_or("");
    if name.starts_with('.') && name != "." {
        return false;
    }
    if name == "node_modules" || name == "target" || name == "dist" || name == "build" {
        return false;
    }
    if !gitignore_patterns.is_empty()
        && gitignore_patterns.iter().any(|p| p.is_match(e.path()))
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir() -> std::path::PathBuf {
        static COUNTER: std::sync::OnceLock<std::sync::atomic::AtomicU32> =
            std::sync::OnceLock::new();
        let c = COUNTER.get_or_init(|| std::sync::atomic::AtomicU32::new(0));
        let id = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hackpi_search_test_{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn create_file(dir: &std::path::Path, path: &str, content: &str) {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&full).unwrap();
        write!(f, "{content}").unwrap();
    }

    fn no_gitignore() -> Vec<globset::GlobMatcher> {
        Vec::new()
    }

    #[test]
    fn test_filter_entry_skips_dist() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "dist/bundle.js", "var x = 1;");

        let dist_entry = walkdir::WalkDir::new(&dir)
            .into_iter()
            .flatten()
            .find(|e| e.path().to_string_lossy().contains("dist"));
        assert!(dist_entry.is_some(), "dist entry should exist in walk");

        // filter_entry should reject dist
        let allowed = filter_entry(
            &walkdir::WalkDir::new(&dir)
                .into_iter()
                .flatten()
                .find(|e| e.path().to_string_lossy().contains("dist"))
                .unwrap(),
            &no_gitignore(),
        );
        assert!(!allowed, "filter_entry should reject dist/ directory");
    }

    #[test]
    fn test_filter_entry_skips_build() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "build/out.o", "binary");

        let allowed = filter_entry(
            &walkdir::WalkDir::new(&dir)
                .into_iter()
                .flatten()
                .find(|e| e.path().to_string_lossy().contains("build"))
                .unwrap(),
            &no_gitignore(),
        );
        assert!(!allowed, "filter_entry should reject build/ directory");
    }

    #[test]
    fn test_filter_entry_skips_node_modules() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "node_modules/pkg/index.js", "module.exports = 1;");

        let allowed = filter_entry(
            &walkdir::WalkDir::new(&dir)
                .into_iter()
                .flatten()
                .find(|e| e.path().to_string_lossy().contains("node_modules"))
                .unwrap(),
            &no_gitignore(),
        );
        assert!(
            !allowed,
            "filter_entry should reject node_modules/ directory"
        );
    }

    #[test]
    fn test_filter_entry_allows_src() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "src/main.rs", "fn main() {}");

        let allowed_count = walkdir::WalkDir::new(&dir)
            .into_iter()
            .flatten()
            .filter(|e| filter_entry(e, &no_gitignore()))
            .filter(|e| e.file_type().is_file())
            .count();
        assert!(
            allowed_count >= 2,
            "filter_entry should allow src files, got: {allowed_count}"
        );
    }

    #[tokio::test]
    async fn test_search_grep_hard_ignores_dist_and_build() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "dist/bundle.js", "var x = 1; var y = 2;");
        create_file(&dir, "build/out.o", "binary data here");

        let tool = SearchGrepTool::new(dir.clone());
        let params = serde_json::json!({ "pattern": "var" });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        let content = match result {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };
        assert!(
            !content.contains("dist/"),
            "search should not return results from dist/, got: {content}"
        );
        assert!(
            !content.contains("build/"),
            "search should not return results from build/, got: {content}"
        );
    }

    #[test]
    fn test_input_schema_has_additional_properties_false() {
        let tool = SearchGrepTool::new(std::path::PathBuf::from("/tmp"));
        let schema = tool.input_schema();
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&serde_json::json!(false)),
            "search_grep tool schema missing additionalProperties: false"
        );
    }

    #[tokio::test]
    async fn test_search_grep_output_format() {
        let dir = temp_dir();
        create_file(&dir, "src/auth.rs", "pub fn handle_auth(token: &str) {}");
        create_file(&dir, "src/db.rs", "use crate::auth::AuthStrategy;");

        let tool = SearchGrepTool::new(dir.clone());
        let params = serde_json::json!({ "pattern": "auth" });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        let content = match result {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };
        // Output should have: path:line:  content format with --- separators
        assert!(
            content.contains("src/auth.rs"),
            "output should include file path, got: {content}"
        );
        assert!(
            content.contains("---"),
            "output should have --- separator between file groups, got: {content}"
        );
        assert!(
            content.contains("handle_auth"),
            "output should include matching line content, got: {content}"
        );
    }

    #[tokio::test]
    async fn test_search_grep_repeated_search_still_finds_files() {
        let dir = temp_dir();
        create_file(&dir, "src/foo.rs", "fn foo() {}");
        create_file(&dir, "src/bar.rs", "fn bar() {}");

        let tool = SearchGrepTool::new(dir.clone());
        let params = serde_json::json!({ "pattern": "fn" });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };

        // First call
        let result1 = tool.execute(params.clone(), &ctx).await;
        assert!(
            matches!(result1, hackpi_core::tools::ToolResult::Success { .. }),
            "first search should succeed"
        );

        // Second call (no caching now — lazy walk each time)
        let result2 = tool.execute(params, &ctx).await;
        let content2 = match result2 {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };
        assert!(
            content2.contains("foo") && content2.contains("bar"),
            "repeated search should still find files, got: {content2}"
        );
    }

    // -----------------------------------------------------------------------
    // BM25 integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_search_grep_schema_has_use_bm25_parameter() {
        let tool = SearchGrepTool::new(std::path::PathBuf::from("/tmp"));
        let schema = tool.input_schema();

        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();
        assert!(
            props.contains_key("use_bm25"),
            "input_schema should include 'use_bm25' parameter, got keys: {:?}",
            props.keys().collect::<Vec<_>>()
        );

        let use_bm25 = props.get("use_bm25").unwrap();
        assert_eq!(
            use_bm25.get("type").and_then(|v| v.as_str()),
            Some("boolean"),
            "use_bm25 should be a boolean"
        );
        assert_eq!(
            use_bm25.get("description").and_then(|v| v.as_str()),
            Some("When true, use BM25 ranking to order results by relevance"),
            "use_bm25 should have a description"
        );
    }

    #[tokio::test]
    async fn test_search_grep_bm25_default_behavior_unchanged() {
        let dir = temp_dir();
        create_file(&dir, "src/a.rs", "fn alpha() { let x = 1; }");
        create_file(&dir, "src/b.rs", "fn beta() { let y = 2; }");

        let tool = SearchGrepTool::new(dir.clone());
        let params = serde_json::json!({ "pattern": "fn" });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };

        let result = tool.execute(params, &ctx).await;
        let content = match result {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };

        // Both files should still be found
        assert!(content.contains("a.rs"), "Should find a.rs: {content}");
        assert!(content.contains("b.rs"), "Should find b.rs: {content}");
    }

    #[tokio::test]
    async fn test_search_grep_bm25_ranks_relevant_files_first() {
        let dir = temp_dir();
        // File with the word "auth" appearing many times as a standalone token
        // (each `auth;` line produces a separate "auth" token after tokenization)
        create_file(
            &dir,
            "src/auth_impl.rs",
            "use auth;
             use auth::middleware;
             use auth::oauth;
             use auth::session;
             fn handle_auth(token: &str) -> bool { true }",
        );
        // File with just one mention of "auth"
        create_file(
            &dir,
            "src/db.rs",
            "use crate::auth::AuthStrategy;
             fn query_db() -> Vec<String> { vec![] }",
        );

        let tool = SearchGrepTool::new(dir.clone());
        let params = serde_json::json!({
            "pattern": "auth",
            "use_bm25": true
        });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };

        let result = tool.execute(params, &ctx).await;
        let content = match result {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };

        // Both files should be found
        assert!(
            content.contains("auth_impl.rs"),
            "Should find auth_impl.rs: {content}"
        );
        assert!(content.contains("db.rs"), "Should find db.rs: {content}");

        // auth_impl.rs should appear before db.rs (higher BM25 relevance)
        let auth_pos = content.find("auth_impl.rs").unwrap();
        let db_pos = content.find("db.rs").unwrap();
        assert!(
            auth_pos < db_pos,
            "auth_impl.rs should be ranked before db.rs (auth_impl at {auth_pos}, db at {db_pos})\n{content}"
        );
    }

    #[tokio::test]
    async fn test_search_grep_bm25_without_match_falls_back() {
        let dir = temp_dir();
        create_file(&dir, "src/a.rs", "fn alpha() {}");
        create_file(&dir, "src/b.rs", "fn beta() {}");

        let tool = SearchGrepTool::new(dir.clone());
        let params = serde_json::json!({
            "pattern": "nonexistent",
            "use_bm25": true
        });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };

        let result = tool.execute(params, &ctx).await;
        let content = match result {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };

        // Should still show "No matches found."
        assert!(
            content.contains("No matches found"),
            "Should say no matches: {content}"
        );
    }

    #[tokio::test]
    async fn test_search_grep_bm25_rebuilds_on_stale_index() {
        let dir = temp_dir();
        create_file(&dir, "src/a.rs", "fn alpha() { auth(); }");
        create_file(&dir, "src/b.rs", "fn beta() { auth(); auth(); }");
        let file_a = dir.join("src/a.rs");

        let tool = SearchGrepTool::new(dir.clone());
        let params = serde_json::json!({
            "pattern": "auth",
            "use_bm25": true
        });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };

        // First search builds index
        let result1 = tool.execute(params.clone(), &ctx).await;
        assert!(matches!(
            result1,
            hackpi_core::tools::ToolResult::Success { .. }
        ));

        // Modify file_a so the BM25 index becomes stale
        std::fs::write(&file_a, "fn alpha() { auth(); auth(); auth(); }").unwrap();

        // Second search should detect staleness and rebuild
        let result2 = tool.execute(params, &ctx).await;
        let content = match result2 {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };

        // Both files should be found
        assert!(
            content.contains("a.rs"),
            "Should find a.rs after rebuild: {content}"
        );
        assert!(
            content.contains("b.rs"),
            "Should find b.rs after rebuild: {content}"
        );
    }
}
