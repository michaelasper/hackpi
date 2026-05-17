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
    _vcs_config: &VcsConfig,
) {
    registry.register(Box::new(git_read::GitReadTool::new(
        workspace_root.to_path_buf(),
    )));
    // GitWriteTool and GitHubTool will be registered in later phases
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
