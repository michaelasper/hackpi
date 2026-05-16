pub mod bash;
pub mod edit;
pub mod read;
pub mod search_grep;
pub mod write;

use hackpi_core::tools::ToolRegistry;
use std::path::PathBuf;

pub fn register_all_tools(registry: &mut ToolRegistry, workspace_root: &PathBuf) {
    registry.register(Box::new(read::ReadTool::new(workspace_root.clone())));
    registry.register(Box::new(search_grep::SearchGrepTool::new(workspace_root.clone())));
    registry.register(Box::new(write::WriteTool::new(workspace_root.clone())));
    registry.register(Box::new(edit::EditTool::new(workspace_root.clone())));
    registry.register(Box::new(bash::BashTool::new(workspace_root.clone())));
}
