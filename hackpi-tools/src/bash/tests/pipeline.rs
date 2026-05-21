use std::path::Path;

use super::new_session;
use super::output_stdout;
use super::parse;
use super::tokenize;
use super::AstNode;
use super::RedirectOp;

// ── || (OR) operator ───────────────────────────────────────────────

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

// ── && (AND) operator ──────────────────────────────────────────────

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

// ── ; (SEQ) operator ──────────────────────────────────────────────

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

// ── Operator + redirect combinations ───────────────────────────────

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

// ── Stdout redirect ────────────────────────────────────────────────

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

// ── Stderr redirect ────────────────────────────────────────────────

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

// ── Stderr-to-stdout merge ─────────────────────────────────────────

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

// ── Pipes ──────────────────────────────────────────────────────────

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
fn test_grep_stdin() {
    let mut session = new_session();
    let out = session.execute("echo hello world | grep hello");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello"));
}

// ── Parser: redirect tokenization ──────────────────────────────────

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

// ── Parser: operator tokenization ──────────────────────────────────

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

// ── Parser: redirect/prod AST ─────────────────────────────────────

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
