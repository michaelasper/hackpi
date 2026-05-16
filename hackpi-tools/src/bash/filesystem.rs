use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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
    pub children: BTreeMap<String, FileNode>,
    pub created: SystemTime,
    pub modified: SystemTime,
}

pub struct InMemoryFs {
    root: Mutex<FileNode>,
}

impl Default for InMemoryFs {
    fn default() -> Self {
        let mut root = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let mut home = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let tmp = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let dev_null = FileNode {
            content: Vec::new(),
            mode: 0o644,
            is_dir: false,
            is_symlink: false,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let user = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };
        home.children.insert("user".into(), user);
        root.children.insert("home".into(), home);
        root.children.insert("tmp".into(), tmp);

        let mut dev = FileNode {
            content: Vec::new(),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };
        dev.children.insert("null".into(), dev_null);
        root.children.insert("dev".into(), dev);

        InMemoryFs {
            root: Mutex::new(root),
        }
    }
}

pub(crate) fn resolve_path_mut<'a>(node: &'a mut FileNode, path: &Path) -> Option<&'a mut FileNode> {
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
        current = current.children.get_mut(seg)?;
    }
    Some(current)
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

impl InMemoryFs {
    fn ensure_parents(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().unwrap_or(Path::new("/"));
        self.create_dir(parent, true)
    }
}

impl FileSystem for InMemoryFs {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        let mut root_guard = self.root.lock().unwrap();
        let root = &mut *root_guard;
        if let Some(node) = resolve_path_mut(root, path) {
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
        let mut guard = self.root.lock().unwrap();
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
                    segments.pop();
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
        let mut guard = self.root.lock().unwrap();
        let root = &mut *guard;
        let components: Vec<_> = path.components().collect();
        let file_name = components
            .last()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");

        let mut current = root;
        for comp in &components[..components.len().saturating_sub(1)] {
            let name = comp.as_os_str().to_str().unwrap_or("");
            if name == "/" || name == "." {
                continue;
            }
            if name == ".." {
                continue;
            }
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
        let root = self.root.lock().unwrap();
        resolve_path_ref(&root, path).is_some()
    }

    fn is_dir(&self, path: &Path) -> bool {
        let root = self.root.lock().unwrap();
        resolve_path_ref(&root, path)
            .map(|n| n.is_dir)
            .unwrap_or(false)
    }

    fn is_file(&self, path: &Path) -> bool {
        let root = self.root.lock().unwrap();
        resolve_path_ref(&root, path)
            .map(|n| !n.is_dir)
            .unwrap_or(false)
    }

    fn read_dir(&self, path: &Path) -> std::io::Result<Vec<DirEntry>> {
        let root = self.root.lock().unwrap();
        let node = resolve_path_ref(&root, path)
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
        let mut guard = self.root.lock().unwrap();
        let root = &mut *guard;
        let components: Vec<_> = path.components().collect();

        let mut current = root;
        for comp in &components {
            let name = comp.as_os_str().to_str().unwrap_or("");
            if name.is_empty() || name == "/" || name == "." {
                continue;
            }
            if name == ".." {
                continue;
            }

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
        let root = self.root.lock().unwrap();
        let node = resolve_path_ref(&root, path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))?;
        Ok(FileMeta {
            size: node.content.len() as u64,
            is_dir: node.is_dir,
            is_symlink: node.is_symlink,
            created: node.created,
            modified: node.modified,
        })
    }

    fn symlink(&self, _target: &Path, _link: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "symlinks not supported in InMemoryFs",
        ))
    }

    fn read_link(&self, _path: &Path) -> std::io::Result<PathBuf> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "symlinks not supported in InMemoryFs",
        ))
    }
}
