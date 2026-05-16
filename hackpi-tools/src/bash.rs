use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

// ── FileSystem trait ──────────────────────────────────────────

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

// ── InMemoryFs ────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct FileNode {
    content: Vec<u8>,
    is_dir: bool,
    is_symlink: bool,
    symlink_target: Option<PathBuf>,
    children: BTreeMap<String, FileNode>,
    created: SystemTime,
    modified: SystemTime,
}

pub struct InMemoryFs {
    root: Mutex<FileNode>,
}

impl Default for InMemoryFs {
    fn default() -> Self {
        let mut root = FileNode {
            content: Vec::new(),
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let home = FileNode {
            content: Vec::new(),
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let tmp = FileNode {
            content: Vec::new(),
            is_dir: true,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        let dev_null = FileNode {
            content: Vec::new(),
            is_dir: false,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        };

        root.children.insert("home".into(), home);
        root.children.insert("tmp".into(), tmp);

        let mut dev = FileNode {
            content: Vec::new(),
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
            root: Mutex::new(root),
        }
    }
}

fn resolve_path<'a>(node: &'a mut FileNode, path: &Path) -> Option<&'a mut FileNode> {
    let components: Vec<_> = path.components().collect();
    let mut current = node;
    for comp in components {
        let name = comp.as_os_str().to_str()?;
        if name == "/" || name == "." {
            continue;
        }
        if name == ".." {
            continue;
        }
        current = current.children.get_mut(name)?;
    }
    Some(current)
}

fn resolve_path_ref<'a>(node: &'a FileNode, path: &Path) -> Option<&'a FileNode> {
    let components: Vec<_> = path.components().collect();
    let mut current = node;
    for comp in components {
        let name = comp.as_os_str().to_str()?;
        if name == "/" || name == "." {
            continue;
        }
        if name == ".." {
            continue;
        }
        current = current.children.get(name)?;
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
        if let Some(node) = resolve_path(root, path) {
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
        let file_name = components.last().and_then(|c| c.as_os_str().to_str()).unwrap_or("");

        let mut current = root;
        for comp in &components[..components.len().saturating_sub(1)] {
            let name = comp.as_os_str().to_str().unwrap_or("");
            if name == "/" || name == "." { continue; }
            if name == ".." { continue; }
            if !current.children.contains_key(name) {
                current.children.insert(name.into(), FileNode {
                    content: Vec::new(),
                    is_dir: true,
                    is_symlink: false,
                    symlink_target: None,
                    children: BTreeMap::new(),
                    created: SystemTime::now(),
                    modified: SystemTime::now(),
                });
            }
            current = current.children.get_mut(name).unwrap();
        }

        current.children.insert(file_name.into(), FileNode {
            content: content.to_vec(),
            is_dir: false,
            is_symlink: false,
            symlink_target: None,
            children: BTreeMap::new(),
            created: SystemTime::now(),
            modified: SystemTime::now(),
        });

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
        let file_name = components.last().and_then(|c| c.as_os_str().to_str()).unwrap_or("");

        let mut current = root;
        for comp in &components[..components.len().saturating_sub(1)] {
            let name = comp.as_os_str().to_str().unwrap_or("");
            if name == "/" || name == "." { continue; }
            if name == ".." { continue; }
            current = match current.children.get_mut(name) {
                Some(n) => n,
                None => return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not found")),
            };
        }

        if current.children.remove(file_name).is_none() {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not found"))
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
        resolve_path_ref(&root, path).map(|n| n.is_dir).unwrap_or(false)
    }

    fn is_file(&self, path: &Path) -> bool {
        let root = self.root.lock().unwrap();
        resolve_path_ref(&root, path).map(|n| !n.is_dir).unwrap_or(false)
    }

    fn read_dir(&self, path: &Path) -> std::io::Result<Vec<DirEntry>> {
        let root = self.root.lock().unwrap();
        let node = resolve_path_ref(&root, path).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "not found")
        })?;

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
            if name.is_empty() || name == "/" || name == "." { continue; }
            if name == ".." { continue; }

            if !current.children.contains_key(name) {
                if !recursive {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "parent not found",
                    ));
                }
                current.children.insert(name.into(), FileNode {
                    content: Vec::new(),
                    is_dir: true,
                    is_symlink: false,
                    symlink_target: None,
                    children: BTreeMap::new(),
                    created: SystemTime::now(),
                    modified: SystemTime::now(),
                });
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
        let node = resolve_path_ref(&root, path).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "not found")
        })?;
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

// ── Command Registry ──────────────────────────────────────────

pub struct CommandContext<'a> {
    pub fs: &'a dyn FileSystem,
    pub env: &'a mut HashMap<String, String>,
    pub cwd: &'a mut PathBuf,
    pub stdin: Option<String>,
    pub stdout: &'a mut Vec<u8>,
    pub stderr: &'a mut Vec<u8>,
    pub cancelled: bool,
}

pub type CommandFn = for<'a> fn(args: &[String], ctx: &mut CommandContext<'a>) -> i32;

pub struct CommandRegistry {
    commands: HashMap<String, CommandFn>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut cmds: HashMap<String, CommandFn> = HashMap::new();
        cmds.insert("cd".to_string(), cmd_cd);
        cmds.insert("pwd".to_string(), cmd_pwd);
        cmds.insert("echo".to_string(), cmd_echo);
        cmds.insert("ls".to_string(), cmd_ls);
        cmds.insert("cat".to_string(), cmd_cat);
        cmds.insert("cp".to_string(), cmd_cp);
        cmds.insert("mv".to_string(), cmd_mv);
        cmds.insert("rm".to_string(), cmd_rm);
        cmds.insert("mkdir".to_string(), cmd_mkdir);
        cmds.insert("touch".to_string(), cmd_touch);
        cmds.insert("grep".to_string(), cmd_grep);
        cmds.insert("head".to_string(), cmd_head);
        cmds.insert("tail".to_string(), cmd_tail);
        cmds.insert("wc".to_string(), cmd_wc);
        cmds.insert("sort".to_string(), cmd_sort);
        cmds.insert("cut".to_string(), cmd_cut);
        cmds.insert("tr".to_string(), cmd_tr);
        cmds.insert("uniq".to_string(), cmd_uniq);
        cmds.insert("env".to_string(), cmd_env);
        cmds.insert("export".to_string(), cmd_export);

        Self { commands: cmds }
    }

    pub fn execute(&self, name: &str, args: &[String], ctx: &mut CommandContext) -> i32 {
        match self.commands.get(name) {
            Some(f) => f(args, ctx),
            None => {
                let _ = writeln!(ctx.stderr, "bash: {name}: command not found");
                127
            }
        }
    }
}

use std::io::Write as IoWrite;

fn cmd_cd(args: &[String], ctx: &mut CommandContext) -> i32 {
    let target = args.first().map(|s| s.as_str()).unwrap_or("~");
    let new_cwd = if target == "~" {
        ctx.env.get("HOME").cloned().unwrap_or_else(|| "/home/user".into())
    } else {
        let base = ctx.cwd.clone();
        let path = base.join(target);
        path.to_string_lossy().to_string()
    };

    if ctx.fs.is_dir(Path::new(&new_cwd)) {
        *ctx.cwd = PathBuf::from(&new_cwd);
        0
    } else {
        let _ = writeln!(ctx.stderr, "cd: {target}: No such directory");
        1
    }
}

fn cmd_pwd(_args: &[String], ctx: &mut CommandContext) -> i32 {
    let _ = writeln!(ctx.stdout, "{}", ctx.cwd.display());
    0
}

fn cmd_echo(args: &[String], ctx: &mut CommandContext) -> i32 {
    let no_newline = args.first().map(|s| s.as_str()) == Some("-n");
    let start = if no_newline { 1 } else { 0 };
    let text = args[start..].join(" ");
    if no_newline {
        let _ = write!(ctx.stdout, "{text}");
    } else {
        let _ = writeln!(ctx.stdout, "{text}");
    }
    0
}

fn cmd_ls(args: &[String], ctx: &mut CommandContext) -> i32 {
    let long = args.contains(&"-l".to_string()) || args.contains(&"-la".to_string());
    let all = args.contains(&"-a".to_string()) || args.contains(&"-la".to_string());

    let targets: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let dirs = if targets.is_empty() {
        vec![ctx.cwd.clone()]
    } else {
        targets.iter().map(|t| ctx.cwd.join(t)).collect()
    };

    for dir in &dirs {
        if dirs.len() > 1 {
            let _ = writeln!(ctx.stdout, "{}:", dir.display());
        }

        match ctx.fs.read_dir(dir) {
            Ok(entries) => {
                let mut entries: Vec<_> = entries
                    .into_iter()
                    .filter(|e| all || !e.name.starts_with('.'))
                    .collect();
                entries.sort_by(|a, b| a.name.cmp(&b.name));

                for entry in &entries {
                    if long {
                        let meta = ctx.fs.metadata(&dir.join(&entry.name)).ok();
                        let size = meta.map(|m| m.size).unwrap_or(0);
                        let mode = if entry.is_dir { "drwxr-xr-x" } else { "-rw-r--r--" };
                        let _ = writeln!(ctx.stdout, "{mode}  {size:>8}  {}", entry.name);
                    } else {
                        if entry.is_dir {
                            let _ = write!(ctx.stdout, "{}/  ", entry.name);
                        } else {
                            let _ = write!(ctx.stdout, "{}  ", entry.name);
                        }
                    }
                }
                if !long {
                    let _ = writeln!(ctx.stdout);
                }
            }
            Err(_) => {
                let _ = writeln!(ctx.stderr, "ls: cannot access '{}': No such file or directory", dir.display());
                return 1;
            }
        }
    }
    0
}

fn cmd_cat(args: &[String], ctx: &mut CommandContext) -> i32 {
    if args.is_empty() {
        if let Some(stdin) = &ctx.stdin {
            let _ = write!(ctx.stdout, "{stdin}");
        }
        return 0;
    }

    for arg in args {
        let path = ctx.cwd.join(arg);
        match ctx.fs.read(&path) {
            Ok(content) => {
                let _ = write!(ctx.stdout, "{}", String::from_utf8_lossy(&content));
            }
            Err(_) => {
                let _ = writeln!(ctx.stderr, "cat: {arg}: No such file or directory");
                return 1;
            }
        }
    }
    0
}

fn cmd_cp(args: &[String], ctx: &mut CommandContext) -> i32 {
    if args.len() < 2 {
        let _ = writeln!(ctx.stderr, "cp: missing file operand");
        return 1;
    }
    let src = ctx.cwd.join(&args[0]);
    let dst = ctx.cwd.join(&args[1]);
    match ctx.fs.copy(&src, &dst) {
        Ok(_) => 0,
        Err(e) => {
            let _ = writeln!(ctx.stderr, "cp: {e}");
            1
        }
    }
}

fn cmd_mv(args: &[String], ctx: &mut CommandContext) -> i32 {
    if args.len() < 2 {
        let _ = writeln!(ctx.stderr, "mv: missing file operand");
        return 1;
    }
    let src = ctx.cwd.join(&args[0]);
    let dst = ctx.cwd.join(&args[1]);
    match ctx.fs.rename(&src, &dst) {
        Ok(_) => 0,
        Err(e) => {
            let _ = writeln!(ctx.stderr, "mv: {e}");
            1
        }
    }
}

fn cmd_rm(args: &[String], ctx: &mut CommandContext) -> i32 {
    let recursive = args.contains(&"-rf".to_string()) || args.contains(&"-r".to_string());
    let targets: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();

    for target in targets {
        let path = ctx.cwd.join(target);
        if ctx.fs.is_dir(&path) {
            if recursive {
                if let Err(e) = ctx.fs.remove_dir(&path, true) {
                    let _ = writeln!(ctx.stderr, "rm: {e}");
                    return 1;
                }
            } else {
                let _ = writeln!(ctx.stderr, "rm: {target}: is a directory");
                return 1;
            }
        } else {
            if let Err(e) = ctx.fs.remove(&path) {
                let _ = writeln!(ctx.stderr, "rm: {e}");
                return 1;
            }
        }
    }
    0
}

fn cmd_mkdir(args: &[String], ctx: &mut CommandContext) -> i32 {
    let parents = args.contains(&"-p".to_string());
    let targets: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();

    for target in targets {
        let path = ctx.cwd.join(target);
        if let Err(e) = ctx.fs.create_dir(&path, parents) {
            let _ = writeln!(ctx.stderr, "mkdir: {e}");
            return 1;
        }
    }
    0
}

fn cmd_touch(args: &[String], ctx: &mut CommandContext) -> i32 {
    for arg in args {
        let path = ctx.cwd.join(arg);
        if !ctx.fs.exists(&path) {
            if let Err(e) = ctx.fs.write(&path, &[]) {
                let _ = writeln!(ctx.stderr, "touch: {e}");
                return 1;
            }
        }
    }
    0
}

fn cmd_grep(args: &[String], ctx: &mut CommandContext) -> i32 {
    let ignore_case = args.contains(&"-i".to_string());
    let targets: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();

    if targets.is_empty() {
        let _ = writeln!(ctx.stderr, "grep: missing pattern");
        return 1;
    }

    let pattern = targets[0];
    let files = &targets[1..];

    let content = if files.is_empty() {
        ctx.stdin.clone().unwrap_or_default()
    } else {
        let mut all = String::new();
        for file in files {
            let path = ctx.cwd.join(file);
            if let Ok(data) = ctx.fs.read(&path) {
                all.push_str(&String::from_utf8_lossy(&data));
            }
        }
        all
    };

    for (i, line) in content.lines().enumerate() {
        let matches = if ignore_case {
            line.to_lowercase().contains(&pattern.to_lowercase())
        } else {
            line.contains(pattern)
        };
        if matches {
            if files.len() > 1 {
                let _ = writeln!(ctx.stdout, "{}:{}:{}", args[1], i + 1, line);
            } else {
                let _ = writeln!(ctx.stdout, "{line}");
            }
        }
    }
    0
}

fn cmd_head(args: &[String], ctx: &mut CommandContext) -> i32 {
    let mut n = 10;
    let mut file_idx = 0;

    if args.first().map(|s| s.as_str()) == Some("-n") {
        if let Some(num) = args.get(1) {
            n = num.parse().unwrap_or(10);
            file_idx = 2;
        }
    }

    let content = if let Some(file) = args.get(file_idx) {
        let path = ctx.cwd.join(file);
        match ctx.fs.read(&path) {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => {
                let _ = writeln!(ctx.stderr, "head: {file}: No such file");
                return 1;
            }
        }
    } else {
        ctx.stdin.clone().unwrap_or_default()
    };

    for line in content.lines().take(n) {
        let _ = writeln!(ctx.stdout, "{line}");
    }
    0
}

fn cmd_tail(args: &[String], ctx: &mut CommandContext) -> i32 {
    let mut n = 10;
    let mut file_idx = 0;

    if args.first().map(|s| s.as_str()) == Some("-n") {
        if let Some(num) = args.get(1) {
            n = num.parse().unwrap_or(10);
            file_idx = 2;
        }
    }

    let content = if let Some(file) = args.get(file_idx) {
        let path = ctx.cwd.join(file);
        match ctx.fs.read(&path) {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => {
                let _ = writeln!(ctx.stderr, "tail: {file}: No such file");
                return 1;
            }
        }
    } else {
        ctx.stdin.clone().unwrap_or_default()
    };

    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    for line in &lines[start..] {
        let _ = writeln!(ctx.stdout, "{line}");
    }
    0
}

fn cmd_wc(args: &[String], ctx: &mut CommandContext) -> i32 {
    let content = if let Some(file) = args.first() {
        let path = ctx.cwd.join(file);
        match ctx.fs.read(&path) {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => {
                let _ = writeln!(ctx.stderr, "wc: {file}: No such file");
                return 1;
            }
        }
    } else {
        ctx.stdin.clone().unwrap_or_default()
    };

    let lines = content.lines().count();
    let words = content.split_whitespace().count();
    let chars = content.chars().count();
    let _ = writeln!(ctx.stdout, "{lines:>8} {words:>8} {chars:>8}");
    0
}

fn cmd_sort(args: &[String], ctx: &mut CommandContext) -> i32 {
    let reverse = args.contains(&"-r".to_string());
    let numeric = args.contains(&"-n".to_string());
    let file = args.iter().find(|a| !a.starts_with('-'));

    let content = if let Some(file) = file {
        let path = ctx.cwd.join(file);
        match ctx.fs.read(&path) {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => {
                let _ = writeln!(ctx.stderr, "sort: {file}: No such file");
                return 1;
            }
        }
    } else {
        ctx.stdin.clone().unwrap_or_default()
    };

    let mut lines: Vec<&str> = content.lines().collect();
    if numeric {
        lines.sort_by(|a, b| {
            let av: f64 = a.trim().parse().unwrap_or(0.0);
            let bv: f64 = b.trim().parse().unwrap_or(0.0);
            av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        lines.sort();
    }
    if reverse {
        lines.reverse();
    }
    for line in lines {
        let _ = writeln!(ctx.stdout, "{line}");
    }
    0
}

fn cmd_cut(args: &[String], ctx: &mut CommandContext) -> i32 {
    let delim = args
        .windows(2)
        .find(|w| w[0] == "-d")
        .and_then(|w| w.get(1))
        .cloned()
        .unwrap_or_else(|| "\t".into());

    let fields = args
        .windows(2)
        .find(|w| w[0] == "-f")
        .and_then(|w| w.get(1))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);

    let file = args.iter().find(|a| {
        !a.starts_with('-')
            && !args
                .iter()
                .enumerate()
                .any(|(i, p)| p == "-d" && i + 1 < args.len() && args[i + 1] == **a)
            && !args
                .iter()
                .enumerate()
                .any(|(i, p)| p == "-f" && i + 1 < args.len() && args[i + 1] == **a)
    });

    let content = if let Some(file) = file {
        let path = ctx.cwd.join(file);
        match ctx.fs.read(&path) {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => {
                let _ = writeln!(ctx.stderr, "cut: {file}: No such file");
                return 1;
            }
        }
    } else {
        ctx.stdin.clone().unwrap_or_default()
    };

    for line in content.lines() {
        if let Some(field) = line.split(&delim).nth(fields.saturating_sub(1)) {
            let _ = writeln!(ctx.stdout, "{field}");
        }
    }
    0
}

fn cmd_tr(args: &[String], ctx: &mut CommandContext) -> i32 {
    if args.len() < 2 {
        let _ = writeln!(ctx.stderr, "tr: missing operand");
        return 1;
    }
    let set1 = &args[0];
    let set2 = &args[1];
    let content = ctx.stdin.clone().unwrap_or_default();
    let result: String = content
        .chars()
        .map(|c| {
            if let Some(pos) = set1.find(c) {
                set2.chars().nth(pos).unwrap_or(c)
            } else {
                c
            }
        })
        .collect();
    let _ = write!(ctx.stdout, "{result}");
    0
}

fn cmd_uniq(args: &[String], ctx: &mut CommandContext) -> i32 {
    let count = args.contains(&"-c".to_string());
    let content = if let Some(file) = args.iter().find(|a| !a.starts_with('-')) {
        let path = ctx.cwd.join(file);
        match ctx.fs.read(&path) {
            Ok(data) => String::from_utf8_lossy(&data).to_string(),
            Err(_) => {
                let _ = writeln!(ctx.stderr, "uniq: {file}: No such file");
                return 1;
            }
        }
    } else {
        ctx.stdin.clone().unwrap_or_default()
    };

    let mut prev: Option<&str> = None;
    let mut run_count = 0;
    for line in content.lines() {
        if prev.map(|p| p == line).unwrap_or(false) {
            run_count += 1;
        } else {
            if let Some(p) = prev {
                if count {
                    let _ = writeln!(ctx.stdout, "{run_count:>4} {p}");
                } else {
                    let _ = writeln!(ctx.stdout, "{p}");
                }
            }
            prev = Some(line);
            run_count = 1;
        }
    }
    if let Some(p) = prev {
        if count {
            let _ = writeln!(ctx.stdout, "{run_count:>4} {p}");
        } else {
            let _ = writeln!(ctx.stdout, "{p}");
        }
    }
    0
}

fn cmd_env(_args: &[String], ctx: &mut CommandContext) -> i32 {
    let mut pairs: Vec<_> = ctx.env.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in &pairs {
        let _ = writeln!(ctx.stdout, "{k}={v}");
    }
    0
}

fn cmd_export(args: &[String], ctx: &mut CommandContext) -> i32 {
    for arg in args {
        if let Some(eq) = arg.find('=') {
            let name = &arg[..eq];
            let value = &arg[eq + 1..];
            ctx.env.insert(name.to_string(), value.to_string());
        }
    }
    0
}

// ── Shell Parser ──────────────────────────────────────────────

#[derive(Debug)]
pub enum RedirectOp {
    Output(String),
    Append(String),
    Input(String),
    Stderr(String),
    StderrToStdout,
}

#[derive(Debug)]
pub struct SimpleCommand {
    pub name: String,
    pub args: Vec<String>,
    pub redirects: Vec<RedirectOp>,
}

#[derive(Debug)]
pub enum AstNode {
    Simple(SimpleCommand),
    Pipeline(Vec<AstNode>),
    And(Box<AstNode>, Box<AstNode>),
    Or(Box<AstNode>, Box<AstNode>),
    Seq(Box<AstNode>, Box<AstNode>),
}

pub fn parse(input: &str) -> Result<AstNode, String> {
    let tokens = tokenize(input)?;
    parse_sequence(&tokens)
}

fn tokenize(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
        } else if in_double {
            if ch == '"' {
                in_double = false;
            } else if ch == '\\' {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            } else {
                current.push(ch);
            }
        } else if ch == '\'' {
            in_single = true;
        } else if ch == '"' {
            in_double = true;
        } else if ch == '#' && current.is_empty() {
            break;
        } else if ch == '|' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push("|".into());
        } else if ch == ';' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push(";".into());
        } else if ch == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push("&&".into());
            } else {
                current.push(ch);
            }
        } else if ch == '>' {
            if chars.peek() == Some(&'>') {
                chars.next();
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push(">>".into());
            } else if chars.peek() == Some(&'&') {
                chars.next();
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push("2>&1".into());
            } else {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push(">".into());
            }
        } else if ch == '<' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            tokens.push("<".into());
        } else if ch == ' ' || ch == '\t' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

fn parse_sequence(tokens: &[String]) -> Result<AstNode, String> {
    let mut nodes = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == ";" {
            i += 1;
            continue;
        }
        let (node, consumed) = parse_and_or(tokens, i)?;
        nodes.push(node);
        i += consumed;
        if i < tokens.len() && tokens[i] == ";" {
            i += 1;
        }
    }

    if nodes.is_empty() {
        return Err("empty command".into());
    }

    let mut iter = nodes.into_iter();
    let mut result = iter.next().unwrap();
    for node in iter {
        result = AstNode::Seq(Box::new(result), Box::new(node));
    }
    Ok(result)
}

fn parse_and_or(tokens: &[String], start: usize) -> Result<(AstNode, usize), String> {
    let (left, mut consumed) = parse_pipeline(tokens, start)?;

    if start + consumed < tokens.len() {
        if tokens[start + consumed] == "&&" {
            let (right, right_consumed) = parse_and_or(tokens, start + consumed + 1)?;
            consumed += 1 + right_consumed;
            return Ok((AstNode::And(Box::new(left), Box::new(right)), consumed));
        } else if tokens[start + consumed] == "||" {
            let (right, right_consumed) = parse_and_or(tokens, start + consumed + 1)?;
            consumed += 1 + right_consumed;
            return Ok((AstNode::Or(Box::new(left), Box::new(right)), consumed));
        }
    }

    Ok((left, consumed))
}

fn parse_pipeline(tokens: &[String], start: usize) -> Result<(AstNode, usize), String> {
    let mut commands = Vec::new();
    let mut i = start;

    loop {
        let (cmd, consumed) = parse_simple(tokens, i)?;
        commands.push(cmd);
        i += consumed;

        if i < tokens.len() && tokens[i] == "|" {
            i += 1;
        } else {
            break;
        }
    }

    if commands.len() == 1 {
        Ok((commands.into_iter().next().unwrap(), i - start))
    } else {
        let pipeline = AstNode::Pipeline(commands);
        Ok((pipeline, i - start))
    }
}

fn parse_simple(tokens: &[String], start: usize) -> Result<(AstNode, usize), String> {
    if start >= tokens.len() {
        return Err("unexpected end".into());
    }

    let mut args = Vec::new();
    let mut redirects = Vec::new();
    let mut i = start;

    while i < tokens.len() {
        match tokens[i].as_str() {
            "|" | ";" | "&&" | "||" => break,
            ">" => {
                i += 1;
                if i < tokens.len() {
                    redirects.push(RedirectOp::Output(tokens[i].clone()));
                    i += 1;
                }
            }
            ">>" => {
                i += 1;
                if i < tokens.len() {
                    redirects.push(RedirectOp::Append(tokens[i].clone()));
                    i += 1;
                }
            }
            "<" => {
                i += 1;
                if i < tokens.len() {
                    redirects.push(RedirectOp::Input(tokens[i].clone()));
                    i += 1;
                }
            }
            "2>" => {
                i += 1;
                if i < tokens.len() {
                    redirects.push(RedirectOp::Stderr(tokens[i].clone()));
                    i += 1;
                }
            }
            "2>&1" => {
                redirects.push(RedirectOp::StderrToStdout);
                i += 1;
            }
            _ => {
                args.push(tokens[i].clone());
                i += 1;
            }
        }
    }

    if args.is_empty() {
        return Err("empty command".into());
    }

    Ok((
        AstNode::Simple(SimpleCommand {
            name: args.remove(0),
            args,
            redirects,
        }),
        i - start,
    ))
}

// ── BashSession ───────────────────────────────────────────────

pub struct BashSession {
    fs: Box<dyn FileSystem>,
    env: HashMap<String, String>,
    cwd: PathBuf,
    registry: CommandRegistry,
}

impl BashSession {
    pub fn new(fs: Box<dyn FileSystem>) -> Self {
        let mut env = HashMap::new();
        env.insert("HOME".into(), "/home/user".into());
        env.insert("PWD".into(), "/home/user".into());
        env.insert("USER".into(), "user".into());
        env.insert("SHELL".into(), "/bin/bash".into());

        Self {
            fs,
            env,
            cwd: PathBuf::from("/home/user"),
            registry: CommandRegistry::new(),
        }
    }

    pub fn execute(&mut self, command: &str) -> BashOutput {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let ast = match parse(command) {
            Ok(ast) => ast,
            Err(e) => {
                let _ = writeln!(stderr, "parse error: {e}");
                return BashOutput {
                    stdout: String::new(),
                    stderr: String::from_utf8_lossy(&stderr).to_string(),
                    exit_code: 2,
                };
            }
        };

        let exit_code = self.execute_node(&ast, &mut stdout, &mut stderr, None);

        BashOutput {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code,
        }
    }

    fn execute_node(
        &mut self,
        node: &AstNode,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
        stdin: Option<String>,
    ) -> i32 {
        match node {
            AstNode::Simple(cmd) => {
                let cwd = self.cwd.clone();

                let mut stdin_local = stdin;
                let mut stderr_capture: Option<Vec<u8>> = None;
                let mut redirect_stdout_to_file: Option<String> = None;
                let mut redirect_append_to_file: Option<String> = None;
                let mut redirect_stderr_to_file: Option<String> = None;
                let mut merge_stderr = false;

                for redirect in &cmd.redirects {
                    match redirect {
                        RedirectOp::Output(path) => {
                            let full_path = cwd.join(path);
                            redirect_stdout_to_file = Some(full_path.to_string_lossy().into());
                        }
                        RedirectOp::Append(path) => {
                            let full_path = cwd.join(path);
                            redirect_append_to_file = Some(full_path.to_string_lossy().into());
                        }
                        RedirectOp::Input(path) => {
                            let full_path = cwd.join(path);
                            if let Ok(content) = self.fs.read(&full_path) {
                                stdin_local = Some(String::from_utf8_lossy(&content).to_string());
                            }
                        }
                        RedirectOp::Stderr(path) => {
                            let full_path = cwd.join(path);
                            redirect_stderr_to_file = Some(full_path.to_string_lossy().into());
                            stderr_capture = Some(Vec::new());
                        }
                        RedirectOp::StderrToStdout => {
                            merge_stderr = true;
                        }
                    }
                }

                let mut merge_buffer = Vec::new();

                let actual_stderr: &mut Vec<u8> = if let Some(ref mut cap) = stderr_capture {
                    cap
                } else if merge_stderr {
                    &mut merge_buffer
                } else {
                    stderr
                };

                let mut ctx = CommandContext {
                    fs: self.fs.as_ref(),
                    env: &mut self.env,
                    cwd: &mut self.cwd,
                    stdin: stdin_local,
                    stdout,
                    stderr: actual_stderr,
                    cancelled: false,
                };

                let exit_code = self.registry.execute(&cmd.name, &cmd.args, &mut ctx);

                if merge_stderr {
                    stdout.extend_from_slice(&merge_buffer);
                }

                if let Some(path) = redirect_stdout_to_file {
                    let _ = self.fs.write(Path::new(&path), stdout);
                }
                if let Some(path) = redirect_append_to_file {
                    let existing = self.fs.read(Path::new(&path)).unwrap_or_default();
                    let mut content = existing;
                    content.extend_from_slice(stdout);
                    let _ = self.fs.write(Path::new(&path), &content);
                }
                if let Some(path) = redirect_stderr_to_file {
                    if let Some(captured) = stderr_capture {
                        let _ = self.fs.write(Path::new(&path), &captured);
                    }
                }

                exit_code
            }
            AstNode::Pipeline(commands) => {
                let mut prev_stdout: Option<String> = None;
                let mut exit_code = 0;

                for (i, cmd) in commands.iter().enumerate() {
                    let mut pipe_stdout = Vec::new();
                    let mut pipe_stderr = Vec::new();
                    let is_last = i == commands.len() - 1;

                    exit_code = self.execute_node(
                        cmd,
                        if is_last { stdout } else { &mut pipe_stdout },
                        &mut pipe_stderr,
                        prev_stdout.take(),
                    );

                    if !is_last {
                        prev_stdout = Some(String::from_utf8_lossy(&pipe_stdout).to_string());
                    }
                }

                exit_code
            }
            AstNode::And(left, right) => {
                let exit = self.execute_node(left, stdout, stderr, stdin);
                if exit == 0 {
                    self.execute_node(right, stdout, stderr, None)
                } else {
                    exit
                }
            }
            AstNode::Or(left, right) => {
                let exit = self.execute_node(left, stdout, stderr, stdin);
                if exit != 0 {
                    self.execute_node(right, stdout, stderr, None)
                } else {
                    exit
                }
            }
            AstNode::Seq(left, right) => {
                self.execute_node(left, stdout, stderr, stdin);
                self.execute_node(right, stdout, stderr, None)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BashOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

// ── BashTool (Tool trait wrapper) ─────────────────────────────

use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::cell::RefCell;

thread_local! {
    static SESSION: RefCell<Option<BashSession>> = const { RefCell::new(None) };
}

fn with_session<F, R>(workspace_root: &PathBuf, f: F) -> R
where
    F: FnOnce(&mut BashSession) -> R,
{
    SESSION.with(|s| {
        let mut session = s.borrow_mut();
        if session.is_none() {
            *session = Some(BashSession::new(Box::new(InMemoryFs::default())));
            let _ = workspace_root;
        }
        f(session.as_mut().unwrap())
    })
}

pub struct BashTool {
    workspace_root: PathBuf,
}

impl BashTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in a persistent virtual shell. The filesystem persists across calls."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 120)."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let command = match params.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'command' parameter.".into(),
                }
            }
        };

        let output = with_session(&self.workspace_root, |session| session.execute(command));

        let mut result = String::new();
        if !output.stdout.is_empty() {
            result.push_str(&output.stdout);
        }
        if !output.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&output.stderr);
        }
        if output.exit_code != 0 && result.is_empty() {
            result = format!("Command exited with code {}", output.exit_code);
        }

        ToolResult::Success { content: result }
    }
}
