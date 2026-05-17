pub mod config;
pub mod git_read;
pub mod git_write;
pub mod github;
pub mod github_api;

pub use config::VcsConfig;
use hackpi_core::tools::ToolRegistry;
use std::path::Path;

pub fn register_vcs_tools(
    registry: &mut ToolRegistry,
    workspace_root: &Path,
    vcs_config: &VcsConfig,
) {
    // Will register GitReadTool, GitWriteTool, GitHubTool in later phases
    // For now, just a placeholder
    let _ = (registry, workspace_root, vcs_config);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hackpi_core::tools::ToolRegistry;
    use std::path::Path;

    #[test]
    fn test_register_vcs_tools_does_not_panic() {
        let mut registry = ToolRegistry::new();
        let config = VcsConfig::from_env(Path::new("/tmp"));
        register_vcs_tools(&mut registry, Path::new("/tmp"), &config);
        // Should not panic — currently registers nothing
    }
}
