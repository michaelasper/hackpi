use async_trait::async_trait;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::SearcherBuilder;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::path::Path;

const MAX_MATCHES: usize = 50;
const MAX_LINE_LENGTH: usize = 500;
const DEFAULT_CONTEXT: usize = 2;
const MAX_CONTEXT: usize = 10;

pub struct SearchGrepTool {
    workspace_root: std::path::PathBuf,
}

impl SearchGrepTool {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self { workspace_root }
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

        let mut output = String::new();
        let mut match_count = 0;
        let mut truncated = false;

        let mut first_file = true;

        let paths = match &include_glob {
            Some(glob) => {
                let pattern = globset::Glob::new(glob).map(|g| g.compile_matcher()).ok();
                let mut matched = Vec::new();
                if let Some(ref matcher) = pattern {
                    let _ = walkdir(&self.workspace_root, &mut matched, matcher);
                }
                matched
            }
            None => {
                let mut all = Vec::new();
                let no_filter = globset::Glob::new("*").unwrap().compile_matcher();
                let _ = walkdir(&self.workspace_root, &mut all, &no_filter);
                all
            }
        };

        let gitignore_patterns = load_gitignore_patterns(&self.workspace_root);

        for file_path in paths {
            if match_count >= MAX_MATCHES {
                truncated = true;
                break;
            }

            if is_ignored_by_gitignore(&file_path, &gitignore_patterns) {
                continue;
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
                if !first_file {
                    output.push('\n');
                }
                first_file = false;
                output.push_str(&format!("--- {} ---\n", file_path.display()));
                output.push_str(&file_output);
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

fn is_ignored_by_gitignore(path: &Path, patterns: &[globset::GlobMatcher]) -> bool {
    patterns.iter().any(|p| p.is_match(path))
}

fn walkdir(
    root: &Path,
    results: &mut Vec<std::path::PathBuf>,
    glob: &globset::GlobMatcher,
) -> Result<(), std::io::Error> {
    let gitignore_patterns = load_gitignore_patterns(root);

    for entry in walkdir::WalkDir::new(root).into_iter().filter_entry(|e| {
        let name = e.file_name().to_str().unwrap_or("");
        if name.starts_with('.') && name != "." {
            return false;
        }
        if name == "node_modules" || name == "target" || name == "dist" || name == "build" {
            return false;
        }
        if is_ignored_by_gitignore(e.path(), &gitignore_patterns) {
            return false;
        }
        true
    }) {
        let entry = entry?;
        if entry.file_type().is_file() && glob.is_match(entry.path()) {
            results.push(entry.path().to_path_buf());
        }
    }
    Ok(())
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

    #[test]
    fn test_walkdir_skips_dist() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "dist/bundle.js", "var x = 1;");

        let mut results = Vec::new();
        let glob = globset::Glob::new("*").unwrap().compile_matcher();
        walkdir(&dir, &mut results, &glob).unwrap();

        let has_dist = results.iter().any(|p| p.to_string_lossy().contains("dist"));
        assert!(
            !has_dist,
            "walkdir should skip dist/ directory, found: {results:?}"
        );
    }

    #[test]
    fn test_walkdir_skips_build() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "build/out.o", "binary");

        let mut results = Vec::new();
        let glob = globset::Glob::new("*").unwrap().compile_matcher();
        walkdir(&dir, &mut results, &glob).unwrap();

        let has_build = results
            .iter()
            .any(|p| p.to_string_lossy().contains("build"));
        assert!(
            !has_build,
            "walkdir should skip build/ directory, found: {results:?}"
        );
    }

    #[test]
    fn test_walkdir_includes_src() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "src/main.rs", "fn main() {}");

        let mut results = Vec::new();
        let glob = globset::Glob::new("*").unwrap().compile_matcher();
        walkdir(&dir, &mut results, &glob).unwrap();

        assert!(
            results.len() >= 2,
            "walkdir should include src files, found: {results:?}"
        );
    }

    #[test]
    fn test_walkdir_skips_node_modules() {
        let dir = temp_dir();
        create_file(&dir, "src/lib.rs", "fn foo() {}");
        create_file(&dir, "node_modules/pkg/index.js", "module.exports = 1;");

        let mut results = Vec::new();
        let glob = globset::Glob::new("*").unwrap().compile_matcher();
        walkdir(&dir, &mut results, &glob).unwrap();

        let has_nm = results
            .iter()
            .any(|p| p.to_string_lossy().contains("node_modules"));
        assert!(
            !has_nm,
            "walkdir should skip node_modules/, found: {results:?}"
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
}
