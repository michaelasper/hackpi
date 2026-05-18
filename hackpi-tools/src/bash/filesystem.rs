use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FileMeta {
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub created: SystemTime,
    pub modified: SystemTime,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

pub trait FileSystem: Send {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>>;
    fn write(&self, path: &Path, content: &[u8]) -> std::io::Result<()>;
    fn append(&self, path: &Path, content: &[u8]) -> std::io::Result<()>;
    fn remove(&self, path: &Path) -> std::io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn copy(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    fn read_dir(&self, path: &Path) -> std::io::Result<Vec<DirEntry>>;
    fn create_dir(&self, path: &Path, recursive: bool) -> std::io::Result<()>;
    fn remove_dir(&self, path: &Path, recursive: bool) -> std::io::Result<()>;
    fn metadata(&self, path: &Path) -> std::io::Result<FileMeta>;
    fn symlink(&self, _target: &Path, _link: &Path) -> std::io::Result<()>;
    fn read_link(&self, _path: &Path) -> std::io::Result<PathBuf>;
}

#[derive(Debug, Clone)]
pub(crate) struct FileNode {
    pub content: Vec<u8>,
    #[allow(dead_code)]
    pub mode: u32,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<PathBuf>,
    pub children: BTreeMap<String, FileNode>,
    pub created: SystemTime,
    pub modified: SystemTime,
}

pub struct InMemoryFs {
    root: RwLock<FileNode>,
}

impl Default for InMemoryFs {
    fn default() -> Self {
        let mut root = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let mut home = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let tmp = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let dev_null = FileNode {
            content: Vec::new(),
            mode: 0o644,
            is_dir: false,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let bashrc = FileNode {
            content: b"# ~/.bashrc - default bash configuration\n".to_vec(),
            mode: 0o644,
            is_dir: false,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let mut user = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };
        user.children.insert(".bashrc".into(), bashrc);
        home.children.insert("user".into(), user);
        root.children.insert("home".into(), home);
        root.children.insert("tmp".into(), tmp);

        let mut dev = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };
        dev.children.insert("null".into(), dev_null);
        root.children.insert("dev".into(), dev);

        InMemoryFs {
            root: RwLock::new(root),
        }
    }
}

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
fn resolve_path_follow<'a>(root: &'a FileNode, path: &Path) -> Option<&'a FileNode> {
    let node = resolve_path_ref(root, path)?;
    follow_symlinks(root, node, path)
}

/// Follow symlink chain starting from `node` up to 10 levels deep.
fn follow_symlinks<'a>(
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

impl InMemoryFs {
    fn ensure_parents(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().unwrap_or(Path::new("/"));
        self.create_dir(parent, true)
    }
}

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
            Ok(node.content.clone())
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
                        content: Vec::new(),
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
        }

        current.children.insert(
            file_name.into(),
            FileNode {
                content: content.to_vec(),
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
        let mut existing = self.read(path).unwrap_or_default();
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
        let content = self.read(from)?;
        self.write(to, &content)?;
        self.remove(from)?;
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
                        content: Vec::new(),
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
                        content: Vec::new(),
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
        }

        current.children.insert(
            file_name.into(),
            FileNode {
                content: Vec::new(),
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
}
