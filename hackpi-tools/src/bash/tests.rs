use std::path::Path;
use tokio::sync::watch;

use super::filesystem::{FileSystem, InMemoryFs};
use super::session::{with_session, BashOutput, BashSession};

fn new_session() -> BashSession {
    let fs = Box::new(InMemoryFs::default());
    let session = BashSession::new(fs);

    session
        .fs
        .write(
            Path::new("/home/user/hello.txt"),
            b"hello world\nline two\nline three\n",
        )
        .unwrap();
    session
        .fs
        .write(Path::new("/home/user/numbers.txt"), b"3\n1\n2\n")
        .unwrap();
    session
        .fs
        .write(Path::new("/home/user/colors.txt"), b"red\ngreen\nblue\n")
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
fn test_with_session_works_without_workspace_root() {
    let (_, rx) = watch::channel(false);
    with_session(None, Some(rx), |session| {
        let out = session.execute("echo hello");
        assert_eq!(out.exit_code, 0);
        assert_eq!(output_stdout(&out), "hello");
    });
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
