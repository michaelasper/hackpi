use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hackpi_core::agent::{Agent, AgentEvent};
use hackpi_core::api::ApiClient;
use hackpi_core::tools::ToolRegistry;
use hackpi_core::types::ApiConfig;
use hackpi_tui::app::{App, AppState};
use hackpi_tui::events::TuiEvent;
use hackpi_tui::input::InputHandler;
use hackpi_tui::ui;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const SYSTEM_PROMPT: &str = "You are hackpi, a coding agent built with Rust. \
You have access to tools for reading, writing, editing, and searching code. \
Always read a file before editing it. Verify changes compile and pass tests.";

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

    let _api = ApiClient::new(ApiConfig::default());
    let workspace_root = std::env::current_dir()?;

    let tool_registry = ToolRegistry::new();
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
                    tui_tx.send(TuiEvent::ToolDelta {
                        id: String::new(),
                        delta,
                    }).ok();
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
                    _ => {
                        if !matches!(app.state, AppState::Generating) {
                            if let Some(submitted) = input.handle_key(key) {
                                tui_tx.send(TuiEvent::Submit(submitted.clone())).ok();

                                let signal_rx_clone = signal_rx.clone();
                                let agent_tx_clone = agent_tx.clone();

                                let agent_instance = Agent::new(
                                    ApiClient::new(ApiConfig::default()),
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
