use std::path::{Component, Path, PathBuf};

use hackpi_core::tools::ToolResult;

/// Normalize a path by resolving `.` and `..` components without touching
/// the filesystem. This mirrors the lexical normalization that
/// `std::fs::canonicalize` performs on the path string, but without
/// resolving symlinks or requiring any component to exist.
///
/// This is essential for the path-jail security check: the naive
/// `workspace_root.join("foo/../../../etc/passwd")` produces a PathBuf whose
/// `starts_with(workspace_root)` is `true` (because `..` is just another
/// component). Normalizing first collapses those `..` components so the
/// boundary check sees the true target location.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => { /* skip `.` */ }
            Component::ParentDir => {
                // Pop the last real component. If there's nothing to pop
                // (e.g. we'd escape above root), preserve the `..` so the
                // resulting path visibly escapes and fails starts_with().
                if !normalized.pop() {
                    normalized.push(component);
                }
            }
            _ => normalized.push(component),
        }
    }
    normalized
}

/// Resolve a file path relative to a workspace root, enforcing that the
/// resolved path stays within the workspace boundary.
///
/// 1. Rejects absolute `file_path` values (which would bypass `join`).
/// 2. Canonicalizes the workspace root and the resolved path.
/// 3. Verifies the resolved path starts with the canonical workspace root.
///
/// If the file does not yet exist (e.g. for write operations), the parent
/// directory is canonicalized and the file name is appended.
pub fn resolve_workspace_path(
    workspace_root: &Path,
    file_path: &str,
) -> Result<PathBuf, ToolResult> {
    // Reject absolute file_paths that bypass join
    if Path::new(file_path).is_absolute() {
        return Err(ToolResult::SystemError {
            message: format!("Security Error: Absolute paths are not allowed: {file_path}"),
        });
    }

    let resolved = workspace_root.join(file_path);

    // Canonicalize workspace root
    let canonical_root =
        std::fs::canonicalize(workspace_root).map_err(|e| ToolResult::SystemError {
            message: format!("Error resolving workspace root: {e}"),
        })?;

    // For path verification, canonicalize the resolved path.
    // If the file doesn't exist yet, canonicalize its parent directory.
    // If the parent also doesn't exist, fall back to string-level verification.
    let canonical_resolved = if let Ok(p) = std::fs::canonicalize(&resolved) {
        p
    } else {
        // File doesn't exist yet — try to canonicalize parent and append filename
        let resolved_parent = resolved.parent();
        match resolved_parent {
            Some(parent) if parent.as_os_str().is_empty() => {
                // Edge case: file_path was just a filename with no directory component
                let file_name = resolved.file_name().unwrap_or_default();
                canonical_root.join(file_name)
            }
            Some(parent) => {
                match std::fs::canonicalize(parent) {
                    Ok(canonical_parent) => {
                        let file_name = resolved.file_name().unwrap_or_default();
                        canonical_parent.join(file_name)
                    }
                    Err(_) => {
                        // Parent doesn't exist either — fall back to lexical
                        // normalization. We must resolve `.` and `..` components
                        // before the starts_with check; otherwise a path like
                        // `foo/../../../etc/passwd` would pass because its prefix
                        // components match the workspace root.
                        let joined = normalize_path(&canonical_root.join(file_path));
                        if !joined.starts_with(&canonical_root) {
                            return Err(ToolResult::SystemError {
                                message: format!(
                                    "Security Error: Path outside workspace: {file_path}"
                                ),
                            });
                        }
                        joined
                    }
                }
            }
            None => {
                return Err(ToolResult::SystemError {
                    message: format!("Error resolving path {file_path}: no parent directory"),
                })
            }
        }
    };
    // Verify resolved starts with root (for the canonicalized path case)
    if !canonical_resolved.starts_with(&canonical_root) {
        return Err(ToolResult::SystemError {
            message: format!("Security Error: Path outside workspace: {file_path}"),
        });
    }

    Ok(canonical_resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn temp_dir() -> std::path::PathBuf {
        static COUNTER: OnceLock<std::sync::atomic::AtomicU32> = OnceLock::new();
        let c = COUNTER.get_or_init(|| std::sync::atomic::AtomicU32::new(0));
        let id = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hackpi_path_jail_test_{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_relative_path_resolves_successfully() {
        let dir = temp_dir();
        std::fs::write(dir.join("existing.txt"), b"hello").unwrap();

        let result = resolve_workspace_path(&dir, "existing.txt");
        assert!(result.is_ok(), "relative path should resolve: {:?}", result);
        let resolved = result.unwrap();
        assert!(resolved.ends_with("existing.txt"));
        assert!(resolved.starts_with(std::fs::canonicalize(&dir).unwrap()));
    }

    #[test]
    fn test_absolute_path_is_rejected() {
        let dir = temp_dir();

        let result = resolve_workspace_path(&dir, "/etc/passwd");
        assert!(result.is_err(), "absolute path should be rejected");
        match result {
            Err(ToolResult::SystemError { message }) => {
                assert!(
                    message.contains("Absolute path"),
                    "error should mention absolute path, got: {message}"
                );
            }
            _ => panic!("expected SystemError"),
        }
    }

    #[test]
    fn test_path_traversal_above_workspace_is_rejected() {
        let dir = temp_dir();

        // Create a subdirectory so we can test traversal from it
        let subdir = dir.join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        // The workspace_root is dir, so ".." should go outside
        let result = resolve_workspace_path(&dir, "../outside.txt");
        assert!(
            result.is_err(),
            "path traversal above workspace should be rejected"
        );
        match result {
            Err(ToolResult::SystemError { message }) => {
                assert!(
                    message.contains("outside workspace") || message.contains("outside workspace"),
                    "error should mention workspace boundary, got: {message}"
                );
            }
            _ => panic!("expected SystemError"),
        }
    }

    #[test]
    fn test_non_existent_path_in_workspace_resolves() {
        let dir = temp_dir();

        // File doesn't exist yet, but parent dir exists
        let result = resolve_workspace_path(&dir, "new_file.txt");
        assert!(
            result.is_ok(),
            "non-existent file in workspace should resolve: {:?}",
            result
        );
        let resolved = result.unwrap();
        assert!(resolved.ends_with("new_file.txt"));
    }

    #[test]
    fn test_non_existent_path_in_subdir_resolves() {
        let dir = temp_dir();
        std::fs::create_dir(dir.join("subdir")).unwrap();

        // File in existing subdirectory, but file doesn't exist
        let result = resolve_workspace_path(&dir, "subdir/new_file.txt");
        assert!(
            result.is_ok(),
            "non-existent file in subdirectory should resolve: {:?}",
            result
        );
        let resolved = result.unwrap();
        assert!(resolved.ends_with("new_file.txt"));
    }

    #[test]
    fn test_symlink_outside_workspace_is_rejected() {
        let dir = temp_dir();
        let outside_dir = std::env::temp_dir().join("hackpi_outside_link_target");
        let _ = std::fs::remove_dir_all(&outside_dir);
        std::fs::create_dir(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("evil.txt"), b"evil").unwrap();

        // Create a symlink inside workspace pointing outside
        let link_path = dir.join("link_to_outside");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside_dir, &link_path).unwrap();
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(&outside_dir, &link_path).unwrap();
        }

        let result = resolve_workspace_path(&dir, "link_to_outside/evil.txt");
        assert!(
            result.is_err(),
            "symlink outside workspace should be rejected"
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(&outside_dir);
    }

    #[test]
    fn test_nonexistent_parent_dir_resolves_via_fallback() {
        let dir = temp_dir();

        // Both parent and file don't exist — should now resolve via string-level fallback
        let result = resolve_workspace_path(&dir, "nonexistent_dir/file.txt");
        assert!(
            result.is_ok(),
            "non-existent parent dir should resolve via fallback: {:?}",
            result
        );
        let resolved = result.unwrap();
        assert!(resolved.ends_with("file.txt"));
        assert!(
            resolved.starts_with(std::fs::canonicalize(&dir).unwrap()),
            "resolved path should be within workspace root"
        );
    }

    #[test]
    fn test_nonexistent_nested_parent_dir_resolves_via_fallback() {
        let dir = temp_dir();

        // Deeply nested parent dirs that don't exist
        let result = resolve_workspace_path(&dir, "a/b/c/d/file.txt");
        assert!(
            result.is_ok(),
            "nested non-existent parent dirs should resolve via fallback: {:?}",
            result
        );
        let resolved = result.unwrap();
        assert!(resolved.ends_with("file.txt"));
        assert!(
            resolved.starts_with(std::fs::canonicalize(&dir).unwrap()),
            "resolved path should be within workspace root"
        );
    }

    // ── COR-19 regression tests ──────────────────────────────────────
    // These verify that directory-traversal attacks through non-existent
    // intermediate directories are blocked by the normalize_path fallback.

    #[test]
    fn test_traversal_through_nonexistent_dir_is_rejected() {
        let dir = temp_dir();

        // "foo" doesn't exist, so the parent canonicalize fails and we
        // fall through to the lexical-normalization fallback. The `..`
        // components escape the workspace root.
        let result = resolve_workspace_path(&dir, "foo/../../../etc/passwd");
        assert!(
            result.is_err(),
            "traversal through nonexistent dir must be rejected, got: {:?}",
            result
        );
        match result {
            Err(ToolResult::SystemError { message }) => {
                assert!(
                    message.contains("outside workspace"),
                    "error should mention workspace boundary, got: {message}"
                );
            }
            _ => panic!("expected SystemError"),
        }
    }

    #[test]
    fn test_traversal_with_dots_only_is_rejected() {
        let dir = temp_dir();

        let result = resolve_workspace_path(&dir, "../../../etc/shadow");
        assert!(
            result.is_err(),
            "bare traversal must be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn test_traversal_with_mixed_existing_and_nonexistent() {
        let dir = temp_dir();
        // Create "real_dir" so one level exists, then traverse through it
        std::fs::create_dir(dir.join("real_dir")).unwrap();

        let result = resolve_workspace_path(&dir, "real_dir/../../etc/passwd");
        assert!(
            result.is_err(),
            "traversal via existing dir must be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn test_traversal_with_deep_nonexistent_chain() {
        let dir = temp_dir();

        let result = resolve_workspace_path(&dir, "a/b/c/../../../../../../etc/passwd");
        assert!(
            result.is_err(),
            "deep traversal through nonexistent dirs must be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn test_dot_component_is_stripped_in_fallback() {
        let dir = temp_dir();

        // "./file.txt" should resolve normally even though parent doesn't
        // need canonicalization
        let result = resolve_workspace_path(&dir, "./file.txt");
        assert!(
            result.is_ok(),
            "'.' components should be resolved: {:?}",
            result
        );
        let resolved = result.unwrap();
        assert!(resolved.ends_with("file.txt"));
        assert!(
            resolved.starts_with(std::fs::canonicalize(&dir).unwrap()),
            "resolved path should be within workspace root"
        );
    }

    #[test]
    fn test_normalize_path_resolves_dotdot() {
        let cases = vec![
            ("/a/b/../c", "/a/c"),
            ("/a/b/../../c", "/c"),
            ("/a/./b", "/a/b"),
            ("/a/b/c/../../d", "/a/d"),
            ("/../etc/passwd", "/../etc/passwd"), // can't go above root; .. is preserved
        ];
        for (input, expected) in cases {
            let normalized = normalize_path(Path::new(input));
            assert_eq!(
                normalized,
                PathBuf::from(expected),
                "normalize_path({input:?}) = {:?}, expected {expected:?}",
                normalized
            );
        }
    }
}
