use std::path::Path;

use super::parsing::compile_glob;

/// Check whether a path matches a glob pattern.
///
/// The path is matched in three ways:
/// 1. As a relative path joined with `workspace_root`
/// 2. As an absolute path (used as-is)
/// 3. With `~/` resolved to the user's home directory
pub fn path_matches_glob(path: &Path, pattern: &str, workspace_root: &Path) -> bool {
    let matcher = match compile_glob(pattern) {
        Ok(m) => m,
        Err(_) => return false,
    };

    // 1. Try relative path joined with workspace_root
    if let Ok(relative) = path.strip_prefix(workspace_root) {
        if matcher.is_match(relative) {
            return true;
        }
    }

    // Also try the path as-is (could be relative to workspace_root without the prefix)
    if matcher.is_match(path) {
        return true;
    }

    // 2. Try absolute path
    if path.is_absolute() && matcher.is_match(path) {
        return true;
    }

    // 3. Try ~/ path resolution
    let path_str = path.to_string_lossy();
    if path_str.starts_with("~/") || path_str == "~" {
        if let Some(home) = home::home_dir() {
            if path_str == "~" {
                if matcher.is_match(&home) {
                    return true;
                }
            } else {
                let resolved = home.join(&path_str[2..]);
                if matcher.is_match(&resolved) {
                    return true;
                }
            }
        }
    }

    false
}
