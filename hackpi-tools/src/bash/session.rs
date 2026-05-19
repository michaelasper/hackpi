use std::collections::HashMap;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use tokio::sync::watch;

use super::commands::{CommandContext, CommandRegistry};
use super::filesystem::FileSystem;
use super::parser::{parse, AstNode, RedirectOp};

pub struct BashSession {
    pub fs: Box<dyn FileSystem>,
    pub env: HashMap<String, String>,
    pub cwd: PathBuf,
    pub registry: CommandRegistry,
    pub command_count: u32,
    pub signal: Option<watch::Receiver<bool>>,
    /// Real host filesystem path, used to run host commands (e.g. `cargo`).
    pub host_workspace_root: PathBuf,
}

impl BashSession {
    pub fn new(fs: Box<dyn FileSystem>) -> Self {
        Self::with_workspace(fs, PathBuf::from("/"))
    }

    /// Create a new BashSession with a configurable home/workspace directory.
    ///
    /// The `workspace_dir` is used as the initial `HOME`, `PWD`, and `cwd`.
    /// This allows the virtual shell to be rooted at the actual project
    /// directory instead of a hardcoded `/home/user`.
    pub fn with_workspace(fs: Box<dyn FileSystem>, workspace_dir: PathBuf) -> Self {
        let home = workspace_dir.to_string_lossy().to_string();
        let mut env = HashMap::new();
        env.insert("HOME".into(), home.clone());
        env.insert("PWD".into(), home);
        env.insert("USER".into(), "user".into());
        env.insert("SHELL".into(), "/bin/bash".into());

        let host_root = workspace_dir.clone();
        Self {
            fs,
            env,
            cwd: workspace_dir,
            registry: CommandRegistry::new(),
            command_count: 0,
            signal: None,
            host_workspace_root: host_root,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.signal.as_ref().is_some_and(|s| *s.borrow())
    }

    fn resolve_vars(&self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '$' {
                let mut name = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        if c == '}' {
                            chars.next();
                            break;
                        }
                        name.push(c);
                        chars.next();
                    }
                } else {
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' {
                            name.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                let val = if name.is_empty() {
                    "$".to_string()
                } else {
                    self.env.get(&name).cloned().unwrap_or_default()
                };
                result.push_str(&val);
            } else {
                result.push(ch);
            }
        }
        result
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
                    command_count: 0,
                };
            }
        };

        let exit_code = self.execute_node(&ast, &mut stdout, &mut stderr, None);

        BashOutput {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code,
            command_count: self.command_count,
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

                self.command_count += 1;

                let name = self.resolve_vars(&cmd.name);

                let mut all_args: Vec<&str> = cmd.args.iter().map(|s| s.as_str()).collect();

                let mut env_overrides: Vec<(String, String)> = Vec::new();
                let mut actual_name = name.as_str();

                loop {
                    if let Some(eq) = actual_name.find('=') {
                        let before_eq = &actual_name[..eq];
                        if !before_eq.is_empty()
                            && before_eq
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                        {
                            env_overrides
                                .push((before_eq.to_string(), actual_name[eq + 1..].to_string()));
                            if let Some(first_arg) = all_args.first() {
                                actual_name = first_arg;
                                all_args = all_args[1..].to_vec();
                                continue;
                            }
                        }
                    }
                    break;
                }

                let is_export = actual_name == "export";

                let mut filtered_args: Vec<String> = Vec::new();
                for arg in &all_args {
                    if !is_export {
                        if let Some(eq) = arg.find('=') {
                            let before_eq = &arg[..eq];
                            if !before_eq.is_empty()
                                && before_eq
                                    .chars()
                                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                            {
                                env_overrides
                                    .push((before_eq.to_string(), arg[eq + 1..].to_string()));
                                continue;
                            }
                        }
                    }
                    filtered_args.push(arg.to_string());
                }

                for (k, v) in &env_overrides {
                    self.env.insert(k.clone(), v.clone());
                }

                let resolved_name = self.resolve_vars(actual_name);
                let resolved_args: Vec<String> =
                    filtered_args.iter().map(|a| self.resolve_vars(a)).collect();

                let mut ctx = CommandContext {
                    fs: self.fs.as_ref(),
                    env: &mut self.env,
                    cwd: &mut self.cwd,
                    stdin: stdin_local,
                    stdout,
                    stderr: actual_stderr,
                    signal: self.signal.as_ref(),
                    host_workspace_root: Some(self.host_workspace_root.clone()),
                };

                let exit_code = self
                    .registry
                    .execute(&resolved_name, &resolved_args, &mut ctx);

                for (k, _) in &env_overrides {
                    self.env.remove(k);
                }

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
                if self.is_cancelled() {
                    return exit;
                }
                if exit == 0 {
                    self.execute_node(right, stdout, stderr, None)
                } else {
                    exit
                }
            }
            AstNode::Or(left, right) => {
                let exit = self.execute_node(left, stdout, stderr, stdin);
                if self.is_cancelled() {
                    return exit;
                }
                if exit != 0 {
                    self.execute_node(right, stdout, stderr, None)
                } else {
                    exit
                }
            }
            AstNode::Seq(left, right) => {
                self.execute_node(left, stdout, stderr, stdin);
                if self.is_cancelled() {
                    return 130;
                }
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
    pub command_count: u32,
}

/// Normalize a path by resolving `.` and `..` components.
/// Works on virtual filesystem paths (no canonicalize needed).
pub(crate) fn normalize_path(path: &str) -> String {
    use std::path::Component;
    let path = std::path::Path::new(path);
    let mut components: Vec<&str> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => continue,
            Component::ParentDir => {
                components.pop();
            }
            Component::RootDir => {
                components.clear();
            }
            Component::Normal(name) => {
                if let Some(s) = name.to_str() {
                    components.push(s);
                }
            }
            Component::Prefix(_) => {}
        }
    }
    if components.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", components.join("/"))
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn test_normalize_path_simple() {
        assert_eq!(normalize_path("/home/user"), "/home/user");
    }

    #[test]
    fn test_normalize_path_dotdot() {
        assert_eq!(normalize_path("/home/user/.."), "/home");
    }

    #[test]
    fn test_normalize_path_complex() {
        assert_eq!(normalize_path("/tmp/../home/user"), "/home/user");
    }

    #[test]
    fn test_normalize_path_above_root() {
        assert_eq!(normalize_path("/../../.."), "/");
    }

    #[test]
    fn test_normalize_path_dot() {
        assert_eq!(normalize_path("/home/./user"), "/home/user");
    }

    #[test]
    fn test_normalize_path_root() {
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn test_normalize_path_relative() {
        assert_eq!(normalize_path("foo/bar"), "/foo/bar");
    }

    #[test]
    fn test_normalize_path_relative_dotdot() {
        assert_eq!(normalize_path("foo/../bar"), "/bar");
    }
}
