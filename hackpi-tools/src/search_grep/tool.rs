use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;

use super::search;

pub struct SearchGrepTool {
    workspace_root: std::path::PathBuf,
    bm25: std::sync::Mutex<Option<crate::search_bm25::Bm25Index>>,
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

        let include_glob = params.get("include_glob").and_then(|v| v.as_str());

        let context_lines = params
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(search::DEFAULT_CONTEXT as u64)
            .min(search::MAX_CONTEXT as u64) as usize;

        let use_bm25 = params
            .get("use_bm25")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        search::execute_search(
            &self.workspace_root,
            &self.bm25,
            pattern,
            include_glob,
            context_lines,
            use_bm25,
        )
    }
}
