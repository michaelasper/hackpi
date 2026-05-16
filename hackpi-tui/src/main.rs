use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hackpi_core::agent::{Agent, AgentEvent};
use hackpi_core::api::ApiClient;
use hackpi_core::tools::ToolRegistry;
use hackpi_core::types::ApiConfig;
use hackpi_tools::register_all_tools;
use hackpi_tui::app::{App, AppState};
use hackpi_tui::events::TuiEvent;
use hackpi_tui::input::InputHandler;
use hackpi_tui::ui;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::Arc;
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

# Workflow
1. Always read a file before editing it.
2. Use search_grep to find relevant code before making changes.
3. Verify changes compile and pass tests (cargo check / cargo test).
4. For new files, use write; for existing files, use edit with LINE#HASH anchors from read output.

# Rules
- Never overwrite existing files with write — use edit instead.
- Never send LINE#HASH: prefixes or diff +/- markers in edit lines (E_INVALID_PATCH).
- Run cargo check after any Rust code change.";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (tui_tx, mut tui_rx) = mpsc::unbounded_channel::<TuiEvent>();
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (signal_tx, signal_rx) = tokio::sync::watch::channel(false);

    let mut app = App::new();
    let mut input = InputHandler::new();

    let api_config = ApiConfig::from_env();
    let workspace_root = std::env::current_dir()?;

    let mut tool_registry = ToolRegistry::new();
    register_all_tools(&mut tool_registry, &workspace_root);
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
                AgentEvent::ToolCallDelta(delta) => {
                    tui_tx
                        .send(TuiEvent::ToolDelta {
                            id: String::new(),
                            delta,
                        })
                        .ok();
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
            }
        }

        if let Ok(event) = tui_rx.try_recv() {
            app.handle_event(event);
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
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
                                tui_tx.send(TuiEvent::Submit(submitted.clone())).ok();

                                let signal_rx_clone = signal_rx.clone();
                                let agent_tx_clone = agent_tx.clone();

                                let agent_instance = Agent::new(
                                    ApiClient::new(api_config.clone()),
                                    tools.clone(),
                                    SYSTEM_PROMPT.to_string(),
                                    workspace_root.clone(),
                                );

                                let mut conversation_mut = Vec::new();
                                let tx_for_agent = agent_tx_clone.clone();

                                tokio::spawn(async move {
                                    agent_instance
                                        .run(
                                            &submitted,
                                            &mut conversation_mut,
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

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
