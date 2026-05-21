pub mod memory;
pub mod path;
pub mod traits;

pub use memory::InMemoryFs;
pub use traits::{DirEntry, FileMeta, FileSystem};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use memory::FileNode;
use path::{resolve_path_follow, resolve_path_ref};

impl FileSystem for InMemoryFs {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        let root_guard = self.root.read().unwrap_or_else(|e| e.into_inner());
        if let Some(node) = resolve_path_follow(&root_guard, path) {
            if node.is_dir {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::IsADirectory,
                    "Is a directory",
                ));
            }
            Ok((*node.content).clone())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File not found",
            ))
        }
    }

    fn write(&self, path: &Path, content: &[u8]) -> std::io::Result<()> {
        self.ensure_parents(path)?;
        let mut guard = self.root.write().unwrap_or_else(|e| e.into_inner());
        let root = &mut *guard;
        let components: Vec<_> = path.components().collect();
        let file_name = components
            .last()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");

        let mut segments: Vec<&str> = Vec::new();
        for comp in &components[..components.len().saturating_sub(1)] {
            let name = comp.as_os_str().to_str().unwrap_or("");
            match name {
                "/" | "." => continue,
                ".." => {
                    if !segments.is_empty() {
                        segments.pop();
                    }
                }
                _ => segments.push(name),
            }
        }

        let mut current = root;
        for seg in &segments {
            if !current.children.contains_key(*seg) {
                current.children.insert(
                    seg.to_string(),
                    FileNode {
                        content: Arc::new(Vec::new()),
                        mode: 0o755,
                        is_dir: true,
                        is_symlink: false,
                        symlink_target: None,
                        children: BTreeMap::new(),
                        created: SystemTime::now(),
                        modified: SystemTime::now(),
                    },
                );
            }
            current = current.children.get_mut(*seg).unwrap();
            if !current.is_dir {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    format!("'{}' is not a directory", seg),
                ));
            }
        }

        // If the target is a symlink, write through it instead of replacing it
        if let Some(existing) = current.children.get(file_name) {
            if existing.is_symlink {
                if let Some(target) = &existing.symlink_target {
                    let resolved = if target.is_relative() {
                        path.parent().unwrap_or(Path::new("/")).join(target)
                    } else {
                        target.clone()
                    };
                    drop(guard);
                    return self.write(&resolved, content);
                }
            }
        }

        current.children.insert(
            file_name.into(),
            FileNode {
                content: Arc::new(content.to_vec()),
                mode: 0o644,
                is_dir: false,
                is_symlink: false,
                symlink_target: None,
                children: BTreeMap::new(),
                created: SystemTime::now(),
                modified: SystemTime::now(),
            },
        );

        Ok(())
    }

    fn append(&self, path: &Path, content: &[u8]) -> std::io::Result<()> {
        let mut existing = match self.read(path) {
            Ok(data) => data,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        existing.extend_from_slice(content);
        self.write(path, &existing)
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        let mut guard = self.root.write().unwrap_or_else(|e| e.into_inner());
        let root = &mut *guard;
        let components: Vec<_> = path.components().collect();

        let mut segments: Vec<&str> = Vec::new();
        for comp in &components {
            let name = comp.as_os_str().to_str().unwrap_or("");
            match name {
                "/" | "." => continue,
                ".." => {
                    segments.pop();
                }
                _ => segments.push(name),
            }
        }

        let file_name = segments.pop().unwrap_or("");
        if file_name.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not found",
            ));
        }

        let mut current = root;
        for &name in &segments {
            current = match current.children.get_mut(name) {
                Some(n) => n,
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "not found",
                    ))
                }
            };
        }

        if current.children.remove(file_name).is_none() {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not found",
            ))
        } else {
            Ok(())
        }
    }

    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        if from == to {
            return Ok(());
        }

        let mut guard = self.root.write().unwrap_or_else(|e| e.into_inner());

        // Helper to normalize path into segments
        let parse_segments = |path: &Path| -> Vec<String> {
            let components: Vec<_> = path.components().collect();
            let mut segments: Vec<String> = Vec::new();
            for comp in &components {
                let name = comp.as_os_str().to_str().unwrap_or("");
                match name {
                    "/" | "." => continue,
                    ".." => {
                        segments.pop();
                    }
                    _ => segments.push(name.to_string()),
                }
            }
            segments
        };

        let from_segs = parse_segments(from);
        let to_segs = parse_segments(to);

        let from_name = from_segs
            .last()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))?;
        let to_name = to_segs
            .last()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))?;

        // Navigate to source parent and remove the node
        let mut current: &mut FileNode = &mut guard;
        for name in &from_segs[..from_segs.len().saturating_sub(1)] {
            current = current
                .children
                .get_mut(name.as_str())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))?;
        }

        let mut node = current
            .children
            .remove(from_name.as_str())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))?;

        // Update modification time
        node.modified = SystemTime::now();

        // Navigate to destination parent, creating directories as needed
        let mut current: &mut FileNode = &mut guard;
        for name in &to_segs[..to_segs.len().saturating_sub(1)] {
            if !current.children.contains_key(name.as_str()) {
                current.children.insert(
                    name.clone(),
                    FileNode {
                        content: Arc::new(Vec::new()),
                        mode: 0o755,
                        is_dir: true,
                        is_symlink: false,
                        symlink_target: None,
                        children: BTreeMap::new(),
                        created: SystemTime::now(),
                        modified: SystemTime::now(),
                    },
                );
            }
            current = current.children.get_mut(name.as_str()).unwrap();
        }

        // Insert the node at the destination
        current.children.insert(to_name.clone(), node);

        Ok(())
    }

    fn copy(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        let content = self.read(from)?;
        self.write(to, &content)
    }

    fn exists(&self, path: &Path) -> bool {
        let root = self.root.read().unwrap_or_else(|e| e.into_inner());
        resolve_path_follow(&root, path).is_some()
    }

    fn is_dir(&self, path: &Path) -> bool {
        let root = self.root.read().unwrap_or_else(|e| e.into_inner());
        resolve_path_follow(&root, path)
            .map(|n| n.is_dir)
            .unwrap_or(false)
    }

    fn is_file(&self, path: &Path) -> bool {
        let root = self.root.read().unwrap_or_else(|e| e.into_inner());
        resolve_path_follow(&root, path)
            .map(|n| !n.is_dir)
            .unwrap_or(false)
    }

    fn read_dir(&self, path: &Path) -> std::io::Result<Vec<DirEntry>> {
        let root = self.root.read().unwrap_or_else(|e| e.into_inner());
        let node = resolve_path_follow(&root, path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))?;

        if !node.is_dir {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "Not a directory",
            ));
        }

        Ok(node
            .children
            .iter()
            .map(|(name, child)| DirEntry {
                name: name.clone(),
                is_dir: child.is_dir,
            })
            .collect())
    }

    fn create_dir(&self, path: &Path, recursive: bool) -> std::io::Result<()> {
        let mut guard = self.root.write().unwrap_or_else(|e| e.into_inner());
        let root = &mut *guard;
        let components: Vec<_> = path.components().collect();

        let mut segments: Vec<&str> = Vec::new();
        for comp in &components {
            let name = comp.as_os_str().to_str().unwrap_or("");
            match name {
                "" | "/" | "." => continue,
                ".." => {
                    segments.pop();
                }
                _ => segments.push(name),
            }
        }

        let mut current = root;
        for &name in &segments {
            if !current.children.contains_key(name) {
                if !recursive {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "parent not found",
                    ));
                }
                current.children.insert(
                    name.into(),
                    FileNode {
                        content: Arc::new(Vec::new()),
                        mode: 0o755,
                        is_dir: true,
                        is_symlink: false,
                        symlink_target: None,
                        children: BTreeMap::new(),
                        created: SystemTime::now(),
                        modified: SystemTime::now(),
                    },
                );
            }
            current = current.children.get_mut(name).unwrap();
            if !current.is_dir {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    format!("'{name}' is not a directory"),
                ));
            }
        }

        Ok(())
    }

    fn remove_dir(&self, path: &Path, recursive: bool) -> std::io::Result<()> {
        if !recursive {
            let children = self.read_dir(path)?;
            if !children.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::DirectoryNotEmpty,
                    "directory not empty",
                ));
            }
        }
        self.remove(path)
    }

    fn metadata(&self, path: &Path) -> std::io::Result<FileMeta> {
        let root = self.root.read().unwrap_or_else(|e| e.into_inner());
        let node = resolve_path_follow(&root, path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))?;
        Ok(FileMeta {
            size: node.content.len() as u64,
            is_dir: node.is_dir,
            is_symlink: node.is_symlink,
            created: node.created,
            modified: node.modified,
        })
    }

    fn symlink(&self, target: &Path, link: &Path) -> std::io::Result<()> {
        let mut guard = self.root.write().unwrap_or_else(|e| e.into_inner());
        let root = &mut *guard;
        let components: Vec<_> = link.components().collect();

        let mut segments: Vec<&str> = Vec::new();
        for comp in &components[..components.len().saturating_sub(1)] {
            let name = comp.as_os_str().to_str().unwrap_or("");
            match name {
                "/" | "." => continue,
                ".." => {
                    if !segments.is_empty() {
                        segments.pop();
                    }
                }
                _ => segments.push(name),
            }
        }

        let file_name = components
            .last()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");

        // Create parent directories if needed
        let mut current = root;
        for seg in &segments {
            if !current.children.contains_key(*seg) {
                current.children.insert(
                    seg.to_string(),
                    FileNode {
                        content: Arc::new(Vec::new()),
                        mode: 0o755,
                        is_dir: true,
                        is_symlink: false,
                        symlink_target: None,
                        children: BTreeMap::new(),
                        created: SystemTime::now(),
                        modified: SystemTime::now(),
                    },
                );
            }
            current = current.children.get_mut(*seg).unwrap();
            if !current.is_dir {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    format!("'{seg}' is not a directory"),
                ));
            }
        }

        current.children.insert(
            file_name.into(),
            FileNode {
                content: Arc::new(Vec::new()),
                mode: 0o644,
                is_dir: false,
                is_symlink: true,
                symlink_target: Some(target.to_path_buf()),
                children: BTreeMap::new(),
                created: SystemTime::now(),
                modified: SystemTime::now(),
            },
        );

        Ok(())
    }

    fn read_link(&self, path: &Path) -> std::io::Result<PathBuf> {
        let root = self.root.read().unwrap_or_else(|e| e.into_inner());
        let node = resolve_path_ref(&root, path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "File not found"))?;
        if !node.is_symlink {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a symlink",
            ));
        }
        node.symlink_target.clone().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "symlink target missing")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;
    use std::path::Path;

    #[test]
    fn test_lock_poison_recovery_write() {
        let fs = InMemoryFs::default();

        // Poison the write lock by panicking while holding it
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = fs.root.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        // With a poisoned lock, .write().unwrap() panics.
        // After the fix (.unwrap_or_else(|e| e.into_inner())), this should recover.
        let result = fs.write(Path::new("/test.txt"), b"hello");
        assert!(result.is_ok());
        let content = fs.read(Path::new("/test.txt")).unwrap();
        assert_eq!(content, b"hello");
    }

    #[test]
    fn test_lock_poison_recovery_read() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/existing.txt"), b"data").unwrap();

        // Poison the write lock
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = fs.root.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        // With a poisoned lock, .read().unwrap() panics.
        // After the fix, this should recover and return the data.
        let result = fs.read(Path::new("/existing.txt"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"data");
    }

    #[test]
    fn test_lock_poison_recovery_exists() {
        let fs = InMemoryFs::default();

        // Poison the write lock
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = fs.root.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        // exists() calls .read().unwrap() — should recover after fix
        let result = fs.exists(Path::new("/"));
        assert!(result);
    }

    #[test]
    fn test_lock_poison_recovery_metadata() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/test.txt"), b"data").unwrap();

        // Poison the write lock
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = fs.root.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        // metadata() calls .read().unwrap() — should recover after fix
        let result = fs.metadata(Path::new("/test.txt"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().size, 4);
    }

    #[test]
    fn test_lock_poison_recovery_create_dir() {
        let fs = InMemoryFs::default();

        // Poison the write lock
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = fs.root.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        // create_dir() calls .write().unwrap() — should recover after fix
        let result = fs.create_dir(Path::new("/newdir"), true);
        assert!(result.is_ok());
        assert!(fs.is_dir(Path::new("/newdir")));
    }

    #[test]
    fn test_lock_poison_recovery_remove() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/test.txt"), b"data").unwrap();

        // Poison the write lock
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _guard = fs.root.write().unwrap();
            panic!("intentional panic to poison lock");
        }));

        // remove() calls .write().unwrap() — should recover after fix
        let result = fs.remove(Path::new("/test.txt"));
        assert!(result.is_ok());
        assert!(!fs.exists(Path::new("/test.txt")));
    }

    // --- COR-26: Append error handling ---

    #[test]
    fn test_append_on_directory_returns_error() {
        let fs = InMemoryFs::default();
        // /tmp exists as a directory — read() returns IsADirectory
        let result = fs.append(Path::new("/tmp"), b"content");
        assert!(result.is_err(), "append on directory should fail");
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::IsADirectory,
            "should propagate IsADirectory error, not silently discard"
        );
    }

    #[test]
    fn test_append_new_file_creates_and_appends() {
        let fs = InMemoryFs::default();
        let result = fs.append(Path::new("/newfile.txt"), b"hello");
        assert!(result.is_ok(), "append to new file should succeed");
        assert_eq!(fs.read(Path::new("/newfile.txt")).unwrap(), b"hello");
    }

    #[test]
    fn test_append_existing_file_appends_content() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/existing.txt"), b"hello").unwrap();
        fs.append(Path::new("/existing.txt"), b" world").unwrap();
        assert_eq!(fs.read(Path::new("/existing.txt")).unwrap(), b"hello world");
    }

    // --- COR-26: Atomic rename ---

    #[test]
    fn test_rename_moves_content_and_removes_source() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/source.txt"), b"hello").unwrap();
        fs.rename(Path::new("/source.txt"), Path::new("/dest.txt"))
            .unwrap();
        assert!(
            !fs.exists(Path::new("/source.txt")),
            "source should not exist after rename"
        );
        assert_eq!(fs.read(Path::new("/dest.txt")).unwrap(), b"hello");
    }

    #[test]
    fn test_rename_nonexistent_source_returns_error() {
        let fs = InMemoryFs::default();
        let result = fs.rename(Path::new("/nonexistent"), Path::new("/dest.txt"));
        assert!(result.is_err(), "rename of nonexistent source should fail");
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_rename_to_existing_overwrites() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/source.txt"), b"hello").unwrap();
        fs.write(Path::new("/dest.txt"), b"world").unwrap();
        fs.rename(Path::new("/source.txt"), Path::new("/dest.txt"))
            .unwrap();
        assert!(
            !fs.exists(Path::new("/source.txt")),
            "source should not exist after rename"
        );
        assert_eq!(
            fs.read(Path::new("/dest.txt")).unwrap(),
            b"hello",
            "dest should contain source content after rename"
        );
    }

    #[test]
    fn test_rename_within_directory_preserves_metadata() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/a.txt"), b"data").unwrap();
        let meta_before = fs.metadata(Path::new("/a.txt")).unwrap();
        fs.rename(Path::new("/a.txt"), Path::new("/b.txt")).unwrap();
        let meta_after = fs.metadata(Path::new("/b.txt")).unwrap();
        assert_eq!(
            meta_after.size, meta_before.size,
            "file size should be preserved after rename"
        );
    }

    // --- COR-25: Arc-optimized reads ---

    #[test]
    fn test_arc_read_returns_correct_content() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/arc_test.txt"), b"hello arc").unwrap();
        let content = fs.read(Path::new("/arc_test.txt")).unwrap();
        assert_eq!(content, b"hello arc");
    }

    #[test]
    fn test_arc_read_large_content() {
        let fs = InMemoryFs::default();
        let large = vec![b'A'; 65536];
        fs.write(Path::new("/large.txt"), &large).unwrap();
        let content = fs.read(Path::new("/large.txt")).unwrap();
        assert_eq!(content.len(), 65536);
        assert_eq!(content, large);
    }

    #[test]
    fn test_arc_overwrite_changes_content() {
        let fs = InMemoryFs::default();
        fs.write(Path::new("/overwrite.txt"), b"first").unwrap();
        assert_eq!(fs.read(Path::new("/overwrite.txt")).unwrap(), b"first");
        fs.write(Path::new("/overwrite.txt"), b"second").unwrap();
        assert_eq!(fs.read(Path::new("/overwrite.txt")).unwrap(), b"second");
    }

    #[test]
    fn test_arc_many_reads_produce_consistent_data() {
        let fs = InMemoryFs::default();
        let data = b"The quick brown fox jumps over the lazy dog";
        fs.write(Path::new("/fox.txt"), data).unwrap();

        // Read many times concurrently
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..20 {
                handles.push(s.spawn(|| {
                    for _ in 0..50 {
                        let content = fs.read(Path::new("/fox.txt")).unwrap();
                        assert_eq!(content, data);
                    }
                }));
            }
        });

        // Verify still correct after concurrent reads
        assert_eq!(fs.read(Path::new("/fox.txt")).unwrap(), data);
    }
}
