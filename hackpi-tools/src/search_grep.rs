use async_trait::async_trait;
use grep_regex::RegexMatcher;
use grep_searcher::SearcherBuilder;
use grep_searcher::sinks::UTF8;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::path::Path;

const MAX_MATCHES: usize = 50;
const MAX_LINE_LENGTH: usize = 500;
const DEFAULT_CONTEXT: usize = 2;

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
            "required": ["pattern"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let pattern = match params.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::SystemError { message: "Missing 'pattern' parameter.".into() },
        };

        let include_glob = params
            .get("include_glob")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let context_lines = params
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_CONTEXT as u64)
            .min(10) as usize;

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

        let paths = match &include_glob {
            Some(glob) => {
                let pattern = globset::Glob::new(glob)
                    .map(|g| g.compile_matcher())
                    .ok();
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

        for file_path in paths {
            if match_count >= MAX_MATCHES {
                truncated = true;
                break;
            }

            let result = searcher.search_path(
                &matcher,
                &file_path,
                UTF8(|lnum, line| {
                    if match_count >= MAX_MATCHES {
                        return Ok(false);
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
                        output.push_str(&msg);
                        match_count += 1;
                        return Ok(true);
                    }

                    let msg = format!("{}:{}:  {line_str}\n", file_path.display(), lnum);
                    output.push_str(&msg);
                    match_count += 1;
                    Ok(true)
                }),
            );

            if let Err(e) = result {
                tracing::warn!("Search error in {}: {e}", file_path.display());
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

fn walkdir(
    root: &Path,
    results: &mut Vec<std::path::PathBuf>,
    glob: &globset::GlobMatcher,
) -> Result<(), std::io::Error> {
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            !name.starts_with('.')
                && name != "node_modules"
                && name != "target"
                && name != ".git"
        })
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            if glob.is_match(entry.path()) {
                results.push(entry.path().to_path_buf());
            }
        }
    }
    Ok(())
}
