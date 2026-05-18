pub mod bash;
pub mod chunker;
pub mod edit;
pub mod path_jail;
pub mod read;
pub mod search_bm25;
pub mod search_grep;
pub mod write;

use hackpi_core::tools::ToolRegistry;
use std::path::Path;

pub fn register_all_tools(registry: &mut ToolRegistry, workspace_root: &Path) {
    registry.register(Box::new(read::ReadTool::new(workspace_root.to_path_buf())));
    registry.register(Box::new(search_grep::SearchGrepTool::new(
        workspace_root.to_path_buf(),
    )));
    registry.register(Box::new(write::WriteTool::new(
        workspace_root.to_path_buf(),
    )));
    registry.register(Box::new(edit::EditTool::new(workspace_root.to_path_buf())));
    registry.register(Box::new(bash::BashTool::new(workspace_root.to_path_buf())));
}
