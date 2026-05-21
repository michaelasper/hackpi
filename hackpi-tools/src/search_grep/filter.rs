use std::path::Path;

/// Load .gitignore patterns from the repository root.
pub(super) fn load_gitignore_patterns(root: &Path) -> Vec<globset::GlobMatcher> {
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

/// Apply the standard filter_entry logic used by all walkdir iterations.
pub(super) fn filter_entry(
    e: &walkdir::DirEntry,
    gitignore_patterns: &[globset::GlobMatcher],
) -> bool {
    let name = e.file_name().to_str().unwrap_or("");
    if name.starts_with('.') && name != "." {
        return false;
    }
    if name == "node_modules" || name == "target" || name == "dist" || name == "build" {
        return false;
    }
    if !gitignore_patterns.is_empty() && gitignore_patterns.iter().any(|p| p.is_match(e.path())) {
        return false;
    }
    true
}
