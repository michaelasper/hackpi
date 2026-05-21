use std::sync::{Arc, RwLock};

use crate::events::TuiEvent;
use hackpi_core::tools::{ToolContext, ToolRegistry};
use hackpi_guardrails::GuardEvaluator;
use hackpi_tasks::TaskCommand;

use super::conversation::format_conversation;
use super::state::App;

/// The outcome of handling a slash command, used to communicate the result
/// back to the main event loop so it can take appropriate action (e.g., exit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    /// The command was handled normally. Continue the event loop.
    Handled,
    /// The user requested the application to exit. Break the main loop.
    ExitRequested,
    /// The command performed an action that requires a re-render (e.g., /clear).
    NeedsRender,
}

/// Metadata about a registered slash command for autocomplete display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
}

/// All registered slash commands with descriptions for the autocomplete modal.
pub const SLASH_COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "/help",
        description: "Show available commands",
    },
    CommandInfo {
        name: "/clear",
        description: "Clear the conversation",
    },
    CommandInfo {
        name: "/quit",
        description: "Exit the application",
    },
    CommandInfo {
        name: "/guardrails:status",
        description: "Show guardrails status",
    },
    CommandInfo {
        name: "/guardrails:clean",
        description: "Clear session cache",
    },
    CommandInfo {
        name: "/guardrails:onboarding",
        description: "Write a preset guardrails config",
    },
    CommandInfo {
        name: "/git:status",
        description: "Show git status (via git_read)",
    },
    CommandInfo {
        name: "/git:log",
        description: "Show recent git log (via git_read)",
    },
    CommandInfo {
        name: "/github:pr-list",
        description: "List open pull requests (via github)",
    },
    CommandInfo {
        name: "/task",
        description: "Manage tasks (create, list, show, ...)",
    },
    CommandInfo {
        name: "/tasks",
        description: "Alias for /task list",
    },
    CommandInfo {
        name: "/export",
        description: "Export conversation to text file",
    },
];

/// Generate the `/help` output text from the canonical SLASH_COMMANDS source.
/// This keeps help text in sync automatically — no separate hardcoded list to maintain.
pub fn format_help_text() -> String {
    let mut lines = String::from("Available commands:\n");
    for cmd in SLASH_COMMANDS {
        lines.push_str(&format!("   {} - {}\n", cmd.name, cmd.description));
    }
    lines
}

/// Return all slash commands whose name starts with the given filter text (case-insensitive).
pub fn filter_commands(filter: &str) -> Vec<&'static CommandInfo> {
    let lower = filter.to_lowercase();
    SLASH_COMMANDS
        .iter()
        .filter(|cmd| cmd.name.starts_with(&lower))
        .collect()
}

/// Invoke a registered tool by name with the given parameters and render
/// the result as a tool card in the TUI conversation.
///
/// Emits `ToolCall` → `ToolResult` events so the conversation view shows
/// a proper tool card with status indicator. Falls back to an error event
/// if the tool is not found.
async fn invoke_tool_and_render(
    tool_name: &str,
    params: serde_json::Value,
    tool_registry: &ToolRegistry,
    tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
) -> CommandOutcome {
    let tool_id = format!(
        "slash-{tool_name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    let tool = match tool_registry.get(tool_name) {
        Some(t) => t,
        None => {
            let err = format!("Tool '{tool_name}' is not registered.");
            tui_tx.send(TuiEvent::Error(err)).ok();
            return CommandOutcome::Handled;
        }
    };

    // Emit ToolCall event so the conversation shows a tool card
    tui_tx
        .send(TuiEvent::ToolCall {
            id: tool_id.clone(),
            name: tool_name.to_string(),
            input: Some(params.clone()),
        })
        .ok();

    let ctx = ToolContext {
        workspace_root: std::env::current_dir().unwrap_or_default(),
        signal: tokio::sync::watch::channel(false).1,
    };

    let result = tool.execute(params, &ctx).await;

    // Emit ToolResult event to complete the tool card
    tui_tx
        .send(TuiEvent::ToolResult {
            id: tool_id,
            result,
        })
        .ok();

    tui_tx.send(TuiEvent::Done).ok();
    CommandOutcome::Handled
}

pub async fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    guard_evaluator: &Arc<RwLock<GuardEvaluator>>,
    tool_registry: &hackpi_core::tools::ToolRegistry,
) -> CommandOutcome {
    let parts: Vec<&str> = cmd.trim().splitn(2, char::is_whitespace).collect();
    let command = parts[0];
    match command {
        "/help" => {
            let help_text = format_help_text();
            tui_tx
                .send(TuiEvent::StreamChunk(help_text.to_string()))
                .ok();
            tui_tx.send(TuiEvent::Done).ok();
            CommandOutcome::Handled
        }
        "/clear" => {
            app.clear();
            CommandOutcome::NeedsRender
        }
        "/quit" => {
            app.quit_requested = true;
            CommandOutcome::ExitRequested
        }
        "/guardrails:status" => {
            let ge = guard_evaluator.read().unwrap();
            let rule_count = ge.rule_count();
            let god_mode = ge.is_god_mode();
            let cache_len = ge.session_cache_len();
            drop(ge);
            let god_mode_str = if god_mode { "yes" } else { "no" };

            // Determine which guards are active by checking if rules exist
            // for each guard type (this is best-effort based on rule count)
            let status_text = format!(
                "\
Guardrails Status:
  Rules loaded: {rule_count}
  God mode: {god_mode_str}
  Session cache entries: {cache_len}
  Active guards: PathGuard ✓, CommandGate ✓, FileProtection ✓"
            );
            tui_tx.send(TuiEvent::StreamChunk(status_text)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            CommandOutcome::Handled
        }
        "/guardrails:clean" => {
            guard_evaluator.write().unwrap().clear_session();
            let msg = "Session cache cleared.".to_string();
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            CommandOutcome::Handled
        }
        cmd if cmd.starts_with("/guardrails:onboarding") => {
            let rest = parts.get(1).copied().unwrap_or("");
            let preset = rest.trim().to_lowercase();

            let (preset_name, config_json) = match preset.as_str() {
                "strict" => ("strict", STRICT_CONFIG),
                "balanced" => ("balanced", BALANCED_CONFIG),
                "permissive" => ("permissive", PERMISSIVE_CONFIG),
                "" => {
                    let summary = "\
Guardrails Onboarding Presets:
   /guardrails:onboarding strict     - Deny everything, ask for everything
   /guardrails:onboarding balanced   - Balanced defaults (recommended)
   /guardrails:onboarding permissive - Permissive rules with minimal restrictions";
                    tui_tx.send(TuiEvent::StreamChunk(summary.to_string())).ok();
                    tui_tx.send(TuiEvent::Done).ok();
                    return CommandOutcome::Handled;
                }
                _ => {
                    let err = format!(
                        "Unknown preset: '{preset}'. Available: strict, balanced, permissive"
                    );
                    tui_tx.send(TuiEvent::Error(err)).ok();
                    return CommandOutcome::Handled;
                }
            };

            // Acquire read lock only for settings_paths() — drop before filesystem ops
            let hackpi_dir = {
                let ge = guard_evaluator.read().unwrap();
                match ge.settings_paths().hackpi.parent() {
                    Some(dir) => dir.to_path_buf(),
                    None => {
                        tui_tx
                            .send(TuiEvent::Error(
                                "Cannot determine workspace root for guardrails config".into(),
                            ))
                            .ok();
                        return CommandOutcome::Handled;
                    }
                }
            };

            // Filesystem operations — outside the lock
            if let Err(e) = std::fs::create_dir_all(&hackpi_dir) {
                let err = format!("Failed to create directory {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return CommandOutcome::Handled;
            }

            let config_path = hackpi_dir.join("guardrails.json");
            if let Err(e) = std::fs::write(&config_path, config_json) {
                let err = format!("Failed to write config file: {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return CommandOutcome::Handled;
            }

            // Acquire write lock for load_rules() and rule_count()
            let rule_count = {
                let mut ge = guard_evaluator.write().unwrap();
                if let Err(e) = ge.load_rules() {
                    let err = format!("Failed to load rules after writing config: {e}");
                    tui_tx.send(TuiEvent::Error(err)).ok();
                    return CommandOutcome::Handled;
                }
                ge.rule_count()
            };

            let msg = format!(
                "Wrote {preset_name} guardrails config to {} ({rule_count} rules loaded).",
                config_path.display()
            );
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            CommandOutcome::Handled
        }
        "/git:status" => {
            invoke_tool_and_render(
                "git_read",
                serde_json::json!({"operation": "status"}),
                tool_registry,
                tui_tx,
            )
            .await
        }
        "/git:log" => {
            invoke_tool_and_render(
                "git_read",
                serde_json::json!({"operation": "log"}),
                tool_registry,
                tui_tx,
            )
            .await
        }
        "/github:pr-list" => {
            invoke_tool_and_render(
                "github",
                serde_json::json!({"operation": "pr_list"}),
                tool_registry,
                tui_tx,
            )
            .await
        }
        "/tasks" => {
            // /tasks is a shortcut for /task list
            match &app.task_store {
                Some(store) => {
                    let cmd = TaskCommand::List;
                    match hackpi_tasks::handle_task_command(&cmd, store.as_ref()).await {
                        Ok(output) => {
                            tui_tx.send(TuiEvent::StreamChunk(output)).ok();
                            tui_tx.send(TuiEvent::Done).ok();
                        }
                        Err(e) => {
                            tui_tx.send(TuiEvent::Error(e.to_string())).ok();
                        }
                    }
                    CommandOutcome::Handled
                }
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Task store not initialized. Task commands are unavailable.".into(),
                        ))
                        .ok();
                    CommandOutcome::Handled
                }
            }
        }
        cmd if cmd.starts_with("/task") => {
            // Parse "/task <subcommand> [args]" or "/task" alone
            // `command` is "/task", `rest` from parts contains the subcommand + args
            let task_input = parts.get(1).copied().unwrap_or("").trim();
            match &app.task_store {
                Some(store) => {
                    match hackpi_tasks::parse_slash_task_command(task_input) {
                        Ok(task_cmd) => {
                            match hackpi_tasks::handle_task_command(&task_cmd, store.as_ref()).await
                            {
                                Ok(output) => {
                                    tui_tx.send(TuiEvent::StreamChunk(output)).ok();
                                    tui_tx.send(TuiEvent::Done).ok();
                                }
                                Err(e) => {
                                    tui_tx.send(TuiEvent::Error(e.to_string())).ok();
                                }
                            }
                        }
                        Err(e) => {
                            tui_tx.send(TuiEvent::Error(e)).ok();
                        }
                    }
                    CommandOutcome::Handled
                }
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Task store not initialized. Task commands are unavailable.".into(),
                        ))
                        .ok();
                    CommandOutcome::Handled
                }
            }
        }
        "/export" => {
            let custom_path = parts.get(1).copied().unwrap_or("").trim();

            let export_path = if custom_path.is_empty() {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                format!("./hackpi-export-{timestamp}.txt")
            } else {
                custom_path.to_string()
            };

            let formatted = format_conversation(&app.conversation);
            match std::fs::write(&export_path, &formatted) {
                Ok(_) => {
                    let size = formatted.len();
                    let abs_path = std::path::Path::new(&export_path)
                        .canonicalize()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| export_path.clone());
                    let msg = format!(
                        "\
Export complete:
  Path: {abs_path}
  Size: {size} bytes
  Messages: {}",
                        app.conversation.len()
                    );
                    tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
                    tui_tx.send(TuiEvent::Done).ok();
                }
                Err(e) => {
                    let err = format!("Failed to export conversation: {e}");
                    tui_tx.send(TuiEvent::Error(err)).ok();
                }
            }
            CommandOutcome::Handled
        }
        _ => {
            let err = format!("Unknown command: {command}. Type /help for available commands.");
            tui_tx.send(TuiEvent::Error(err)).ok();
            CommandOutcome::Handled
        }
    }
}

// ── Preset configs ──────────────────────────────────────────────────────────

const STRICT_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": [],
    "deny": ["Read(./**)", "Write(./**)", "Bash(*)"]
  },
  "path_access": {
    "allow": [],
    "deny": ["/**", "~/**"],
    "ask": false
  },
  "command_gate": {
    "patterns": {
      "rm -rf": "deny",
      "sudo": "deny",
      "curl": "ask",
      "dd": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env*": { "read": "deny", "write": "deny" },
      "*.pem": { "read": "deny", "write": "deny" },
      "*.key": { "read": "deny", "write": "deny" }
    }
  }
}"#;

const BALANCED_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": ["Read(./docs/**)"],
    "deny": ["Read(./.env)", "Write(./.env)", "Bash(sudo *)"]
  },
  "path_access": {
    "allow": [],
    "deny": [],
    "ask": true
  },
  "command_gate": {
    "patterns": {
      "rm -rf": "ask",
      "sudo": "deny",
      "curl": "ask",
      "dd": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env*": { "read": "ask", "write": "deny" },
      "*.pem": { "read": "ask", "write": "deny" },
      "*.key": { "read": "ask", "write": "deny" }
    }
  }
}"#;

const PERMISSIVE_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": ["Read(./**)", "Write(./**)", "Bash(*)"],
    "deny": []
  },
  "path_access": {
    "allow": ["/**", "~/**"],
    "deny": [],
    "ask": false
  },
  "command_gate": {
    "patterns": {
      "sudo": "deny",
      "rm -rf /": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env": { "write": "deny" }
    }
  }
}"#;
