use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hackpi_core::agent::{Agent, AgentEvent};
use hackpi_core::api::ApiClient;
use hackpi_core::tools::PermissionRequest;
use hackpi_core::tools::ToolRegistry;
use hackpi_core::types::ApiConfig;
use hackpi_guardrails::{GuardEvaluator, PermissionDecision, SettingsPaths};
use hackpi_tools::register_all_tools;
use hackpi_tui::app::{handle_slash_command, App, AppState};
use hackpi_tui::events::TuiEvent;
use hackpi_tui::input::InputHandler;
use hackpi_tui::ui;
use hackpi_vcs::{register_vcs_tools, VcsConfig};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

const SYSTEM_PROMPT: &str = "\
# Identity
You are hackpi, a coding agent built with Rust. You help users write, debug, and refactor code.

# Tool Access
- read: view files and directories (returns LINE#HASH: prefixes for editing)
- search_grep: search codebase for regex patterns with context lines
- write: create new files (will reject writes to existing files)
- edit: modify existing files using LINE#HASH anchors from read output
- bash: execute commands in a persistent virtual shell
- git_read: inspect repository state (status, diff, log, branches, remotes)
- git_write: modify repository (add, commit, push, pull, checkout, branch, merge, rebase, stash)
- github: GitHub operations (create/list PRs, issues, releases, comments)
- task: manage tasks (create, list, show, update, transition, block, unblock)

# Workflow
1. Always read a file before editing it.
2. Use search_grep to find relevant code before making changes.
3. Verify changes compile and pass tests (cargo check / cargo test).
4. For new files, use write; for existing files, use edit with LINE#HASH anchors from read output.
5. When making commits, always git_read status first to verify changes.
6. When creating PRs, always push first, then use github pr_create.
7. Use the task tool to track your work items. Create tasks for significant features, update their state as you progress.

# Rules
- Never overwrite existing files with write — use edit instead.
- Never send LINE#HASH: prefixes or diff +/- markers in edit lines (E_INVALID_PATCH).
- Run cargo check after any Rust code change.";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Parse --god flag manually to avoid adding a CLI dependency
    let god_mode = std::env::args().any(|arg| arg == "--god");

    // Create GuardEvaluator with settings paths from current directory
    let workspace_root = std::env::current_dir()?;
    let settings_paths = SettingsPaths::new(&workspace_root);
    let guard_evaluator = Arc::new(RwLock::new(GuardEvaluator::new(
        god_mode,
        settings_paths.clone(),
    )));

    // Load rules at startup
    if let Ok(mut evaluator) = guard_evaluator.write() {
        if let Err(e) = evaluator.load_rules() {
            tracing::warn!("Failed to load guardrail rules: {e}");
        }
    }

    // TODO: Spawn hot reload thread in a future phase.
    // The HotReloader needs access to the GuardEvaluator's internal rule list
    // (Arc<RwLock<Vec<PermissionRule>>>), which requires either exposing it
    // via a method or restructuring GuardEvaluator to use shared rules internally.
    // When that's ready:
    //   let hot_reloader = HotReloader::new(rules, settings_paths);
    //   let _handle = hot_reloader.start()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tui_tx, mut tui_rx) = mpsc::unbounded_channel::<TuiEvent>();
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (permission_tx, mut permission_rx) = mpsc::unbounded_channel::<PermissionRequest>();
    let (signal_tx, signal_rx) = tokio::sync::watch::channel(false);

    let mut app = App::new();

    // Initialize task store
    let tasks_dir = workspace_root.join(".hackpi").join("tasks");
    match hackpi_tasks::JsonTaskStore::new(tasks_dir).await {
        Ok(store) => {
            app.task_store = Some(Arc::new(store));
        }
        Err(e) => {
            tracing::warn!(
                "Failed to initialize task store: {e}. Task commands will be unavailable."
            );
        }
    }

    let mut input = InputHandler::new();
    let conversation_mut = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let api_config = ApiConfig::from_env();

    let mut tool_registry = ToolRegistry::new();
    tool_registry.set_guard_evaluator(Arc::clone(&guard_evaluator));
    tool_registry.set_permission_tx(permission_tx);
    register_all_tools(&mut tool_registry, &workspace_root);
    let vcs_config = VcsConfig::from_env(&workspace_root);
    register_vcs_tools(&mut tool_registry, &workspace_root, &vcs_config);

    // Register task tool using the same store as slash commands
    if let Some(ref task_store) = app.task_store {
        let task_store_dyn: Arc<dyn hackpi_tasks::TaskStore> =
            Arc::clone(task_store) as Arc<dyn hackpi_tasks::TaskStore>;
        let task_tool = hackpi_tasks::TaskTool::new(task_store_dyn);
        task_tool.register(&mut tool_registry);
    }

    let tools = Arc::new(tool_registry);

    terminal.clear()?;

    loop {
        terminal.draw(|f| ui::render(f, &app))?;

        if let Ok(agent_event) = agent_rx.try_recv() {
            match agent_event {
                AgentEvent::TextChunk(text) => {
                    tui_tx.send(TuiEvent::StreamChunk(text)).ok();
                }
                AgentEvent::ToolCallStart { id, name } => {
                    tui_tx.send(TuiEvent::ToolCall { id, name }).ok();
                }
                AgentEvent::ToolCallEnd { id, result } => {
                    tui_tx.send(TuiEvent::ToolResult { id, result }).ok();
                }
                AgentEvent::Usage(usage) => {
                    tui_tx.send(TuiEvent::Usage(usage)).ok();
                }
                AgentEvent::Error(err) => {
                    tui_tx.send(TuiEvent::Error(err)).ok();
                }
                AgentEvent::Done => {
                    tui_tx.send(TuiEvent::Done).ok();
                }
                AgentEvent::PermissionRequest {
                    id,
                    reason,
                    response,
                } => {
                    tui_tx
                        .send(TuiEvent::PermissionRequest {
                            id,
                            reason,
                            response,
                        })
                        .ok();
                }
            }
        }

        // Check for permission requests from the dispatch permission channel
        if let Ok((id, reason, response)) = permission_rx.try_recv() {
            tui_tx
                .send(TuiEvent::PermissionRequest {
                    id,
                    reason,
                    response,
                })
                .ok();
        }

        if let Ok(event) = tui_rx.try_recv() {
            app.handle_event(event);
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                // If a permission prompt is active, intercept all keys
                if app.pending_permission.is_some() {
                    let decision = match key.code {
                        KeyCode::Char('1') => Some(PermissionDecision::AllowOnce),
                        KeyCode::Char('2') => Some(PermissionDecision::AllowSession),
                        KeyCode::Char('3') => Some(PermissionDecision::Deny),
                        KeyCode::Char('4') => Some(PermissionDecision::AlwaysAllow),
                        KeyCode::Char('5') => Some(PermissionDecision::AlwaysDeny),
                        KeyCode::Esc => Some(PermissionDecision::Deny),
                        _ => None,
                    };

                    if let Some(decision) = decision {
                        if let Some(mut prompt) = app.pending_permission.take() {
                            if let Some(sender) = prompt.response.take() {
                                sender.send(decision).ok();
                            }
                        }
                    }
                } else {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                            if matches!(app.state, AppState::Generating) {
                                signal_tx.send(true).ok();
                                app.state = AppState::Interrupted;
                            }
                        }
                        KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                            app.clear();
                        }
                        KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                            break;
                        }
                        KeyCode::PageUp => {
                            app.scroll_offset = app.scroll_offset.saturating_sub(5);
                        }
                        KeyCode::PageDown => {
                            app.scroll_offset = app.scroll_offset.saturating_add(5);
                        }
                        KeyCode::Home => {
                            app.scroll_offset = 0;
                        }
                        KeyCode::End => {
                            app.scroll_offset = usize::MAX;
                        }
                        _ => {
                            if !matches!(app.state, AppState::Generating) {
                                input.handle_key(key);
                                app.input = input.buffer.clone();
                                if let Some(submitted) = input.last_submitted() {
                                    // Check for slash commands first
                                    if submitted.starts_with('/') {
                                        let mut guard = guard_evaluator.write().unwrap();
                                        handle_slash_command(
                                            &submitted,
                                            &mut app,
                                            &tui_tx,
                                            &mut *guard,
                                            &tools,
                                        )
                                        .await;
                                    } else {
                                        tui_tx.send(TuiEvent::Submit(submitted.clone())).ok();

                                        let signal_rx_clone = signal_rx.clone();
                                        let agent_tx_clone = agent_tx.clone();

                                        let agent_instance = Agent::new(
                                            ApiClient::new(api_config.clone())?,
                                            tools.clone(),
                                            SYSTEM_PROMPT.to_string(),
                                            workspace_root.clone(),
                                        );

                                        let conversation_clone = Arc::clone(&conversation_mut);
                                        let tx_for_agent = agent_tx_clone.clone();

                                        tokio::spawn(async move {
                                            let mut conv_guard = conversation_clone.lock().await;
                                            agent_instance
                                                .run(
                                                    &submitted,
                                                    &mut conv_guard,
                                                    tx_for_agent,
                                                    signal_rx_clone,
                                                )
                                                .await;
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
