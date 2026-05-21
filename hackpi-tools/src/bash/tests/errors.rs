use tokio::sync::watch;

use super::new_session;
use super::parse;
use super::tokenize;
use super::AstNode;

// ── Command not found ──────────────────────────────────────────────

#[test]
fn test_command_not_found() {
    let mut session = new_session();
    let out = session.execute("nonexistent_cmd");
    assert_eq!(out.exit_code, 127);
    assert!(out.stderr.contains("command not found"));
}

// ── Parse errors ───────────────────────────────────────────────────

#[test]
fn test_parse_error_empty() {
    let mut session = new_session();
    let out = session.execute("");
    assert_eq!(out.exit_code, 2);
}

// ── Cancellation ───────────────────────────────────────────────────

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

// ── Command count ──────────────────────────────────────────────────

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

// ── Parser: general unit tests ─────────────────────────────────────

#[test]
fn test_tokenize_simple_command() {
    let tokens = tokenize("echo hello world").unwrap();
    assert_eq!(tokens, vec!["echo", "hello", "world"]);
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

// ── Parser: AST unit tests ─────────────────────────────────────────

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
fn test_parse_empty_command_returns_error() {
    let result = parse("");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("empty"));
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
