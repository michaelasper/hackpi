use std::collections::HashMap;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

use super::filesystem::FileSystem;

pub struct CommandContext<'a> {
    pub fs: &'a dyn FileSystem,
    pub env: &'a mut HashMap<String, String>,
    pub cwd: &'a mut PathBuf,
    pub stdin: Option<String>,
    pub stdout: &'a mut Vec<u8>,
    pub stderr: &'a mut Vec<u8>,
    pub signal: Option<&'a tokio::sync::watch::Receiver<bool>>,
}

impl CommandContext<'_> {
    pub fn is_cancelled(&self) -> bool {
        self.signal.is_some_and(|s| *s.borrow())
    }
}

pub type CommandFn = for<'a> fn(args: &[String], ctx: &mut CommandContext<'a>) -> i32;

pub struct CommandRegistry {
    commands: HashMap<String, CommandFn>,
    help_texts: HashMap<&'static str, &'static str>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
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
        cmds.insert("ln".to_string(), cmd_ln);

        let mut help = HashMap::new();
        help.insert("cd", "cd [path]  Change directory (default: ~)");
        help.insert("pwd", "pwd  Print working directory");
        help.insert("echo", "echo [-n] [args...]  Print arguments");
        help.insert("ls", "ls [-la] [path...]  List directory contents");
        help.insert("cat", "cat [files...]  Concatenate and print files");
        help.insert("cp", "cp src dst  Copy files");
        help.insert("mv", "mv src dst  Move/rename files");
        help.insert("rm", "rm [-rf] path  Remove files or directories");
        help.insert("mkdir", "mkdir [-p] path  Create directories");
        help.insert("touch", "touch path  Create or update file timestamp");
        help.insert(
            "grep",
            "grep [-i] pattern [files...]  Search for pattern in files",
        );
        help.insert(
            "head",
            "head [-n N] [file]  Print first N lines (default: 10)",
        );
        help.insert(
            "tail",
            "tail [-n N] [file]  Print last N lines (default: 10)",
        );
        help.insert("wc", "wc [file]  Count lines, words, chars");
        help.insert("sort", "sort [-r] [-n] [file]  Sort lines");
        help.insert("cut", "cut -d DELIM -f FIELD [file]  Cut columns");
        help.insert("tr", "tr SET1 SET2  Translate characters");
        help.insert("uniq", "uniq [-c] [file]  Filter adjacent duplicate lines");
        help.insert("env", "env  Print environment variables");
        help.insert("export", "export NAME=value  Set environment variable");
        help.insert("ln", "ln [-s] target link  Create hard or symbolic links");

        Self {
            commands: cmds,
            help_texts: help,
        }
    }

    pub fn execute(&self, name: &str, args: &[String], ctx: &mut CommandContext) -> i32 {
        if args.iter().any(|a| a == "--help" || a == "-h") {
            if let Some(text) = self.help_texts.get(name) {
                let _ = writeln!(ctx.stdout, "{text}");
                return 0;
            }
        }
        match self.commands.get(name) {
            Some(f) => f(args, ctx),
            None => {
                let _ = writeln!(ctx.stderr, "bash: {name}: command not found");
                127
            }
        }
    }
}

fn cmd_cd(args: &[String], ctx: &mut CommandContext) -> i32 {
    let target = args.first().map(|s| s.as_str()).unwrap_or("~");
    let new_cwd = if target == "~" {
        ctx.env
            .get("HOME")
            .cloned()
            .unwrap_or_else(|| "/home/user".into())
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
                        let mode = if entry.is_dir {
                            "drwxr-xr-x"
                        } else {
                            "-rw-r--r--"
                        };
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
                let _ = writeln!(
                    ctx.stderr,
                    "ls: cannot access '{}': No such file or directory",
                    dir.display()
                );
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

    // Build regex with optional case-insensitive flag.
    // The regex crate uses efficient (Boyer-Moore style) matching internally.
    let re = match regex::RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
    {
        Ok(re) => re,
        Err(e) => {
            let _ = writeln!(ctx.stderr, "grep: invalid pattern '{pattern}': {e}");
            return 2;
        }
    };

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
        if re.is_match(line) {
            if files.len() > 1 {
                let prefix = targets.get(1).map(|s| s.as_str()).unwrap_or("");
                let _ = writeln!(ctx.stdout, "{prefix}:{}:{line}", i + 1);
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

fn cmd_ln(args: &[String], ctx: &mut CommandContext) -> i32 {
    let symlink = args.contains(&"-s".to_string());
    let targets: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if targets.len() < 2 {
        let _ = writeln!(ctx.stderr, "ln: missing operand");
        return 1;
    }
    let target = ctx.cwd.join(targets[0]);
    let link = ctx.cwd.join(targets[1]);
    if symlink {
        match ctx.fs.symlink(&target, &link) {
            Ok(_) => 0,
            Err(e) => {
                let _ = writeln!(ctx.stderr, "ln: {e}");
                1
            }
        }
    } else {
        match ctx.fs.copy(&target, &link) {
            Ok(_) => 0,
            Err(e) => {
                let _ = writeln!(ctx.stderr, "ln: {e}");
                1
            }
        }
    }
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
