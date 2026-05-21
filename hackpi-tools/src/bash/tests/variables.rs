use std::path::Path;

use super::new_session;
use super::output_stdout;
use super::tokenize;

// ── Variable expansion ─────────────────────────────────────────────

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
fn test_var_in_arg() {
    let mut session = new_session();
    session.env.insert("FILE".into(), "hello.txt".into());
    let out = session.execute("cat $FILE");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("hello world"));
}

// ── Environment overrides ──────────────────────────────────────────

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

// ── Parser: variable tokenization ──────────────────────────────────

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
fn test_tokenize_env_override() {
    let tokens = tokenize("FOO=bar echo hello").unwrap();
    assert_eq!(tokens, vec!["FOO=bar", "echo", "hello"]);
}
