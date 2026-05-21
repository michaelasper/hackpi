mod filter;
mod search;
mod tool;

pub use tool::SearchGrepTool;

#[cfg(test)]
mod tests {
    use super::filter::filter_entry;
    use super::tool::SearchGrepTool;
    use hackpi_core::tools::Tool;
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
    async fn test_search_grep_include_glob_relative_matches_correctly() {
        let dir = temp_dir();
        create_file(&dir, "src/auth.rs", "pub fn handle_auth() {}");
        create_file(&dir, "src/db.rs", "pub fn query_db() {}");
        create_file(&dir, "tests/integration.rs", "mod auth; mod db;");

        let tool = SearchGrepTool::new(dir.clone());
        // Relative glob should match files under src/ only
        let params = serde_json::json!({
            "pattern": "fn",
            "include_glob": "src/**/*.rs"
        });
        let ctx = hackpi_core::tools::ToolContext {
            workspace_root: dir.clone(),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        let content = match result {
            hackpi_core::tools::ToolResult::Success { content } => content,
            other => panic!("expected Success, got {other:?}"),
        };

        assert!(
            content.contains("src/auth.rs"),
            "output should include src/auth.rs, got: {content}"
        );
        assert!(
            content.contains("src/db.rs"),
            "output should include src/db.rs, got: {content}"
        );
        assert!(
            !content.contains("tests/"),
            "output should NOT include files under tests/, got: {content}"
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
