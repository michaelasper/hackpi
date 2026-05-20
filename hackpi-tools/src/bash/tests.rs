use std::path::Path;
use tokio::sync::watch;

use super::filesystem::{FileSystem, InMemoryFs};
use super::parser::{parse, tokenize, AstNode, RedirectOp};
use super::session::{BashOutput, BashSession};
use super::tool::BashTool;
use hackpi_core::tools::Tool;

fn new_session() -> BashSession {
    let home = Path::new("/home/user");
    let fs = Box::new(InMemoryFs::with_home(&std::path::PathBuf::from("/")));
    let session = BashSession::with_workspace(fs, home.to_path_buf());

    session
        .fs
        .write(
            &home.join("hello.txt"),
            b"hello world\nline two\nline three\n",
        )
        .unwrap();
    session
        .fs
        .write(&home.join("numbers.txt"), b"3\n1\n2\n")
        .unwrap();
    session
        .fs
        .write(&home.join("colors.txt"), b"red\ngreen\nblue\n")
        .unwrap();

    session
}

fn output_stdout(out: &BashOutput) -> &str {
    out.stdout.trim()
}

#[test]
fn test_echo() {
    let mut session = new_session();
    let out = session.execute("echo hello world");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "hello world");
}

#[test]
fn test_echo_no_newline() {
    let mut session = new_session();
    let out = session.execute("echo -n hello");
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "hello");
}

#[test]
fn test_pwd() {
    let mut session = new_session();
    let out = session.execute("pwd");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "/home/user");
}

#[test]
fn test_cd() {
    let mut session = new_session();
    let out = session.execute("cd /tmp");
    assert_eq!(out.exit_code, 0);
    let out = session.execute("pwd");
    assert_eq!(output_stdout(&out), "/tmp");
}

#[test]
fn test_cd_home() {
    let mut session = new_session();
    session.execute("cd /tmp");
    let out = session.execute("cd");
    assert_eq!(out.exit_code, 0);
    let out = session.execute("pwd");
    assert_eq!(output_stdout(&out), "/home/user");
}

#[test]
fn test_cd_bad_path() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent");
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("No such directory"));
}

#[test]
fn test_or_operator_first_fails_second_runs() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent || echo fallback");
    assert_eq!(out.exit_code, 0);
    assert!(output_stdout(&out).contains("fallback"));
}

#[test]
fn test_or_operator_first_succeeds_second_skipped() {
    let mut session = new_session();
    let out = session.execute("echo first || echo second");
    assert_eq!(out.exit_code, 0);
    let stdout = output_stdout(&out);
    assert!(stdout.contains("first"));
    assert!(!stdout.contains("second"));
}

#[test]
fn test_or_operator_exit_code_from_failing() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent || cd /nonexistent2");
    assert_ne!(out.exit_code, 0);
}

#[test]
fn test_and_operator_both_succeed() {
    let mut session = new_session();
    let out = session.execute("echo first && echo second");
    assert_eq!(out.exit_code, 0);
    let stdout = output_stdout(&out);
    assert!(stdout.contains("first"));
    assert!(stdout.contains("second"));
}

#[test]
fn test_and_operator_first_fails_second_skipped() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent && echo second");
    assert_ne!(out.exit_code, 0);
    assert!(!out.stdout.contains("second"));
}

#[test]
fn test_seq_operator() {
    let mut session = new_session();
    let out = session.execute("echo a; echo b");
    assert_eq!(out.exit_code, 0);
    let stdout = output_stdout(&out);
    assert!(stdout.contains("a"));
    assert!(stdout.contains("b"));
}

#[test]
fn test_stderr_redirect_to_file() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent 2>/tmp/stderr.txt");
    assert_eq!(out.exit_code, 1);
    let content = session.fs.read(Path::new("/tmp/stderr.txt")).unwrap();
    let stderr_content = String::from_utf8_lossy(&content);
    assert!(stderr_content.contains("No such directory"));
}

#[test]
fn test_stderr_redirect_stdout_clean() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent 2>/tmp/err.txt");
    assert_eq!(out.exit_code, 1);
    assert!(out.stdout.is_empty());
}

#[test]
fn test_stderr_append_to_file() {
    let mut session = new_session();
    session.execute("cd /nonexistent 2>/tmp/err.txt");
    let out = session.execute("cd /nonexistent2 2>>/tmp/err.txt");
    assert_eq!(out.exit_code, 1);
    let content = session.fs.read(Path::new("/tmp/err.txt")).unwrap();
    let stderr_content = String::from_utf8_lossy(&content);
    assert!(stderr_content.contains("No such directory"));
}

#[test]
fn test_stderr_to_stdout_merge() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent 2>&1");
    assert_eq!(out.exit_code, 1);
    assert!(out.stdout.contains("No such directory"));
}

#[test]
fn test_stdout_and_stderr_merged() {
    let mut session = new_session();
    let out = session.execute("echo hi 2>&1");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hi"));
}

#[test]
fn test_stdout_redirect_with_1_prefix() {
    let mut session = new_session();
    let out = session.execute("echo hello 1>/tmp/out.txt");
    assert_eq!(out.exit_code, 0);
    let content = session.fs.read(Path::new("/tmp/out.txt")).unwrap();
    assert_eq!(String::from_utf8_lossy(&content), "hello\n");
}

#[test]
fn test_stdout_append_with_1_prefix() {
    let mut session = new_session();
    session.execute("echo line1 1>/tmp/out.txt");
    session.execute("echo line2 1>>/tmp/out.txt");
    let content = session.fs.read(Path::new("/tmp/out.txt")).unwrap();
    let text = String::from_utf8_lossy(&content);
    assert!(text.contains("line1"));
    assert!(text.contains("line2"));
}

#[test]
fn test_stdout_redirect() {
    let mut session = new_session();
    let out = session.execute("echo hello >/tmp/out.txt");
    assert_eq!(out.exit_code, 0);
    let content = session.fs.read(Path::new("/tmp/out.txt")).unwrap();
    assert_eq!(String::from_utf8_lossy(&content), "hello\n");
}

#[test]
fn test_stdout_append() {
    let mut session = new_session();
    session.execute("echo a >/tmp/out.txt");
    session.execute("echo b >>/tmp/out.txt");
    let content = session.fs.read(Path::new("/tmp/out.txt")).unwrap();
    let text = String::from_utf8_lossy(&content);
    assert!(text.contains("a"));
    assert!(text.contains("b"));
}

#[test]
fn test_help_echo() {
    let mut session = new_session();
    let out = session.execute("echo --help");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Print arguments"));
}

#[test]
fn test_help_cd() {
    let mut session = new_session();
    let out = session.execute("cd --help");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Change directory"));
}

#[test]
fn test_help_ls() {
    let mut session = new_session();
    let out = session.execute("ls --help");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("List directory"));
}

#[test]
fn test_help_cat() {
    let mut session = new_session();
    let out = session.execute("cat --help");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Concatenate"));
}

#[test]
fn test_help_grep() {
    let mut session = new_session();
    let out = session.execute("grep --help");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Search for pattern"));
}

#[test]
fn test_help_unknown_command() {
    let mut session = new_session();
    let out = session.execute("nonexistent --help");
    assert_eq!(out.exit_code, 127);
    assert!(out.stderr.contains("command not found"));
}

#[test]
fn test_help_short_flag() {
    let mut session = new_session();
    let out = session.execute("echo -h");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("Print arguments"));
}

#[test]
fn test_var_expansion_simple() {
    let mut session = new_session();
    session.env.insert("MYVAR".into(), "world".into());
    let out = session.execute("echo hello $MYVAR");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "hello world");
}

#[test]
fn test_var_expansion_braces() {
    let mut session = new_session();
    session.env.insert("MYVAR".into(), "world".into());
    let out = session.execute("echo hello ${MYVAR}");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "hello world");
}

#[test]
fn test_var_expansion_undefined() {
    let mut session = new_session();
    let out = session.execute("echo hello $UNDEFINED");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.starts_with("hello "));
}

#[test]
fn test_var_expansion_home_var() {
    let mut session = new_session();
    let out = session.execute("echo $HOME");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "/home/user");
}

#[test]
fn test_var_expansion_user_var() {
    let mut session = new_session();
    let out = session.execute("echo $USER");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "user");
}

#[test]
fn test_var_expansion_in_command_name() {
    let mut session = new_session();
    session.env.insert("CMD".into(), "echo".into());
    let out = session.execute("echo hello");
    assert_eq!(out.exit_code, 0);
}

#[test]
fn test_brace_var_empty_name() {
    let mut session = new_session();
    let out = session.execute("echo $");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "$");
}

#[test]
fn test_brace_var_expansion_multiple() {
    let mut session = new_session();
    session.env.insert("A".into(), "1".into());
    session.env.insert("B".into(), "2".into());
    let out = session.execute("echo ${A} ${B} ${A}");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "1 2 1");
}

#[test]
fn test_var_in_filename_arg() {
    let mut session = new_session();
    session
        .fs
        .write(Path::new("/home/user/mydata.txt"), b"content")
        .unwrap();
    let out = session.execute("cat $HOME/mydata.txt");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "content");
}

#[test]
fn test_env_override_before_command() {
    let mut session = new_session();
    let out = session.execute("FOO=bar echo $FOO");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "bar");
}

#[test]
fn test_env_override_does_not_persist() {
    let mut session = new_session();
    session.execute("FOO=bar echo hello");
    let out = session.execute("echo $FOO");
    assert_eq!(output_stdout(&out), "");
}

#[test]
fn test_env_override_multiple() {
    let mut session = new_session();
    let out = session.execute("A=1 B=2 echo $A $B");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "1 2");
}

#[test]
fn test_command_count_starts_at_zero() {
    let session = new_session();
    assert_eq!(session.command_count, 0);
}

#[test]
fn test_command_count_increments() {
    let mut session = new_session();
    let out1 = session.execute("echo a");
    assert_eq!(out1.command_count, 1);
    let out2 = session.execute("echo b");
    assert_eq!(out2.command_count, 2);
}

#[test]
fn test_command_count_with_seq() {
    let mut session = new_session();
    let out = session.execute("echo a; echo b; echo c");
    assert_eq!(out.command_count, 3);
}

#[test]
fn test_command_count_with_and() {
    let mut session = new_session();
    let out = session.execute("echo a && echo b");
    assert_eq!(out.command_count, 2);
}

#[test]
fn test_pipeline_simple() {
    let mut session = new_session();
    let out = session.execute("echo hello world | wc");
    assert_eq!(out.exit_code, 0);
    let parts: Vec<&str> = output_stdout(&out).split_whitespace().collect();
    assert_eq!(parts[0], "1");
    assert_eq!(parts[1], "2");
}

#[test]
fn test_ls_home() {
    let mut session = new_session();
    let out = session.execute("ls /home/user");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello.txt"));
}

#[test]
fn test_ls_bad_path() {
    let mut session = new_session();
    let out = session.execute("ls /nonexistent");
    assert_eq!(out.exit_code, 1);
}

#[test]
fn test_cat_file() {
    let mut session = new_session();
    let out = session.execute("cat hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello world"));
}

#[test]
fn test_cat_bad_file() {
    let mut session = new_session();
    let out = session.execute("cat nonexistent.txt");
    assert_eq!(out.exit_code, 1);
}

#[test]
fn test_mkdir() {
    let mut session = new_session();
    let out = session.execute("mkdir -p /tmp/newdir");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_dir(Path::new("/tmp/newdir")));
}

#[test]
fn test_mkdir_p() {
    let mut session = new_session();
    let out = session.execute("mkdir -p /tmp/a/b/c");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_dir(Path::new("/tmp/a/b/c")));
}

#[test]
fn test_touch_new_file() {
    let mut session = new_session();
    let out = session.execute("touch /tmp/newfile.txt");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_file(Path::new("/tmp/newfile.txt")));
}

#[test]
fn test_rm_file() {
    let mut session = new_session();
    let out = session.execute("rm /home/user/hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/home/user/hello.txt")));
}

#[test]
fn test_rm_rf_dir() {
    let mut session = new_session();
    session.execute("mkdir -p /tmp/rmtest/subdir");
    let out = session.execute("rm -rf /tmp/rmtest");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/tmp/rmtest")));
}

#[test]
fn test_rm_dir_without_rf_fails() {
    let mut session = new_session();
    session.execute("mkdir /tmp/rmtest2");
    let out = session.execute("rm /tmp/rmtest2");
    assert_eq!(out.exit_code, 1);
}

#[test]
fn test_cp_file() {
    let mut session = new_session();
    let out = session.execute("cp hello.txt /tmp/hello_copy.txt");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_file(Path::new("/tmp/hello_copy.txt")));
}

#[test]
fn test_mv_file() {
    let mut session = new_session();
    let out = session.execute("mv hello.txt /tmp/moved.txt");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/home/user/hello.txt")));
    assert!(session.fs.is_file(Path::new("/tmp/moved.txt")));
}

#[test]
fn test_grep_stdin() {
    let mut session = new_session();
    let out = session.execute("echo hello world | grep hello");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello"));
}

#[test]
fn test_head_default() {
    let mut session = new_session();
    let out = session.execute("head hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello world"));
}

#[test]
fn test_tail_default() {
    let mut session = new_session();
    let out = session.execute("tail hello.txt");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("line three"));
}

#[test]
fn test_wc() {
    let mut session = new_session();
    let out = session.execute("wc hello.txt");
    assert_eq!(out.exit_code, 0);
    let parts: Vec<&str> = output_stdout(&out).split_whitespace().collect();
    assert_eq!(parts[0], "3");
}

#[test]
fn test_sort() {
    let mut session = new_session();
    let out = session.execute("sort numbers.txt");
    assert_eq!(out.exit_code, 0);
    let lines: Vec<&str> = output_stdout(&out).lines().collect();
    assert_eq!(lines, vec!["1", "2", "3"]);
}

#[test]
fn test_env() {
    let mut session = new_session();
    let out = session.execute("env");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("HOME=/home/user"));
    assert!(out.stdout.contains("USER=user"));
}

#[test]
fn test_export() {
    let mut session = new_session();
    let out = session.execute("export FOO=bar");
    assert_eq!(out.exit_code, 0);
    assert_eq!(session.env.get("FOO").unwrap(), "bar");
}

#[test]
fn test_command_not_found() {
    let mut session = new_session();
    let out = session.execute("nonexistent_cmd");
    assert_eq!(out.exit_code, 127);
    assert!(out.stderr.contains("command not found"));
}

#[test]
fn test_parse_error_empty() {
    let mut session = new_session();
    let out = session.execute("");
    assert_eq!(out.exit_code, 2);
}

#[test]
fn test_session_is_cancelled() {
    let session = new_session();
    assert!(!session.is_cancelled());
}

#[test]
fn test_cancelled_with_signal() {
    let (tx, rx) = watch::channel(false);
    let mut session = new_session();
    session.signal = Some(rx);
    assert!(!session.is_cancelled());
    tx.send(true).unwrap();
    assert!(session.is_cancelled());
}

#[test]
fn test_cancellation_stops_and_chain() {
    let (tx, rx) = watch::channel(false);
    let mut session = new_session();
    session.signal = Some(rx);
    tx.send(true).unwrap();
    let out = session.execute("echo a && echo b");
    assert_eq!(out.exit_code, 0);
}

#[test]
fn test_var_in_arg() {
    let mut session = new_session();
    session.env.insert("FILE".into(), "hello.txt".into());
    let out = session.execute("cat $FILE");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello world"));
}

#[test]
fn test_or_with_stderr_redirect() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent 2>/tmp/err.txt || echo ok");
    assert_eq!(out.exit_code, 0);
    assert!(output_stdout(&out).contains("ok"));
    let content = session.fs.read(Path::new("/tmp/err.txt")).unwrap();
    assert!(!content.is_empty());
}

#[test]
fn test_mkdir_with_parent_traversal() {
    let mut session = new_session();
    let out = session.execute("mkdir -p /tmp/a/b/../c");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_dir(Path::new("/tmp/a/c")));
    assert!(!session.fs.is_dir(Path::new("/tmp/a/b")));
}

#[test]
fn test_echo_with_parent_traversal() {
    let mut session = new_session();
    let out = session.execute("echo hello > /tmp/a/../b.txt");
    assert_eq!(out.exit_code, 0);
    assert!(session.fs.is_file(Path::new("/tmp/b.txt")));
    let content = session.fs.read(Path::new("/tmp/b.txt")).unwrap();
    assert_eq!(String::from_utf8_lossy(&content), "hello\n");
    assert!(!session.fs.is_dir(Path::new("/tmp/a")));
}

#[test]
fn test_rm_with_parent_traversal() {
    let mut session = new_session();
    session.execute("mkdir -p /tmp/a");
    session
        .fs
        .write(Path::new("/tmp/b.txt"), b"content")
        .unwrap();
    let out = session.execute("rm /tmp/a/../b.txt");
    assert_eq!(out.exit_code, 0);
    assert!(!session.fs.exists(Path::new("/tmp/b.txt")));
}

#[test]
fn test_bash_tool_execute_echo() {
    let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
    let (_tx, rx) = watch::channel(false);
    let ctx = hackpi_core::tools::ToolContext {
        workspace_root: std::path::PathBuf::from("/tmp"),
        signal: rx,
    };
    let params = serde_json::json!({"command": "echo hello"});
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(params, &ctx));
    match result {
        hackpi_core::tools::ToolResult::Success { content } => {
            assert_eq!(content.trim(), "hello");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[test]
fn test_bash_tool_session_persists_across_calls() {
    let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
    let (_tx, rx) = watch::channel(false);
    let ctx = hackpi_core::tools::ToolContext {
        workspace_root: std::path::PathBuf::from("/tmp"),
        signal: rx,
    };

    // First call: cd to /tmp
    let params = serde_json::json!({"command": "cd /tmp"});
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(params, &ctx));
    assert!(matches!(
        result,
        hackpi_core::tools::ToolResult::Success { .. }
    ));

    // Second call: pwd should show /tmp (session persisted)
    let params = serde_json::json!({"command": "pwd"});
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(params, &ctx));
    match result {
        hackpi_core::tools::ToolResult::Success { content } => {
            assert_eq!(content.trim(), "/tmp");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[test]
fn test_seq_with_stderr_redirect() {
    let mut session = new_session();
    let out = session.execute("cd /nonexistent 2>/tmp/seq_err.txt; echo ok");
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("ok"),
        "right operand should run after left"
    );
    let content = session.fs.read(Path::new("/tmp/seq_err.txt")).unwrap();
    let stderr_content = String::from_utf8_lossy(&content);
    assert!(
        stderr_content.contains("No such directory"),
        "stderr from left operand should be captured in file"
    );
}

#[test]
fn test_seq_exit_code_comes_from_right() {
    let mut session = new_session();
    let out = session.execute("false; echo ok");
    assert_eq!(
        out.exit_code, 0,
        "seq should return right operand's exit code"
    );
    assert!(
        out.stdout.contains("ok"),
        "right operand should run regardless of left exit code"
    );
}

#[test]
fn test_seq_last_command_exit_code() {
    let mut session = new_session();
    let out = session.execute("echo a; false");
    assert_ne!(
        out.exit_code, 0,
        "seq should return right operand's exit code when it fails"
    );
}

#[test]
fn test_ln_symlink_creates_link() {
    let mut session = new_session();
    session
        .fs
        .write(Path::new("/home/user/target.txt"), b"link target")
        .unwrap();
    let out = session.execute("ln -s target.txt link.txt");
    assert_eq!(out.exit_code, 0);
    let content = session.fs.read(Path::new("/home/user/link.txt")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&content),
        "link target",
        "symlink should resolve to target content"
    );
}

#[test]
fn test_ln_symlink_missing_target() {
    let mut session = new_session();
    let out = session.execute("ln -s nonexistent.txt link.txt");
    assert_eq!(
        out.exit_code, 0,
        "ln -s should succeed even with missing target"
    );
    // Reading a dangling symlink should fail
    let result = session.fs.read(Path::new("/home/user/link.txt"));
    assert!(result.is_err(), "reading a dangling symlink should fail");
}

#[test]
fn test_ln_hardlink() {
    let mut session = new_session();
    session
        .fs
        .write(Path::new("/home/user/original.txt"), b"hardlink content")
        .unwrap();
    let out = session.execute("ln original.txt hardlink.txt");
    assert_eq!(out.exit_code, 0);
    let content = session
        .fs
        .read(Path::new("/home/user/hardlink.txt"))
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&content), "hardlink content");
}

#[test]
fn test_ln_missing_operand() {
    let mut session = new_session();
    let out = session.execute("ln");
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("missing operand"));
}

#[test]
fn test_session_workdir_set_via_cwd() {
    let mut session = new_session();
    session.cwd = Path::new("/home").to_path_buf();
    assert_eq!(session.cwd, Path::new("/home"));

    // Test with relative path that goes to root
    let mut session = new_session();
    session.cwd = Path::new("/home").to_path_buf();
    assert_eq!(session.cwd, Path::new("/home"), "workdir should be /home");
}

#[test]
fn test_bash_tool_workdir_parameter_affects_session() {
    let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
    let (_tx, rx) = watch::channel(false);
    let ctx = hackpi_core::tools::ToolContext {
        workspace_root: std::path::PathBuf::from("/tmp"),
        signal: rx,
    };

    // Execute with workdir=/tmp - should create a file in /tmp
    let params = serde_json::json!({"command": "touch test.txt", "workdir": "/tmp"});
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(params, &ctx));
    assert!(matches!(
        result,
        hackpi_core::tools::ToolResult::Success { .. }
    ));

    // Verify file was created in /tmp using session directly
    let params = serde_json::json!({"command": "ls /tmp/test.txt"});
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(params, &ctx));
    assert!(matches!(
        result,
        hackpi_core::tools::ToolResult::Success { .. }
    ));
}

#[test]
fn test_bash_tool_workdir_with_dotdot_normalizes() {
    let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
    let (_tx, rx) = watch::channel(false);
    let ctx = hackpi_core::tools::ToolContext {
        workspace_root: std::path::PathBuf::from("/tmp"),
        signal: rx,
    };

    // Use workdir with .. traversal
    let params = serde_json::json!({"command": "pwd", "workdir": "/tmp/../home"});
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(params, &ctx));
    match result {
        hackpi_core::tools::ToolResult::Success { content } => {
            assert_eq!(content.trim(), "/home");
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[test]
fn test_home_user_bashrc_exists() {
    let fs = Box::new(InMemoryFs::with_home(&std::path::PathBuf::from("/")));
    let session = BashSession::with_workspace(fs, std::path::PathBuf::from("/"));
    assert!(session.fs.is_file(Path::new("/home/user/.bashrc")));
}

#[test]
fn test_default_inmemoryfs_does_not_have_home() {
    // The default InMemoryFs should be minimal — no /home/user
    let fs = InMemoryFs::default();
    assert!(!fs.is_dir(std::path::Path::new("/home")));
}

#[test]
fn test_readlink_on_symlink() {
    let session = new_session();
    session
        .fs
        .write(Path::new("/home/user/real.txt"), b"content")
        .unwrap();
    session
        .fs
        .symlink(
            Path::new("/home/user/real.txt"),
            Path::new("/home/user/link.txt"),
        )
        .unwrap();
    let target = session
        .fs
        .read_link(Path::new("/home/user/link.txt"))
        .unwrap();
    assert_eq!(target, Path::new("/home/user/real.txt"));
}

#[test]
fn test_cargo_help_shows_description() {
    let mut session = new_session();
    let out = session.execute("cargo --help");
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("Run cargo commands on the host"),
        "cargo --help should show cargo description, got: {}",
        out.stdout
    );
}

#[test]
fn test_cargo_registered_not_command_not_found() {
    let mut session = new_session();
    // cargo with no args should attempt host execution (not "command not found")
    let out = session.execute("cargo");
    // On systems without cargo or with a bad cwd, exit code may be non-zero,
    // but it MUST NOT be 127 (command not found)
    assert_ne!(
        out.exit_code, 127,
        "cargo should be a registered command, not 'command not found'"
    );
    // It should either succeed (0) or fail with a cargo/process error (non-127)
}

#[test]
fn test_cargo_version_runs_on_host() {
    let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
    let (_tx, rx) = tokio::sync::watch::channel(false);
    let ctx = hackpi_core::tools::ToolContext {
        workspace_root: std::path::PathBuf::from("/tmp"),
        signal: rx,
    };
    let params = serde_json::json!({"command": "cargo --version"});
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tool.execute(params, &ctx));
    match result {
        hackpi_core::tools::ToolResult::Success { content } => {
            assert!(
                content.contains("cargo"),
                "expected cargo version output, got: {content}"
            );
        }
        other => panic!("Expected Success, got: {other:?}"),
    }
}

#[test]
fn test_concurrent_reads_do_not_deadlock() {
    let fs = Box::new(InMemoryFs::default());
    fs.write(Path::new("/file1.txt"), b"content1").unwrap();
    fs.write(Path::new("/file2.txt"), b"content2").unwrap();
    fs.write(Path::new("/file3.txt"), b"content3").unwrap();

    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..10 {
            handles.push(s.spawn(|| {
                for _ in 0..100 {
                    let _r1 = fs.read(Path::new("/file1.txt"));
                    let _r2 = fs.read(Path::new("/file2.txt"));
                    let _r3 = fs.read(Path::new("/file3.txt"));
                    let _e = fs.exists(Path::new("/file1.txt"));
                    let _d = fs.is_dir(Path::new("/"));
                    let _f = fs.is_file(Path::new("/file1.txt"));
                }
            }));
        }
    });

    assert_eq!(fs.read(Path::new("/file1.txt")).unwrap(), b"content1");
    assert_eq!(fs.read(Path::new("/file2.txt")).unwrap(), b"content2");
}

// --- Parser unit tests ---

#[test]
fn test_tokenize_simple_command() {
    let tokens = tokenize("echo hello world").unwrap();
    assert_eq!(tokens, vec!["echo", "hello", "world"]);
}

#[test]
fn test_tokenize_redirect_stdout() {
    let tokens = tokenize("echo hello > /tmp/out.txt").unwrap();
    assert_eq!(tokens, vec!["echo", "hello", ">", "/tmp/out.txt"]);
}

#[test]
fn test_tokenize_redirect_stderr() {
    let tokens = tokenize("cmd 2>/tmp/err.txt").unwrap();
    assert_eq!(tokens, vec!["cmd", "2>", "/tmp/err.txt"]);
}

#[test]
fn test_tokenize_redirect_stderr_to_stdout() {
    let tokens = tokenize("cmd 2>&1").unwrap();
    assert_eq!(tokens, vec!["cmd", "2>&1"]);
}

#[test]
fn test_tokenize_redirect_stdout_to_stderr() {
    let tokens = tokenize("cmd 1>&2").unwrap();
    assert_eq!(tokens, vec!["cmd", "1>&2"]);
}

#[test]
fn test_tokenize_append_stdout() {
    let tokens = tokenize("echo a >> /tmp/log.txt").unwrap();
    assert_eq!(tokens, vec!["echo", "a", ">>", "/tmp/log.txt"]);
}

#[test]
fn test_tokenize_append_stderr() {
    let tokens = tokenize("cmd 2>>/tmp/err.log").unwrap();
    assert_eq!(tokens, vec!["cmd", "2>>", "/tmp/err.log"]);
}

#[test]
fn test_tokenize_stdin_redirect() {
    let tokens = tokenize("cat < /tmp/input.txt").unwrap();
    assert_eq!(tokens, vec!["cat", "<", "/tmp/input.txt"]);
}

#[test]
fn test_tokenize_quoted_strings() {
    let tokens = tokenize("echo \"hello world\" 'foo bar'").unwrap();
    // Single quotes are preserved in tokens so resolve_vars can detect them.
    assert_eq!(tokens, vec!["echo", "hello world", "'foo bar'"]);
}

#[test]
fn test_tokenize_single_quotes_preserve_backslash() {
    let tokens = tokenize("echo 'hello\\nworld'").unwrap();
    // Single quotes are preserved in tokens so resolve_vars can detect them.
    assert_eq!(tokens, vec!["echo", "'hello\\nworld'"]);
}

#[test]
fn test_tokenize_double_quotes_escape() {
    let tokens = tokenize("echo \"hello\\\"world\"").unwrap();
    assert_eq!(tokens, vec!["echo", "hello\"world"]);
}

#[test]
fn test_tokenize_variable() {
    let tokens = tokenize("echo $HOME").unwrap();
    assert_eq!(tokens, vec!["echo", "$HOME"]);
}

#[test]
fn test_tokenize_variable_braces() {
    let tokens = tokenize("echo ${HOME}").unwrap();
    assert_eq!(tokens, vec!["echo", "${HOME}"]);
}

#[test]
fn test_tokenize_pipe() {
    let tokens = tokenize("echo hello | wc").unwrap();
    assert_eq!(tokens, vec!["echo", "hello", "|", "wc"]);
}

#[test]
fn test_tokenize_and_operator() {
    let tokens = tokenize("echo a && echo b").unwrap();
    assert_eq!(tokens, vec!["echo", "a", "&&", "echo", "b"]);
}

#[test]
fn test_tokenize_or_operator() {
    let tokens = tokenize("false || echo fallback").unwrap();
    assert_eq!(tokens, vec!["false", "||", "echo", "fallback"]);
}

#[test]
fn test_tokenize_semicolon() {
    let tokens = tokenize("echo a; echo b").unwrap();
    assert_eq!(tokens, vec!["echo", "a", ";", "echo", "b"]);
}

#[test]
fn test_tokenize_comment() {
    let tokens = tokenize("echo hello # this is a comment").unwrap();
    assert_eq!(tokens, vec!["echo", "hello"]);
}

#[test]
fn test_tokenize_comment_no_space() {
    let tokens = tokenize("# just a comment").unwrap();
    let result: Vec<String> = vec![];
    assert_eq!(tokens, result);
}

#[test]
fn test_tokenize_empty_input() {
    let tokens = tokenize("").unwrap();
    let result: Vec<String> = vec![];
    assert_eq!(tokens, result);
}

#[test]
fn test_tokenize_whitespace_only() {
    let tokens = tokenize("   \t  ").unwrap();
    let result: Vec<String> = vec![];
    assert_eq!(tokens, result);
}

#[test]
fn test_tokenize_env_override() {
    let tokens = tokenize("FOO=bar echo hello").unwrap();
    assert_eq!(tokens, vec!["FOO=bar", "echo", "hello"]);
}

#[test]
fn test_parse_simple_command() {
    let ast = parse("echo hello world").unwrap();
    match ast {
        AstNode::Simple(cmd) => {
            assert_eq!(cmd.name, "echo");
            assert_eq!(cmd.args, vec!["hello", "world"]);
            assert!(cmd.redirects.is_empty());
        }
        _ => panic!("expected Simple command"),
    }
}

#[test]
fn test_parse_with_redirects() {
    let ast = parse("echo hello > /tmp/out.txt 2>/tmp/err.txt").unwrap();
    match ast {
        AstNode::Simple(ref cmd) => {
            assert_eq!(cmd.name, "echo");
            assert_eq!(cmd.args, vec!["hello"]);
            assert_eq!(cmd.redirects.len(), 2);
            match &cmd.redirects[0] {
                RedirectOp::Output(p) => assert_eq!(p, "/tmp/out.txt"),
                _ => panic!("expected Output redirect"),
            }
            match &cmd.redirects[1] {
                RedirectOp::Stderr(p) => assert_eq!(p, "/tmp/err.txt"),
                _ => panic!("expected Stderr redirect"),
            }
        }
        _ => panic!("expected Simple command"),
    }
}

#[test]
fn test_parse_empty_command_returns_error() {
    let result = parse("");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
}

#[test]
fn test_parse_pipeline() {
    let ast = parse("echo hello | wc").unwrap();
    match ast {
        AstNode::Pipeline(commands) => {
            assert_eq!(commands.len(), 2);
        }
        _ => panic!("expected Pipeline"),
    }
}

#[test]
fn test_parse_and_operator() {
    let ast = parse("echo a && echo b").unwrap();
    match ast {
        AstNode::And(_, _) => {}
        _ => panic!("expected And"),
    }
}

#[test]
fn test_parse_or_operator() {
    let ast = parse("false || echo b").unwrap();
    match ast {
        AstNode::Or(_, _) => {}
        _ => panic!("expected Or"),
    }
}

#[test]
fn test_parse_seq_operator() {
    let ast = parse("echo a; echo b").unwrap();
    match ast {
        AstNode::Seq(_, _) => {}
        _ => panic!("expected Seq"),
    }
}

#[test]
fn test_stderr_to_stdout_redirect_captured() {
    let ast = parse("cmd 2>&1").unwrap();
    match ast {
        AstNode::Simple(ref cmd) => {
            assert_eq!(cmd.redirects.len(), 1);
            match &cmd.redirects[0] {
                RedirectOp::StderrToStdout => {}
                other => panic!("expected StderrToStdout, got {other:?}"),
            }
        }
        _ => panic!("expected Simple command"),
    }
}

#[test]
fn test_stdout_to_stderr_redirect_captured() {
    let ast = parse("cmd 1>&2").unwrap();
    match ast {
        AstNode::Simple(ref cmd) => {
            assert_eq!(cmd.redirects.len(), 1);
            match &cmd.redirects[0] {
                RedirectOp::StderrToStdout => {}
                other => panic!("expected StderrToStdout, got {other:?}"),
            }
        }
        _ => panic!("expected Simple command"),
    }
}
