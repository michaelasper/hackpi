use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::SystemTime;

use super::traits::FileSystem;

#[derive(Debug, Clone)]
pub(crate) struct FileNode {
    pub content: Arc<Vec<u8>>,
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
    pub(crate) root: RwLock<FileNode>,
}

/// Create a minimal root filesystem with `/tmp` and `/dev/null`.
fn create_root() -> FileNode {
    let tmp = FileNode {
        content: Arc::new(Vec::new()),
        mode: 0o755,
        is_dir: true,
        is_symlink: false,
        symlink_target: None,
        children: BTreeMap::new(),
        created: SystemTime::now(),
        modified: SystemTime::now(),
    };

    let dev_null = FileNode {
        content: Arc::new(Vec::new()),
        mode: 0o644,
        is_dir: false,
        is_symlink: false,
        symlink_target: None,
        children: BTreeMap::new(),
        created: SystemTime::now(),
        modified: SystemTime::now(),
    };

    let mut root = FileNode {
        content: Arc::new(Vec::new()),
        mode: 0o755,
        is_dir: true,
        is_symlink: false,
        symlink_target: None,
        children: BTreeMap::new(),
        created: SystemTime::now(),
        modified: SystemTime::now(),
    };
    root.children.insert("tmp".into(), tmp);

    let mut dev = FileNode {
        content: Arc::new(Vec::new()),
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

    root
}

impl Default for InMemoryFs {
    fn default() -> Self {
        InMemoryFs {
            root: RwLock::new(create_root()),
        }
    }
}

impl InMemoryFs {
    /// Create an InMemoryFs with a home directory rooted at the given path.
    ///
    /// Creates `/home/user` and `~/.bashrc` under the virtual filesystem
    /// rooted at the workspace root, so tools like `cd ~` resolve correctly.
    pub fn with_home(workspace_root: &Path) -> Self {
        let mut root = create_root();

        let bashrc = FileNode {
            content: Arc::new(b"# ~/.bashrc - default bash configuration\n".to_vec()),
            mode: 0o644,
            is_dir: false,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let mut user_home = FileNode {
            content: Arc::new(Vec::new()),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };
        user_home.children.insert(".bashrc".into(), bashrc);

        // Create /home/user under the virtual root
        let mut home = FileNode {
            content: Arc::new(Vec::new()),
            mode: 0o755,
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };
        home.children.insert("user".into(), user_home);
        root.children.insert("home".into(), home);

        // Also create the workspace root directory in the virtual fs
        let mut current = &mut root;
        for component in workspace_root.iter() {
            let name = component.to_str().unwrap_or("");
            if name.is_empty() || name == "/" {
                continue;
            }
            if !current.children.contains_key(name) {
                current.children.insert(
                    name.to_string(),
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
        }

        InMemoryFs {
            root: RwLock::new(root),
        }
    }

    /// Ensure parent directories exist for a given path.
    pub(crate) fn ensure_parents(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().unwrap_or(Path::new("/"));
        self.create_dir(parent, true)
    }
}
