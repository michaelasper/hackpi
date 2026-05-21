use std::path::Path;

use super::memory::FileNode;

/// Resolve a path relative to `node` into a reference to the target node.
///
/// Handles `.`, `..`, and absolute paths anchored at the given root node.
/// Does NOT follow symlinks.
pub(crate) fn resolve_path_ref<'a>(node: &'a FileNode, path: &Path) -> Option<&'a FileNode> {
    let components: Vec<_> = path.components().collect();
    let mut segments: Vec<&str> = Vec::new();
    for comp in components {
        let name = comp.as_os_str().to_str()?;
        match name {
            "/" | "." => continue,
            ".." => {
                segments.pop();
            }
            _ => segments.push(name),
        }
    }
    let mut current = node;
    for seg in segments {
        current = current.children.get(seg)?;
    }
    Some(current)
}

/// Like `resolve_path_ref`, but follows symlinks on the final component.
/// Does NOT follow symlinks on intermediate path components.
pub(crate) fn resolve_path_follow<'a>(root: &'a FileNode, path: &Path) -> Option<&'a FileNode> {
    let node = resolve_path_ref(root, path)?;
    follow_symlinks(root, node, path)
}

/// Follow symlink chain starting from `node` up to 10 levels deep.
pub(crate) fn follow_symlinks<'a>(
    root: &'a FileNode,
    mut node: &'a FileNode,
    path: &Path,
) -> Option<&'a FileNode> {
    let mut current_path = path.to_path_buf();
    for _ in 0..10 {
        if !node.is_symlink {
            return Some(node);
        }
        let target = node.symlink_target.as_ref()?;
        let resolved = if target.is_absolute() {
            resolve_path_ref(root, target)?
        } else {
            let parent = current_path.parent()?;
            let full = parent.join(target);
            current_path = full.clone();
            resolve_path_ref(root, &full)?
        };
        node = resolved;
    }
    None // Too many levels of symlink indirection
}
