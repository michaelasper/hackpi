use crate::types::ToolSchema;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

pub struct ToolContext {
    pub workspace_root: std::path::PathBuf,
    pub conversation_id: String,
    pub signal: tokio::sync::watch::Receiver<bool>,
}

#[derive(Debug, Clone)]
pub enum ToolResult {
    Success { content: String },
    SystemError { message: String },
    Timeout,
    Cancelled,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn all_schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .map(|t| ToolSchema {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    pub async fn dispatch(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        let tool = self.get(name)?;
        Some(tool.execute(params, ctx).await)
    }
}
