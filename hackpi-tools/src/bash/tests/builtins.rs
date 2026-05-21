use tokio::sync::watch;

use super::new_session;
use super::output_stdout;
use super::BashTool;
use hackpi_core::tools::Tool;

// ── echo ───────────────────────────────────────────────────────────

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

// ── pwd ────────────────────────────────────────────────────────────

#[test]
fn test_pwd() {
    let mut session = new_session();
    let out = session.execute("pwd");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "/home/user");
}

// ── cd ─────────────────────────────────────────────────────────────

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

// ── env ────────────────────────────────────────────────────────────

#[test]
fn test_env() {
    let mut session = new_session();
    let out = session.execute("env");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.contains("HOME=/home/user"));
    assert!(out.stdout.contains("USER=user"));
}

// ── export ─────────────────────────────────────────────────────────

#[test]
fn test_export() {
    let mut session = new_session();
    let out = session.execute("export FOO=bar");
    assert_eq!(out.exit_code, 0);
    assert_eq!(session.env.get("FOO").unwrap(), "bar");
}

// ── help ───────────────────────────────────────────────────────────

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

// ── cargo (host command passthrough) ───────────────────────────────

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

// ── which ──────────────────────────────────────────────────────────

#[test]
fn test_which_finds_builtin() {
    let mut session = new_session();
    let out = session.execute("which echo");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "echo");
}

#[test]
fn test_which_not_found() {
    let mut session = new_session();
    let out = session.execute("which nonexistent_cmd");
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stdout.is_empty(),
        "stdout should be empty when not found"
    );
}

#[test]
fn test_which_no_args_exits_zero() {
    let mut session = new_session();
    let out = session.execute("which");
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.is_empty(), "stdout should be empty with no args");
}

#[test]
fn test_which_multiple_some_found() {
    let mut session = new_session();
    let out = session.execute("which echo ls nonexistent");
    assert_eq!(out.exit_code, 0, "should exit 0 if at least one is found");
    let stdout = output_stdout(&out);
    assert!(stdout.contains("echo"), "should list echo");
    assert!(stdout.contains("ls"), "should list ls");
    assert!(
        !stdout.contains("nonexistent"),
        "should not print unknown commands"
    );
}

#[test]
fn test_which_help_shows_description() {
    let mut session = new_session();
    let out = session.execute("which --help");
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("Locate a built-in command"),
        "which --help should show description, got: {}",
        out.stdout
    );
}

#[test]
fn test_which_registered_not_command_not_found() {
    let mut session = new_session();
    let out = session.execute("which");
    assert_ne!(
        out.exit_code, 127,
        "which should be a registered command, not 'command not found'"
    );
}

#[test]
fn test_which_finds_which_itself() {
    let mut session = new_session();
    let out = session.execute("which which");
    assert_eq!(out.exit_code, 0);
    assert_eq!(output_stdout(&out), "which");
}

// ── BashTool integration ───────────────────────────────────────────

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
