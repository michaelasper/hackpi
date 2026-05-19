use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use hackpi_core::agent::{Agent, AgentEvent};
use hackpi_core::api::ApiClient;
use hackpi_core::system_prompt;
use hackpi_core::tools::PermissionRequest;
use hackpi_core::tools::ToolRegistry;
use hackpi_core::types::ApiConfig;
use hackpi_guardrails::{GuardEvaluator, PermissionDecision, SettingsPaths};
use hackpi_tools::register_all_tools;
use hackpi_tui::app::{handle_slash_command, App, AppState, AppView};
use hackpi_tui::events::TuiEvent;
use hackpi_tui::input::InputHandler;
use hackpi_tui::ui;
use hackpi_vcs::{register_vcs_tools, VcsConfig};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

/// Build the system prompt from structured sections.
///
/// Uses the 4-section format (Identity, Tools, Workflow, Rules)
/// defined in `hackpi_core::system_prompt`.
fn build_system_prompt() -> String {
    system_prompt::build_system_prompt()
}

#[tokio::main]
#[allow(clippy::await_holding_lock)]
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
    let (mut signal_tx, _signal_rx) = tokio::sync::watch::channel(false);
    let cancelled = Arc::new(AtomicBool::new(false));

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

    // Spawn a background thread to read crossterm keyboard events.
    // This avoids blocking the tokio runtime and lets the main loop
    // use tokio::select! instead of busy-polling.
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<Event>();
    let key_tx_task = key_tx.clone();
    tokio::task::spawn_blocking(move || {
        loop {
            match event::read() {
                Ok(event) => {
                    if key_tx_task.send(event).is_err() {
                        break; // Main loop dropped the receiver
                    }
                }
                Err(e) => {
                    tracing::error!("Crossterm event read error: {e}");
                    break;
                }
            }
        }
    });
    // key_tx is held here so the blocking task only exits
    // when this function returns (dropping key_tx).

    let mut should_render = true;
    let mut spinner_tick = tokio::time::interval(std::time::Duration::from_millis(100));

    loop {
        // Only render when state actually changes.
        // `should_render` is set to `true` in every `tokio::select!` branch
        // and every drain loop below, so the initial render happens once and
        // subsequent renders only occur after at least one event has arrived.
        if should_render {
            terminal.draw(|f| ui::render(f, &app))?;
            should_render = false;
        }

        tokio::select! {
            _ = spinner_tick.tick() => {
                if matches!(app.state, AppState::Generating) {
                    app.loading_frame = app.loading_frame.wrapping_add(1);
                    should_render = true;
                }
            }
            Some(agent_event) = agent_rx.recv() => {
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
                should_render = true;
            }
            Some(permission_req) = permission_rx.recv() => {
                let (id, reason, response) = permission_req;
                tui_tx
                    .send(TuiEvent::PermissionRequest {
                        id,
                        reason,
                        response,
                    })
                    .ok();
                should_render = true;
            }
            Some(event) = tui_rx.recv() => {
                app.handle_event(event);
                should_render = true;
            }
            Some(key_event) = key_rx.recv() => {
                if let Event::Key(key) = key_event {
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
                    } else if app.autocomplete_visible {
                        // Autocomplete popover is active — intercept navigation/selection keys
                        match key.code {
                            KeyCode::Up => {
                                app.autocomplete_prev();
                            }
                            KeyCode::Down => {
                                app.autocomplete_next();
                            }
                            KeyCode::Tab => {
                                if let Some(cmd) = app.autocomplete_select() {
                                    let full_cmd = format!("{} ", cmd);
                                    input.buffer = full_cmd;
                                    input.cursor = input.buffer.len();
                                    app.autocomplete_visible = false;
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(cmd) = app.autocomplete_select() {
                                    let full_cmd = cmd.to_string();
                                    input.set_submit(full_cmd);
                                    app.autocomplete_visible = false;
                                } else {
                                    // No selection — pass through to normal Enter handling
                                    input.handle_key(key);
                                }
                            }
                            KeyCode::Esc => {
                                app.autocomplete_visible = false;
                            }
                            _ => {
                                input.handle_key(key);
                            }
                        }
                    } else if app.creating_task {
                        // Task creation inline prompt is active
                        match key.code {
                            KeyCode::Enter => {
                                app.submit_create_task();
                            }
                            KeyCode::Esc => {
                                app.cancel_create_task();
                            }
                            KeyCode::Backspace => {
                                app.task_create_input.pop();
                            }
                            KeyCode::Char(ch) => {
                                app.task_create_input.push(ch);
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                                if matches!(app.state, AppState::Generating) {
                                    signal_tx.send(true).ok();
                                    cancelled.store(true, Ordering::Relaxed);
                                    app.state = AppState::Interrupted;
                                }
                            }
                            KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                                app.clear();
                            }
                            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                                break;
                            }
                            KeyCode::Tab => {
                                app.cycle_view();
                                // Refresh cache when entering TaskBoard
                                if matches!(app.active_view, AppView::TaskBoard) {
                                    app.refresh_task_cache();
                                }
                            }
                            KeyCode::Up => {
                                if matches!(app.active_view, AppView::TaskBoard) {
                                    app.task_cursor_up();
                                } else if matches!(app.active_view, AppView::TaskDetail(_)) {
                                    app.task_detail_prev();
                                } else {
                                    app.auto_scroll = false;
                                    app.scroll_offset = app.scroll_offset.saturating_sub(3);
                                }
                            }
                            KeyCode::Down => {
                                if matches!(app.active_view, AppView::TaskBoard) {
                                    app.task_cursor_down();
                                } else if matches!(app.active_view, AppView::TaskDetail(_)) {
                                    app.task_detail_next();
                                } else {
                                    app.auto_scroll = false;
                                    app.scroll_offset = app.scroll_offset.saturating_add(3);
                                }
                            }
                            KeyCode::Enter => {
                                let has_input = !input.buffer.trim().is_empty();
                                if matches!(app.active_view, AppView::TaskBoard)
                                    && !has_input
                                {
                                    app.enter_task_detail();
                                } else if !matches!(app.state, AppState::Generating) {
                                    input.handle_key(key);
                                }
                            }
                            KeyCode::Esc => {
                                if !matches!(app.active_view, AppView::Conversation) {
                                    app.go_back();
                                    // Refresh cache when returning to TaskBoard
                                    if matches!(app.active_view, AppView::TaskBoard) {
                                        app.refresh_task_cache();
                                    }
                                }
                            }
                            KeyCode::Char('n') => {
                                if matches!(
                                    app.active_view,
                                    AppView::TaskBoard | AppView::TaskDetail(_)
                                ) {
                                    app.begin_create_task();
                                }
                            }
                            KeyCode::PageUp => {
                                if matches!(app.active_view, AppView::Conversation) {
                                    app.auto_scroll = false;
                                }
                                app.scroll_offset = app.scroll_offset.saturating_sub(10);
                            }
                            KeyCode::PageDown => {
                                if matches!(app.active_view, AppView::Conversation) {
                                    app.auto_scroll = false;
                                }
                                app.scroll_offset = app.scroll_offset.saturating_add(10);
                            }
                            KeyCode::Home => {
                                if matches!(app.active_view, AppView::Conversation) {
                                    app.auto_scroll = false;
                                }
                                app.scroll_offset = 0;
                            }
                            KeyCode::End => {
                                if matches!(app.active_view, AppView::Conversation) {
                                    app.auto_scroll = true;
                                }
                            }
                            _ => {
                                if !matches!(app.state, AppState::Generating) {
                                    input.handle_key(key);
                                }
                            }
                        }
                    }

                    // Sync input buffer to app.input for display after every key event
                    app.input = input.buffer.clone();
                    app.input_cursor = input.cursor;
                    // Update autocomplete visibility based on current input
                    app.update_autocomplete_state();

                    // Process any submitted text (from Enter or catch-all key handling)
                    if !matches!(app.state, AppState::Generating) {
                        if let Some(submitted) = input.last_submitted() {
                            // Check for slash commands first
                            if submitted.starts_with('/') {
                                // Guard is held across await because handle_slash_command
                                // is async and needs mutable access to the evaluator.
                                // Dropping the guard after the call is safe since no
                                // other task touches the evaluator concurrently here.
                                let mut guard = guard_evaluator.write().unwrap();
                                handle_slash_command(
                                    &submitted,
                                    &mut app,
                                    &tui_tx,
                                    &mut guard,
                                    &tools,
                                )
                                .await;
                                drop(guard);
                                // Refresh task cache after task operations on TaskBoard
                                if submitted.starts_with("/task") {
                                    if matches!(app.active_view, AppView::TaskBoard) {
                                        app.refresh_task_cache();
                                    } else if let AppView::TaskDetail(_) = app.active_view {
                                        let detail_id = match &app.active_view {
                                            AppView::TaskDetail(id) => id.clone(),
                                            _ => String::new(),
                                        };
                                        app.load_task_detail(&detail_id);
                                    }
                                }
                            } else {
                                tui_tx.send(TuiEvent::Submit(submitted.clone())).ok();

                                // Reset both cancellation sources before each new agent run.
                                // Without this, a previous Ctrl+C interrupt would latch and
                                // cause the next Agent::run to exit immediately.
                                cancelled.store(false, std::sync::atomic::Ordering::SeqCst);
                                let (new_tx, new_rx) = tokio::sync::watch::channel(false);
                                signal_tx = new_tx;

                                let signal_rx_clone = new_rx;
                                let cancelled_clone = Arc::clone(&cancelled);
                                let agent_tx_clone = agent_tx.clone();

                                let agent_instance = Agent::new(
                                    ApiClient::new(api_config.clone())?,
                                    tools.clone(),
                                    build_system_prompt(),
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
                                            cancelled_clone,
                                        )
                                        .await;
                                });
                            }
                        }
                    }
                }
                should_render = true;
            }
        }

        // Drain any additional pending events after select to batch
        // multiple events that arrived before the render cycle.
        while let Ok(agent_event) = agent_rx.try_recv() {
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
            should_render = true;
        }
        while let Ok((id, reason, response)) = permission_rx.try_recv() {
            tui_tx
                .send(TuiEvent::PermissionRequest {
                    id,
                    reason,
                    response,
                })
                .ok();
            should_render = true;
        }
        while let Ok(event) = tui_rx.try_recv() {
            app.handle_event(event);
            should_render = true;
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
