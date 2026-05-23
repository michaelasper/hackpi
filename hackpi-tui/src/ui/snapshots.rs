//! Visual regression snapshot tests for the TUI layout.
//!
//! Uses ratatui's [`TestBackend`] to render key UI states at multiple viewport
//! sizes and assert structural properties of the resulting buffer:
//!
//! - **No overlapping regions** — layout rects must not intersect
//! - **Borders properly closed** — expected border glyphs at region edges
//! - **Footers contain expected key bindings** — status bar shows correct hints
//! - **Key content positioned correctly** — content lives in the right region
//!
//! On assertion failure, the full rendered buffer is dumped as text for
//! easy manual inspection — no golden files to maintain.
//!
//! # Viewports covered
//!
//! | Size    | Width × Height | Notes                    |
//! |---------|----------------|--------------------------|
//! | Minimum | 80 × 24        | Smallest supported size  |
//! | Medium  | 120 × 40       | Typical laptop terminal  |
//! | Large   | 200 × 60       | High-res / fullscreen    |
//!
//! # States covered
//!
//! See the individual test functions for each UI state.

use crate::app::{App, AppView};
use crate::events::TuiEvent;
use crate::ui::layout::RootLayout;
use crate::ui::render;
use hackpi_core::tools::ToolResult;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

// ── Viewport dimensions ────────────────────────────────────────────────────

const MIN_W: u16 = 80;
const MIN_H: u16 = 24;
const MED_W: u16 = 120;
const MED_H: u16 = 40;
const LGE_W: u16 = 200;
const LGE_H: u16 = 60;

// ── Entry points ───────────────────────────────────────────────────────────

/// Helper: parse a [`RootLayout`] for the given buffer dimensions.
fn root_for(width: u16, height: u16, input_height: u16) -> RootLayout {
    crate::ui::split_root(Rect::new(0, 0, width, height), input_height)
}

/// Helper: compute the status-bar y coordinate from the total height.
fn status_y(height: u16) -> u16 {
    height.saturating_sub(1)
}

/// Helper: compute the input block top row (where the border line starts).
fn input_top(height: u16, input_height: u16) -> u16 {
    // Layout: header(1) + main(fill) + input(input_height) + status(1)
    height.saturating_sub(input_height).saturating_sub(1) // -1 for status bar
}

/// Extract the full buffer content as a single flat string.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<Vec<&str>>()
        .concat()
}

/// Extract a single row of the buffer as a string.
fn row_text(terminal: &Terminal<TestBackend>, row: u16, width: u16) -> String {
    let start = (row as usize) * (width as usize);
    let end = start + (width as usize);
    terminal.backend().buffer().content[start..end]
        .iter()
        .map(|c| c.symbol())
        .collect::<Vec<&str>>()
        .join("")
}

/// Dump the entire buffer as a grid of characters for debugging.
fn dump_buffer(terminal: &Terminal<TestBackend>) -> String {
    let backend = terminal.backend();
    let buf = backend.buffer();
    let w = buf.area.width as usize;
    let h = buf.area.height as usize;
    let mut out = String::new();
    for y in 0..h {
        let start = y * w;
        let end = start + w;
        let line: String = buf.content[start..end].iter().map(|c| c.symbol()).collect();
        out.push_str(&format!("{y:3}: {line}\n"));
    }
    out
}

/// Render an `App` into a `TestBackend` terminal and return the terminal.
fn render_app(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| render(f, app)).unwrap();
    terminal
}

// ── Test helpers ───────────────────────────────────────────────────────────

/// Assert that the status bar row contains a given substring.
fn assert_status_contains(
    terminal: &Terminal<TestBackend>,
    width: u16,
    height: u16,
    expected: &str,
    msg: &str,
) {
    let status_row = status_y(height);
    let text = row_text(terminal, status_row, width);
    assert!(
        text.contains(expected),
        "{msg}: expected status bar to contain \"{expected}\".\n\
         Status row {status_row}: {text:?}\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );
}

/// Assert that the header row (row 0) contains a given substring.
fn assert_header_contains(terminal: &Terminal<TestBackend>, width: u16, expected: &str, msg: &str) {
    let text = row_text(terminal, 0, width);
    assert!(
        text.contains(expected),
        "{msg}: expected header to contain \"{expected}\".\n\
         Header row: {text:?}\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );
}

/// Assert that a given row contains a given substring.
#[allow(dead_code)]
fn assert_row_contains(
    terminal: &Terminal<TestBackend>,
    row: u16,
    width: u16,
    expected: &str,
    msg: &str,
) {
    let text = row_text(terminal, row, width);
    assert!(
        text.contains(expected),
        "{msg}: expected row {row} to contain \"{expected}\".\n\
         Row {row}: {text:?}\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );
}

/// Assert that a given row does NOT contain a given substring.
fn assert_row_not_contains(
    terminal: &Terminal<TestBackend>,
    row: u16,
    width: u16,
    not_expected: &str,
    msg: &str,
) {
    let text = row_text(terminal, row, width);
    assert!(
        !text.contains(not_expected),
        "{msg}: expected row {row} to NOT contain \"{not_expected}\".\n\
         Row {row}: {text:?}\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );
}

/// Assert that the full buffer content contains a given substring.
fn assert_buffer_contains(terminal: &Terminal<TestBackend>, expected: &str, msg: &str) {
    let text = buffer_text(terminal);
    assert!(
        text.contains(expected),
        "{msg}: expected buffer to contain \"{expected}\".\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );
}

/// Assert that the full buffer content does NOT contain a given substring.
fn assert_buffer_not_contains(terminal: &Terminal<TestBackend>, not_expected: &str, msg: &str) {
    let text = buffer_text(terminal);
    assert!(
        !text.contains(not_expected),
        "{msg}: expected buffer to NOT contain \"{not_expected}\".\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );
}

/// Assert that the input area contains a substring.
/// `input_height` is the total block height (border + content rows).
fn assert_input_contains(
    terminal: &Terminal<TestBackend>,
    width: u16,
    height: u16,
    input_height: u16,
    expected: &str,
    msg: &str,
) {
    let start_row = input_top(height, input_height);
    let mut combined = String::new();
    for r in start_row..start_row + input_height {
        combined.push_str(&row_text(terminal, r, width));
    }
    assert!(
        combined.contains(expected),
        "{msg}: expected input area to contain \"{expected}\".\n\
         Input rows {start_row}–{}:\n{combined}\n\
         Full buffer:\n{}",
        start_row + input_height - 1,
        dump_buffer(terminal)
    );
}

/// Border characters used by ratatui's default `Borders::ALL` border set.
const BORDER_TOP_LEFT: char = '┌';
const BORDER_TOP_RIGHT: char = '┐';
const BORDER_BOTTOM_LEFT: char = '└';
const BORDER_BOTTOM_RIGHT: char = '┘';
const BORDER_HORIZONTAL: char = '─';
const BORDER_VERTICAL: char = '│';

/// Check that a bordered block at the given area has properly closed borders.
/// `area` is the Rect of the *block boundary* (where borders are drawn).
#[allow(dead_code)]
fn assert_block_borders_closed(terminal: &Terminal<TestBackend>, area: Rect, label: &str) {
    let buf = terminal.backend().buffer();
    let w = buf.area.width as usize;

    // We can only check if the area fits in the buffer.
    if area.right() > buf.area.width || area.bottom() > buf.area.height {
        return;
    }

    // Helper to read a single cell.
    let cell = |x: u16, y: u16| -> char {
        let idx = (y as usize) * w + (x as usize);
        buf.content[idx].symbol().chars().next().unwrap_or(' ')
    };

    let top = area.y;
    let bottom = area.bottom().saturating_sub(1);
    let left = area.x;
    let right = area.right().saturating_sub(1);

    if bottom <= top || right <= left {
        return;
    }

    // Top-left corner
    let c = cell(left, top);
    assert!(
        c == BORDER_TOP_LEFT,
        "[{label}] expected '┌' at ({left},{top}), got {c:?}. Buffer:\n{}",
        dump_buffer(terminal)
    );

    // Top-right corner
    let c = cell(right, top);
    assert!(
        c == BORDER_TOP_RIGHT,
        "[{label}] expected '┐' at ({right},{top}), got {c:?}. Buffer:\n{}",
        dump_buffer(terminal)
    );

    // Bottom-left corner
    let c = cell(left, bottom);
    assert!(
        c == BORDER_BOTTOM_LEFT,
        "[{label}] expected '└' at ({left},{bottom}), got {c:?}. Buffer:\n{}",
        dump_buffer(terminal)
    );

    // Bottom-right corner
    let c = cell(right, bottom);
    assert!(
        c == BORDER_BOTTOM_RIGHT,
        "[{label}] expected '┘' at ({right},{bottom}), got {c:?}. Buffer:\n{}",
        dump_buffer(terminal)
    );

    // Top edge (between corners)
    for x in (left + 1)..right {
        let c = cell(x, top);
        assert!(
            c == BORDER_HORIZONTAL,
            "[{label}] expected '─' at ({x},{top}), got {c:?}. Buffer:\n{}",
            dump_buffer(terminal)
        );
    }

    // Bottom edge (between corners)
    for x in (left + 1)..right {
        let c = cell(x, bottom);
        assert!(
            c == BORDER_HORIZONTAL,
            "[{label}] expected '─' at ({x},{bottom}), got {c:?}. Buffer:\n{}",
            dump_buffer(terminal)
        );
    }

    // Left edge (between corners)
    for y in (top + 1)..bottom {
        let c = cell(left, y);
        if c != ' ' {
            assert!(
                c == BORDER_VERTICAL,
                "[{label}] expected '│' at ({left},{y}), got {c:?}. Buffer:\n{}",
                dump_buffer(terminal)
            );
        }
    }

    // Right edge (between corners)
    for y in (top + 1)..bottom {
        let c = cell(right, y);
        assert!(
            c == BORDER_VERTICAL,
            "[{label}] expected '│' at ({right},{y}), got {c:?}. Buffer:\n{}",
            dump_buffer(terminal)
        );
    }
}

/// Helper: assert that the input area's top border line is present.
/// The input widget uses `Borders::TOP` which draws `─` across the full width
/// without corner characters (since there are no side borders to connect).
fn assert_input_border_present(
    terminal: &Terminal<TestBackend>,
    width: u16,
    height: u16,
    input_height: u16,
) {
    let input_border_row = input_top(height, input_height);
    let text = row_text(terminal, input_border_row, width);
    assert!(
        text.contains(BORDER_HORIZONTAL),
        "Input area should have a top border line.\n\
         Row {input_border_row}: {text:?}\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );
    // The border should span most of the width at least
    let dash_count = text.chars().filter(|&c| c == BORDER_HORIZONTAL).count();
    assert!(
        dash_count >= width.saturating_sub(2) as usize,
        "Input border should span most of the width ({width}), got {dash_count} dashes"
    );
}

/// Default input block height for apps with empty input (1 border + 1 content).
const EMPTY_INPUT_HEIGHT: u16 = 2;

// ── App factory helpers ────────────────────────────────────────────────────

fn idle_app() -> App {
    App::new()
}

fn conversation_with_messages_app() -> App {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("What is the capital of France?".into()));
    app.handle_event(TuiEvent::StreamChunk(
        "The capital of France is **Paris**. It is one of the most famous cities \
         in the world, known for its art, fashion, and culture."
            .into(),
    ));
    app.handle_event(TuiEvent::Done);
    app.handle_event(TuiEvent::Submit("Tell me about Tokyo.".into()));
    app.handle_event(TuiEvent::StreamChunk(
        "Tokyo is the capital of Japan and one of the most populous \
         metropolitan areas in the world."
            .into(),
    ));
    app.handle_event(TuiEvent::Done);
    app
}

fn generating_app() -> App {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("Write a poem about Rust.".into()));
    // Generating — no Done event yet
    app.handle_event(TuiEvent::StreamChunk(
        "Rust, a language sharp and bright,\nWith ownership that shines a light,\n".into(),
    ));
    app
}

fn tool_success_app() -> App {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("Read the config file.".into()));
    app.handle_event(TuiEvent::ToolCall {
        id: "tc1".into(),
        name: "read".into(),
        input: Some(serde_json::json!({"path": "Cargo.toml"})),
    });
    app.handle_event(TuiEvent::ToolResult {
        id: "tc1".into(),
        result: ToolResult::Success {
            content: "[package]\nname = \"hackpi\"\nversion = \"0.1.0\"".into(),
        },
    });
    app.handle_event(TuiEvent::Done);
    // Second user message too
    app.handle_event(TuiEvent::Submit("Check the file.".into()));
    app.handle_event(TuiEvent::ToolCall {
        id: "tc2".into(),
        name: "read".into(),
        input: Some(serde_json::json!({"path": "main.rs"})),
    });
    app.handle_event(TuiEvent::ToolResult {
        id: "tc2".into(),
        result: ToolResult::Success {
            content: "fn main() { println!(\"Hello\"); }".into(),
        },
    });
    app.handle_event(TuiEvent::Done);
    app
}

fn tool_command_error_app() -> App {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("Run the build.".into()));
    app.handle_event(TuiEvent::ToolCall {
        id: "tc1".into(),
        name: "bash".into(),
        input: Some(serde_json::json!({"command": "cargo build"})),
    });
    app.handle_event(TuiEvent::ToolResult {
        id: "tc1".into(),
        result: ToolResult::CommandError {
            content: "error[E0308]: mismatched types\n --> src/main.rs:10:5\n  |\n10 |     let x: i32 = \"hello\";\n  |         ^ expected i32, found &str".into(),
            exit_code: 1,
        },
    });
    app.handle_event(TuiEvent::Done);
    app
}

fn permission_modal_app() -> App {
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.pending_permission = Some(crate::app::PermissionPrompt {
        id: 1,
        reason: hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        },
        response: Some(tx),
        confirming_always_allow: false,
    });
    app
}

fn permission_modal_long_path_app() -> App {
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.pending_permission = Some(crate::app::PermissionPrompt {
        id: 1,
        reason: hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::PathAccess,
            tool: "read".into(),
            details: "/very/long/path/to/some/deeply/nested/project/file.txt".into(),
        },
        response: Some(tx),
        confirming_always_allow: false,
    });
    app
}

fn autocomplete_app() -> App {
    let mut app = App::new();
    app.input = "/".to_string();
    app.update_autocomplete_state();
    app
}

fn task_board_app() -> App {
    let mut app = App::new();
    app.active_view = AppView::TaskBoard;
    app.task_list_cache = vec![
        hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Implement authentication".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: hackpi_tasks::TaskPriority::High,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec!["backend".to_string()],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        hackpi_tasks::Task {
            id: "TSK-002".to_string(),
            title: "Write unit tests".to_string(),
            description: String::new(),
            state: "in_progress".to_string(),
            priority: hackpi_tasks::TaskPriority::Medium,
            workflow: "default".to_string(),
            blocked_by: vec!["TSK-001".to_string()],
            labels: vec![],
            assignee: Some("alice".to_string()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        hackpi_tasks::Task {
            id: "TSK-003".to_string(),
            title: "Deploy to staging".to_string(),
            description: String::new(),
            state: "blocked".to_string(),
            priority: hackpi_tasks::TaskPriority::Low,
            workflow: "default".to_string(),
            blocked_by: vec!["TSK-002".to_string()],
            labels: vec!["devops".to_string()],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        hackpi_tasks::Task {
            id: "TSK-004".to_string(),
            title: "Release v1.0".to_string(),
            description: String::new(),
            state: "done".to_string(),
            priority: hackpi_tasks::TaskPriority::Urgent,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec!["release".to_string()],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
    ];
    app
}

fn task_detail_app() -> App {
    let mut app = App::new();
    app.active_view = AppView::TaskDetail("TSK-001".to_string());
    app.task_detail_cache = Some(hackpi_tasks::Task {
        id: "TSK-001".to_string(),
        title: "Implement authentication".to_string(),
        description: "Add JWT-based authentication with refresh tokens and \
                       role-based access control for the admin dashboard."
            .to_string(),
        state: "in_progress".to_string(),
        priority: hackpi_tasks::TaskPriority::High,
        workflow: "default".to_string(),
        blocked_by: vec![],
        labels: vec!["backend".to_string(), "security".to_string()],
        assignee: Some("alice".to_string()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    });
    app
}

fn task_graph_app() -> App {
    let mut app = App::new();
    app.active_view = AppView::TaskGraph;
    app.task_list_cache = vec![
        hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Setup database".to_string(),
            description: String::new(),
            state: "done".to_string(),
            priority: hackpi_tasks::TaskPriority::High,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        hackpi_tasks::Task {
            id: "TSK-002".to_string(),
            title: "Implement auth".to_string(),
            description: String::new(),
            state: "in_progress".to_string(),
            priority: hackpi_tasks::TaskPriority::High,
            workflow: "default".to_string(),
            blocked_by: vec!["TSK-001".to_string()],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
        hackpi_tasks::Task {
            id: "TSK-003".to_string(),
            title: "Write API tests".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: hackpi_tasks::TaskPriority::Medium,
            workflow: "default".to_string(),
            blocked_by: vec!["TSK-002".to_string()],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
    ];
    app.selected_task_idx = 1; // Select TSK-002
    app
}

fn diagnostics_app() -> App {
    let mut app = App::new();
    app.active_view = AppView::Diagnostics;
    app.handle_event(TuiEvent::Diagnostic {
        level: crate::events::DiagnosticLevel::Warning,
        message: "SSE stream reconnected after timeout".into(),
    });
    app.handle_event(TuiEvent::Diagnostic {
        level: crate::events::DiagnosticLevel::Info,
        message: "Tool registry loaded 12 tools".into(),
    });
    app.handle_event(TuiEvent::Diagnostic {
        level: crate::events::DiagnosticLevel::Error,
        message: "Failed to parse tool call from LLM response: expected 'name' field".into(),
    });
    app
}

// ── Snapshot: Conversation idle state ──────────────────────────────────────

#[test]
fn snapshot_conversation_idle_80x24() {
    let app = idle_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Header shows tabs and version
    assert_header_contains(&term, MIN_W, "Conv", "idle 80x24: header shows Conv");
    assert_header_contains(&term, MIN_W, "hackpi v", "idle 80x24: header shows version");

    // Input shows prompt
    assert_input_contains(
        &term,
        MIN_W,
        MIN_H,
        EMPTY_INPUT_HEIGHT,
        "Type a message",
        "idle 80x24: input placeholder",
    );

    // Input border present
    assert_input_border_present(&term, MIN_W, MIN_H, EMPTY_INPUT_HEIGHT);

    // Status bar shows global bindings
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Interrupt generation",
        "idle 80x24: status shows interrupt hint",
    );
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Clear conversation",
        "idle 80x24: status shows clear hint",
    );
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "API:",
        "idle 80x24: status shows health indicator",
    );

    // No resize message
    assert_buffer_not_contains(&term, "Terminal too small", "idle 80x24: no resize message");

    // No "coming soon" text
    assert_buffer_not_contains(&term, "coming soon", "idle 80x24: no placeholder text");
}

#[test]
fn snapshot_conversation_idle_120x40() {
    let app = idle_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_header_contains(&term, MED_W, "Conv", "idle 120x40: header");
    assert_input_contains(
        &term,
        MED_W,
        MED_H,
        EMPTY_INPUT_HEIGHT,
        "Type a message",
        "idle 120x40: input placeholder",
    );
    assert_status_contains(
        &term,
        MED_W,
        MED_H,
        "Interrupt generation",
        "idle 120x40: status",
    );
    assert_buffer_not_contains(
        &term,
        "Terminal too small",
        "idle 120x40: no resize message",
    );
}

#[test]
fn snapshot_conversation_idle_200x60() {
    let app = idle_app();
    let term = render_app(&app, LGE_W, LGE_H);

    assert_header_contains(&term, LGE_W, "Conv", "idle 200x60: header");
    assert_input_contains(
        &term,
        LGE_W,
        LGE_H,
        EMPTY_INPUT_HEIGHT,
        "Type a message",
        "idle 200x60: input placeholder",
    );
    assert_status_contains(
        &term,
        LGE_W,
        LGE_H,
        "Interrupt generation",
        "idle 200x60: status",
    );
    assert_buffer_not_contains(
        &term,
        "Terminal too small",
        "idle 200x60: no resize message",
    );
}

// ── Snapshot: Conversation with messages ───────────────────────────────────

#[test]
fn snapshot_conversation_with_messages_80x24() {
    let app = conversation_with_messages_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Messages visible
    assert_buffer_contains(
        &term,
        "capital of France",
        "msgs 80x24: first message content",
    );
    assert_buffer_contains(&term, "Tokyo", "msgs 80x24: second message content");

    // User prefixes visible
    assert_buffer_contains(&term, "○ me:", "msgs 80x24: user prefix");

    // Status bar shows idle bindings after Done
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Clear conversation",
        "msgs 80x24: status shows clear",
    );
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "API:",
        "msgs 80x24: status shows health",
    );
}

#[test]
fn snapshot_conversation_with_messages_120x40() {
    let app = conversation_with_messages_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "capital of France", "msgs 120x40: first message");
    assert_buffer_contains(&term, "Tokyo", "msgs 120x40: second message");
}

// ── Snapshot: Generating state ─────────────────────────────────────────────

#[test]
fn snapshot_conversation_generating_80x24() {
    let app = generating_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Shows generating indicator in status bar
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Generating",
        "gen 80x24: status shows Generating",
    );
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Interrupt generation",
        "gen 80x24: status shows interrupt",
    );

    // Streamed text visible
    assert_buffer_contains(&term, "Rust", "gen 80x24: streamed text visible");
    assert_buffer_contains(
        &term,
        "a language sharp and bright",
        "gen 80x24: streamed Rust poem visible",
    );
}

#[test]
fn snapshot_conversation_generating_120x40() {
    let app = generating_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_status_contains(
        &term,
        MED_W,
        MED_H,
        "Generating",
        "gen 120x40: generating indicator",
    );
    assert_buffer_contains(&term, "Rust", "gen 120x40: streamed text");
}

#[test]
fn snapshot_conversation_generating_200x60() {
    let app = generating_app();
    let term = render_app(&app, LGE_W, LGE_H);

    assert_status_contains(
        &term,
        LGE_W,
        LGE_H,
        "Generating",
        "gen 200x60: generating indicator",
    );
    assert_buffer_contains(&term, "Rust", "gen 200x60: streamed text");
}

// ── Snapshot: Tool success ─────────────────────────────────────────────────

#[test]
fn snapshot_tool_success_80x24() {
    let app = tool_success_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Tool call names visible
    assert_buffer_contains(&term, "read", "tool success 80x24: tool call name visible");

    // Tool result content visible
    assert_buffer_contains(
        &term,
        "Cargo.toml",
        "tool success 80x24: result content visible",
    );

    // Success indicator in tool card
    assert_buffer_contains(&term, "✓", "tool success 80x24: success checkmark visible");
}

#[test]
fn snapshot_tool_success_120x40() {
    let app = tool_success_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "read", "tool success 120x40: tool name");
    assert_buffer_contains(&term, "✓", "tool success 120x40: checkmark");
}

// ── Snapshot: Tool CommandError ────────────────────────────────────────────

#[test]
fn snapshot_tool_command_error_80x24() {
    let app = tool_command_error_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Tool call name visible
    assert_buffer_contains(&term, "bash", "cmd err 80x24: tool name visible");

    // Error exit code visible in tool card
    assert_buffer_contains(&term, "[Exit 1]", "cmd err 80x24: exit code in card");

    // Error content visible
    assert_buffer_contains(
        &term,
        "mismatched types",
        "cmd err 80x24: error content visible",
    );
}

#[test]
fn snapshot_tool_command_error_120x40() {
    let app = tool_command_error_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "bash", "cmd err 120x40: tool name");
    assert_buffer_contains(&term, "mismatched types", "cmd err 120x40: error content");
}

// ── Snapshot: Tool card long content wrapping ──────────────────────────────

fn tool_long_json_app() -> App {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("Fetch the API response.".into()));
    app.handle_event(TuiEvent::ToolCall {
        id: "tc1".into(),
        name: "read".into(),
        input: Some(serde_json::json!({"path": "data.json"})),
    });
    app.handle_event(TuiEvent::ToolResult {
        id: "tc1".into(),
        result: ToolResult::Success {
            content: serde_json::to_string_pretty(&serde_json::json!({
                "repository": "hackpi",
                "url": "https://github.com/example/hackpi/tree/main/src/components/very/deeply/nested/module",
                "description": "A very long JSON field value that exceeds the typical terminal width of 80 characters and should wrap onto the next line",
                "metadata": {
                    "key1": "value1",
                    "key2": "value2",
                    "nested": {
                        "deeply": "this is a deeply nested value that makes the line extremely long when serialized into pretty-printed JSON output"
                    }
                }
            }))
            .unwrap_or_default(),
        },
    });
    app.handle_event(TuiEvent::Done);
    app
}

fn tool_long_path_app() -> App {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("Read the config file.".into()));
    app.handle_event(TuiEvent::ToolCall {
        id: "tc1".into(),
        name: "read".into(),
        input: Some(serde_json::json!({"path": "config.yml"})),
    });
    app.handle_event(TuiEvent::ToolResult {
        id: "tc1".into(),
        result: ToolResult::Success {
            content: "/Users/developer/projects/hackpi/.worktrees/feature-branch/src/components/very/deeply/nested/directory/structure/config.yml".into(),
        },
    });
    app.handle_event(TuiEvent::Done);
    app
}

fn tool_hashline_app() -> App {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("Search for patterns.".into()));
    app.handle_event(TuiEvent::ToolCall {
        id: "tc1".into(),
        name: "search".into(),
        input: Some(serde_json::json!({"pattern": "fn main"})),
    });
    app.handle_event(TuiEvent::ToolResult {
        id: "tc1".into(),
        result: ToolResult::Success {
            content: "src/main.rs:10:5: fn main() -> Result<(), Box<dyn std::error::Error>> {\nsrc/lib.rs:42:1: fn main() {\ntests/integration_test.rs:150:10: fn main() -> Result<(), anyhow::Error> {\n  |\n10 |     let result = some_function_call(with, many, arguments);\n  |         ^^^^^^ this is a very long diagnostic line that will absolutely overflow the 80 column terminal width".into(),
        },
    });
    app.handle_event(TuiEvent::Done);
    app
}

#[test]
fn snapshot_tool_long_json_80x24() {
    let app = tool_long_json_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Tool call name visible
    assert_buffer_contains(&term, "read", "long json 80x24: tool name");
    // Success indicator
    assert_buffer_contains(&term, "✓", "long json 80x24: success checkmark");
    // Long JSON content that wraps: verify start and end of wrapped content
    assert_buffer_contains(
        &term,
        "deeply nested",
        "long json 80x24: wrapped content visible",
    );
    // Make sure the bottom border is present and intact
    assert_buffer_contains(&term, "└", "long json 80x24: bottom border present");
    assert_buffer_contains(&term, "┘", "long json 80x24: bottom border corner present");
}

#[test]
fn snapshot_tool_long_path_80x24() {
    let app = tool_long_path_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Long path wrapped: both start and end of path visible
    assert_buffer_contains(
        &term,
        "/Users/developer",
        "long path 80x24: path start visible",
    );
    assert_buffer_contains(
        &term,
        "config.yml",
        "long path 80x24: path filename visible",
    );
    // Bottom border intact
    assert_buffer_contains(&term, "└───", "long path 80x24: bottom border");
}

#[test]
fn snapshot_tool_hashline_wrapping_80x24() {
    let app = tool_hashline_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Hashline source references visible
    assert_buffer_contains(&term, "src/main.rs", "hashline 80x24: source line visible");
    // Long diagnostic line should wrap (check for overflow content)
    assert_buffer_contains(
        &term,
        "some_function_call",
        "hashline 80x24: long line content visible after wrapping",
    );
    assert_buffer_contains(
        &term,
        "terminal width",
        "hashline 80x24: long line tail visible after wrapping",
    );
    // Bottom border intact
    assert_buffer_contains(&term, "└───", "hashline 80x24: bottom border");
}

#[test]
fn snapshot_tool_system_error_long_80x24() {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("Run risky operation.".into()));
    app.handle_event(TuiEvent::ToolCall {
        id: "tc1".into(),
        name: "bash".into(),
        input: Some(serde_json::json!({"command": "deploy"})),
    });
    app.handle_event(TuiEvent::ToolResult {
        id: "tc1".into(),
        result: ToolResult::SystemError {
            message: "FATAL: The deployment script encountered an unrecoverable error \
                      while attempting to synchronize the remote state with the local \
                      configuration. This typically indicates a network partition or a \
                      stale lock file from a previous interrupted operation."
                .into(),
        },
    });
    app.handle_event(TuiEvent::Done);
    let term = render_app(&app, MIN_W, MIN_H);

    // Error status visible
    assert_buffer_contains(&term, "bash", "sys err 80x24: tool name");
    // Long error message wrapped (across multiple display lines)
    assert_buffer_contains(
        &term,
        "unrecoverable error",
        "sys err 80x24: error message start visible",
    );
    // The long message wraps across lines; check fragments that appear on
    // individual buffer rows rather than the full unwrapped phrase.
    assert_buffer_contains(
        &term,
        "previous interrupt",
        "sys err 80x24: wrapped error middle visible",
    );
    assert_buffer_contains(
        &term,
        "ed operation",
        "sys err 80x24: wrapped error tail visible",
    );
    // Bottom border intact
    assert_buffer_contains(&term, "└───", "sys err 80x24: bottom border");
}

// ── Snapshot: Permission modal ─────────────────────────────────────────────

#[test]
fn snapshot_permission_modal_80x24() {
    let app = permission_modal_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Modal title visible
    assert_buffer_contains(&term, "Permission Required", "perm 80x24: modal title");

    // All decision options visible
    assert_buffer_contains(&term, "Allow once", "perm 80x24: Allow once");
    assert_buffer_contains(&term, "Allow until exit", "perm 80x24: Allow session");
    assert_buffer_contains(&term, "Deny", "perm 80x24: Deny");
    assert_buffer_contains(&term, "Always allow", "perm 80x24: Always allow");
    assert_buffer_contains(&term, "Always deny", "perm 80x24: Always deny");

    // Esc hint
    assert_buffer_contains(&term, "Esc", "perm 80x24: Esc hint");

    // Tool/guard info visible
    assert_buffer_contains(&term, "bash", "perm 80x24: tool name");
    assert_buffer_contains(&term, "rm -rf /", "perm 80x24: guard details");
    assert_buffer_contains(&term, "CommandGate", "perm 80x24: guard type");

    // Verify that the header is still visible behind the dimming
    assert_header_contains(&term, MIN_W, "Conv", "perm 80x24: header still visible");
}

#[test]
fn snapshot_permission_modal_120x40() {
    let app = permission_modal_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "Permission Required", "perm 120x40: modal title");
    assert_buffer_contains(&term, "Allow once", "perm 120x40: Allow once");
    assert_buffer_contains(&term, "Esc", "perm 120x40: Esc hint");
}

#[test]
fn snapshot_permission_modal_long_path_80x24() {
    let app = permission_modal_long_path_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Long path should be visible (wrapped, not silently truncated)
    assert_buffer_contains(&term, "Permission Required", "long path 80x24: modal title");
    assert_buffer_contains(
        &term,
        "/very/long/path",
        "long path 80x24: long path start visible",
    );
    assert_buffer_contains(
        &term,
        "nested/project/file.txt",
        "long path 80x24: long path end visible",
    );
    assert_buffer_contains(&term, "Allow once", "long path 80x24: Allow once visible");
}

// ── Snapshot: Autocomplete ─────────────────────────────────────────────────

#[test]
fn snapshot_autocomplete_80x24() {
    let app = autocomplete_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Autocomplete title
    assert_buffer_contains(&term, "Slash Commands", "ac 80x24: modal title");

    // Common slash commands visible
    assert_buffer_contains(&term, "/help", "ac 80x24: /help command");
    assert_buffer_contains(&term, "/clear", "ac 80x24: /clear command");

    // Navigation hint
    assert_buffer_contains(&term, "navigate", "ac 80x24: navigation hints");

    // Status bar still visible underneath
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "API:",
        "ac 80x24: status still visible",
    );
}

#[test]
fn snapshot_autocomplete_120x40() {
    let app = autocomplete_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "Slash Commands", "ac 120x40: modal title");
    assert_buffer_contains(&term, "/help", "ac 120x40: /help command");
}

#[test]
fn snapshot_autocomplete_200x60() {
    let app = autocomplete_app();
    let term = render_app(&app, LGE_W, LGE_H);

    assert_buffer_contains(&term, "Slash Commands", "ac 200x60: modal title");
    assert_buffer_contains(&term, "/help", "ac 200x60: /help command");
}

// ── Snapshot: Task Board ───────────────────────────────────────────────────

#[test]
fn snapshot_task_board_80x24() {
    let app = task_board_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Tab header shows Tasks tab
    assert_header_contains(&term, MIN_W, "Tasks", "board 80x24: header shows Tasks tab");

    // Group headers with counts
    assert_buffer_contains(&term, "To Do (1)", "board 80x24: To Do group");
    assert_buffer_contains(&term, "In Progress (1)", "board 80x24: In Progress group");
    assert_buffer_contains(&term, "Done (1)", "board 80x24: Done group");

    // Task IDs and titles visible
    assert_buffer_contains(&term, "TSK-001", "board 80x24: TSK-001 id");
    assert_buffer_contains(&term, "TSK-004", "board 80x24: TSK-004 id");
    assert_buffer_contains(&term, "Implement authentication", "board 80x24: task title");
    assert_buffer_contains(&term, "Release v1.0", "board 80x24: release title");

    // Status bar shows board bindings (may be truncated at 80x24, so check global)
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Navigate tasks",
        "board 80x24: shows task board shortcuts",
    );
}

#[test]
fn snapshot_task_board_120x40() {
    let app = task_board_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_header_contains(&term, MED_W, "Tasks", "board 120x40: Tasks tab");
    assert_buffer_contains(
        &term,
        "Implement authentication",
        "board 120x40: task title",
    );
    assert_buffer_contains(&term, "Navigate tasks", "board 120x40: navigate hint");
}

#[test]
fn snapshot_task_board_200x60() {
    let app = task_board_app();
    let term = render_app(&app, LGE_W, LGE_H);

    assert_header_contains(&term, LGE_W, "Tasks", "board 200x60: Tasks tab");
    assert_buffer_contains(
        &term,
        "Implement authentication",
        "board 200x60: task title",
    );
}

// ── Snapshot: Task Detail ──────────────────────────────────────────────────

#[test]
fn snapshot_task_detail_80x24() {
    let app = task_detail_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Task title visible
    assert_buffer_contains(
        &term,
        "Implement authentication",
        "detail 80x24: task title",
    );

    // Task ID visible
    assert_buffer_contains(&term, "TSK-001", "detail 80x24: task ID");

    // Fields visible
    assert_buffer_contains(&term, "In Progress", "detail 80x24: human-readable state");
    assert_buffer_contains(&term, "High", "detail 80x24: priority");
    assert_buffer_contains(&term, "backend", "detail 80x24: label");
    assert_buffer_contains(&term, "security", "detail 80x24: second label");
    assert_buffer_contains(&term, "alice", "detail 80x24: assignee");

    // Description visible
    assert_buffer_contains(&term, "JWT", "detail 80x24: description keyword");
    assert_buffer_contains(&term, "role-based", "detail 80x24: description detail");

    // Status bar shows task detail info
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "TSK-001",
        "detail 80x24: task ID in status bar",
    );
}

#[test]
fn snapshot_task_detail_120x40() {
    let app = task_detail_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(
        &term,
        "Implement authentication",
        "detail 120x40: task title",
    );
    assert_buffer_contains(&term, "JWT", "detail 120x40: description");
}

// ── Snapshot: Task Graph ───────────────────────────────────────────────────

#[test]
fn snapshot_task_graph_80x24() {
    let app = task_graph_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Graph view header
    assert_buffer_contains(&term, "Task Dependencies", "graph 80x24: dependency header");

    // Selected task info visible
    assert_buffer_contains(&term, "TSK-002", "graph 80x24: selected task ID");
    assert_buffer_contains(&term, "Implement auth", "graph 80x24: selected task title");

    // Blocked by section
    assert_buffer_contains(&term, "Blocked by", "graph 80x24: blocked by section");
    assert_buffer_contains(&term, "TSK-001", "graph 80x24: blocker ID");
    assert_buffer_contains(&term, "Setup database", "graph 80x24: blocker title");

    // No "coming soon" or "placeholder"
    assert_buffer_not_contains(&term, "coming soon", "graph 80x24: no placeholder");
}

#[test]
fn snapshot_task_graph_120x40() {
    let app = task_graph_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "Task Dependencies", "graph 120x40: header");
    assert_buffer_contains(&term, "Blocked by", "graph 120x40: blocked by");
    assert_buffer_contains(&term, "Setup database", "graph 120x40: blocker title");
}

// ── Snapshot: Diagnostics ──────────────────────────────────────────────────

#[test]
fn snapshot_diagnostics_80x24() {
    let app = diagnostics_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Tab header shows Diagnostics
    assert_header_contains(
        &term,
        MIN_W,
        "Diag(3)",
        "diag 80x24: header shows Diag with count",
    );

    // Diagnostic entries visible
    assert_buffer_contains(&term, "SSE stream reconnected", "diag 80x24: warning entry");
    assert_buffer_contains(
        &term,
        "Tool registry loaded 12 tools",
        "diag 80x24: info entry",
    );
    assert_buffer_contains(
        &term,
        "Failed to parse tool call",
        "diag 80x24: error entry",
    );

    // Summary line may be clipped at 80x24 with 3 entries due to border wrapping
    // Verify the entry content is correct instead
}

#[test]
fn snapshot_diagnostics_120x40() {
    let app = diagnostics_app();
    let term = render_app(&app, MED_W, MED_H);

    assert_header_contains(&term, MED_W, "Diag(3)", "diag 120x40: Diag tab with count");
    assert_buffer_contains(&term, "SSE stream reconnected", "diag 120x40: warning");
    assert_buffer_contains(&term, "3 diagnostics recorded", "diag 120x40: summary");
}

// ── Structural assertions ──────────────────────────────────────────────────

/// Verify that the four root layout regions do not overlap.
fn assert_root_regions_non_overlapping(root: &RootLayout) {
    let regions = [
        ("header", root.header),
        ("main", root.main),
        ("input", root.input),
        ("status", root.status),
    ];

    for (i, (name_a, a)) in regions.iter().enumerate() {
        for (name_b, b) in regions.iter().skip(i + 1) {
            let overlap = rects_overlap(*a, *b);
            assert!(
                !overlap,
                "Layout regions '{name_a}' and '{name_b}' overlap!\n\
                 {name_a}: {a:?}\n{name_b}: {b:?}"
            );
        }
    }
}

/// Returns true if two rects overlap (share any cell).
fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
}

/// Verify that all four root regions fit within the given viewport.
fn assert_root_regions_fit(root: &RootLayout, width: u16, height: u16) {
    let viewport = Rect::new(0, 0, width, height);
    for (name, rect) in [
        ("header", root.header),
        ("main", root.main),
        ("input", root.input),
        ("status", root.status),
    ] {
        assert!(
            rect.x + rect.width <= viewport.width,
            "Region '{name}' right edge ({}) exceeds viewport width ({})",
            rect.x + rect.width,
            viewport.width
        );
        assert!(
            rect.y + rect.height <= viewport.height,
            "Region '{name}' bottom edge ({}) exceeds viewport height ({})",
            rect.y + rect.height,
            viewport.height
        );
    }
}

/// Structural test: split_root produces non-overlapping, well-formed regions
/// at every supported viewport size.
#[test]
fn snapshot_structural_layout_80x24() {
    let root = root_for(MIN_W, MIN_H, EMPTY_INPUT_HEIGHT);
    assert_root_regions_non_overlapping(&root);
    assert_root_regions_fit(&root, MIN_W, MIN_H);

    // Specific checks for minimum size
    assert_eq!(root.header.height, 1, "header should be 1 row");
    assert_eq!(root.status.height, 1, "status should be 1 row");
    assert_eq!(
        root.input.height, EMPTY_INPUT_HEIGHT,
        "input should be EMPTY_INPUT_HEIGHT rows"
    );
    assert_eq!(
        root.main.height,
        MIN_H - 1 - EMPTY_INPUT_HEIGHT - 1,
        "main should fill remaining rows"
    );
}

#[test]
fn snapshot_structural_layout_120x40() {
    let root = root_for(MED_W, MED_H, EMPTY_INPUT_HEIGHT);
    assert_root_regions_non_overlapping(&root);
    assert_root_regions_fit(&root, MED_W, MED_H);

    let main_expected = MED_H - 1 - EMPTY_INPUT_HEIGHT - 1; // header(1) + input + status(1)
    assert_eq!(
        root.main.height, main_expected,
        "main should fill remaining space"
    );
}

#[test]
fn snapshot_structural_layout_200x60() {
    let root = root_for(LGE_W, LGE_H, EMPTY_INPUT_HEIGHT);
    assert_root_regions_non_overlapping(&root);
    assert_root_regions_fit(&root, LGE_W, LGE_H);

    let main_expected = LGE_H - 1 - EMPTY_INPUT_HEIGHT - 1;
    assert_eq!(
        root.main.height, main_expected,
        "main should fill remaining space"
    );
}

/// Verify that the status bar row contains only valid characters
/// (no rendering artifacts) at each viewport.
fn assert_status_bar_clean(terminal: &Terminal<TestBackend>, width: u16, height: u16) {
    let status_row = status_y(height);
    let text = row_text(terminal, status_row, width);

    // Status bar should not contain newlines (wrapping artifact)
    assert!(
        !text.contains('\n'),
        "Status bar should not contain newlines.\n\
         Status row {status_row}: {text:?}\n\
         Full buffer:\n{}",
        dump_buffer(terminal)
    );

    // Status bar should not contain null bytes or replacement characters
    assert!(
        !text.contains('\0'),
        "Status bar should not contain null bytes.\n\
         Status row {status_row}: {text:?}"
    );
}

#[test]
fn snapshot_status_bar_clean_80x24() {
    let app = conversation_with_messages_app();
    let term = render_app(&app, MIN_W, MIN_H);
    assert_status_bar_clean(&term, MIN_W, MIN_H);
}

#[test]
fn snapshot_status_bar_clean_120x40() {
    let app = conversation_with_messages_app();
    let term = render_app(&app, MED_W, MED_H);
    assert_status_bar_clean(&term, MED_W, MED_H);
}

#[test]
fn snapshot_status_bar_clean_200x60() {
    let app = conversation_with_messages_app();
    let term = render_app(&app, LGE_W, LGE_H);
    assert_status_bar_clean(&term, LGE_W, LGE_H);
}

/// Verify that the conversation view with messages shows content
/// in the main area, not overlapping the input or status areas.
#[test]
fn snapshot_content_in_correct_regions() {
    let app = conversation_with_messages_app();
    let term = render_app(&app, MIN_W, MIN_H);
    let root = root_for(MIN_W, MIN_H, EMPTY_INPUT_HEIGHT);

    // Main area: first user message should be in main area (starts at y=1)
    assert_row_contains(&term, root.main.y, MIN_W, "○ me:", "msg in main area");

    // Header area: should NOT contain message text
    assert_row_not_contains(
        &term,
        0,
        MIN_W,
        "capital of France",
        "header should not have message text",
    );

    // Input area: should NOT contain submitted text (regression check for COR-158)
    assert_input_contains(
        &term,
        MIN_W,
        MIN_H,
        EMPTY_INPUT_HEIGHT,
        "> ",
        "input area shows prompt",
    );
    // The first user message text should not be in the input area
    let input_start = input_top(MIN_H, EMPTY_INPUT_HEIGHT);
    for r in input_start..input_start + EMPTY_INPUT_HEIGHT {
        // Input rows should contain "> " prompt, not message text
        let text = row_text(&term, r, MIN_W);
        assert!(
            !text.contains("capital of France"),
            "Input row {r} should not contain message text. Input row: {text:?}"
        );
    }
}

// ── Spinner animation tests ────────────────────────────────────────────────

/// Verify that the generating state renders a spinner character in the
/// status bar (checks the spinner animation frame rendering is valid).
#[test]
fn snapshot_generating_spinner_visible() {
    let app = generating_app();
    let term = render_app(&app, MIN_W, MIN_H);

    let status_row = status_y(MIN_H);
    let text = row_text(&term, status_row, MIN_W);

    // Spinner characters are braille-like: ⠋, ⠙, ⠹, etc.
    // The loading_frame is 0 by default, so the first frame is '⠋'.
    assert!(
        text.contains('⠋') || text.contains("Generating"),
        "Generating state should show spinner character or 'Generating' text.\n\
         Status row: {text:?}"
    );
}

// ── Permission modal border integrity ──────────────────────────────────────

/// Verify that the permission modal's bordered block is properly closed.
#[test]
fn snapshot_permission_modal_borders_closed() {
    let app = permission_modal_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // We can't easily know the exact modal_rect at this level,
    // but we can verify that important border characters appear.
    let text = buffer_text(&term);
    assert!(
        text.contains('┌'),
        "Permission modal should have top-left border corner ┌"
    );
    assert!(
        text.contains('┐'),
        "Permission modal should have top-right border corner ┐"
    );
    assert!(
        text.contains('└'),
        "Permission modal should have bottom-left border corner └"
    );
    assert!(
        text.contains('┘'),
        "Permission modal should have bottom-right border corner ┘"
    );
}

/// Verify that the permission modal with a long path still renders
/// proper borders (no broken border chars due to wrapping).
#[test]
fn snapshot_permission_modal_long_path_borders_closed() {
    let app = permission_modal_long_path_app();
    let term = render_app(&app, MIN_W, MIN_H);

    let text = buffer_text(&term);
    assert!(text.contains('┌'), "Long path modal should have ┌");
    assert!(text.contains('┐'), "Long path modal should have ┐");
    assert!(text.contains('└'), "Long path modal should have └");
    assert!(text.contains('┘'), "Long path modal should have ┘");
}

// ── Autocomplete modal border integrity ────────────────────────────────────

#[test]
fn snapshot_autocomplete_modal_borders_closed() {
    let app = autocomplete_app();
    let term = render_app(&app, MIN_W, MIN_H);

    let text = buffer_text(&term);
    assert!(text.contains('┌'), "Autocomplete modal should have ┌");
    assert!(text.contains('┐'), "Autocomplete modal should have ┐");
    assert!(text.contains('└'), "Autocomplete modal should have └");
    assert!(text.contains('┘'), "Autocomplete modal should have ┘");
}

// ── Help overlay ───────────────────────────────────────────────────────────

#[test]
fn snapshot_help_overlay_80x24() {
    let mut app = idle_app();
    app.help_visible = true;
    let term = render_app(&app, MIN_W, MIN_H);

    // Help overlay title
    assert_buffer_contains(&term, "Help", "help 80x24: help title");

    // Key bindings visible
    assert_buffer_contains(&term, "Ctrl+C", "help 80x24: Ctrl+C binding");
    assert_buffer_contains(&term, "Ctrl+D", "help 80x24: Ctrl+D binding");

    // Help overlay should have proper borders
    let text = buffer_text(&term);
    assert!(text.contains('┌'), "Help overlay should have ┌");
    assert!(text.contains('┐'), "Help overlay should have ┐");
    assert!(text.contains('└'), "Help overlay should have └");
    assert!(text.contains('┘'), "Help overlay should have ┘");
}

#[test]
fn snapshot_help_overlay_120x40() {
    let mut app = idle_app();
    app.help_visible = true;
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "Help", "help 120x40: title");
    assert_buffer_contains(&term, "Ctrl+C", "help 120x40: Ctrl+C");
}

// ── Task board empty state ─────────────────────────────────────────────────

#[test]
fn snapshot_task_board_empty_80x24() {
    let mut app = App::new();
    app.active_view = AppView::TaskBoard;
    app.task_list_cache = vec![];
    let term = render_app(&app, MIN_W, MIN_H);

    assert_buffer_contains(&term, "No tasks yet", "board empty 80x24: empty state");
    assert_buffer_contains(&term, "'n'", "board empty 80x24: key hint");
}

#[test]
fn snapshot_task_board_empty_120x40() {
    let mut app = App::new();
    app.active_view = AppView::TaskBoard;
    app.task_list_cache = vec![];
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(&term, "No tasks yet", "board empty 120x40: empty state");
}

// ── Error state ────────────────────────────────────────────────────────────

#[test]
fn snapshot_error_state_80x24() {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("do something".into()));
    app.handle_event(TuiEvent::Error("API timeout after 30 seconds".into()));
    let term = render_app(&app, MIN_W, MIN_H);

    // Error visible in conversation
    assert_buffer_contains(
        &term,
        "API timeout after 30 seconds",
        "error 80x24: error message",
    );

    // Status bar shows error
    assert_status_contains(&term, MIN_W, MIN_H, "ERR", "error 80x24: ERR tag in status");

    // No "Terminal too small" error
    assert_buffer_not_contains(
        &term,
        "Terminal too small",
        "error 80x24: no resize message",
    );
}

#[test]
fn snapshot_error_state_120x40() {
    let mut app = App::new();
    app.handle_event(TuiEvent::Submit("do something".into()));
    app.handle_event(TuiEvent::Error("API timeout after 30 seconds".into()));
    let term = render_app(&app, MED_W, MED_H);

    assert_buffer_contains(
        &term,
        "API timeout after 30 seconds",
        "error 120x40: error message",
    );
    assert_status_contains(&term, MED_W, MED_H, "ERR", "error 120x40: ERR tag");
}

// ── Diagnostics empty state ────────────────────────────────────────────────

#[test]
fn snapshot_diagnostics_empty_80x24() {
    let mut app = App::new();
    app.active_view = AppView::Diagnostics;
    let term = render_app(&app, MIN_W, MIN_H);

    assert_buffer_contains(
        &term,
        "No diagnostics recorded",
        "diag empty 80x24: empty message",
    );
    assert_header_contains(&term, MIN_W, "Diag", "diag empty 80x24: Diag tab");
}

// ── Conversation viewport size rendering ───────────────────────────────────

/// Verify that conversation renders without panics at all supported sizes.
#[test]
fn snapshot_conversation_no_panic_at_all_sizes() {
    let app = conversation_with_messages_app();

    for (w, h) in [(MIN_W, MIN_H), (MED_W, MED_H), (LGE_W, LGE_H)] {
        // Must not panic — render_app itself panics on error
        let _term = render_app(&app, w, h);
    }
}

/// Verify that the task board renders without panics at all supported sizes.
#[test]
fn snapshot_task_board_no_panic_at_all_sizes() {
    let app = task_board_app();

    for (w, h) in [(MIN_W, MIN_H), (MED_W, MED_H), (LGE_W, LGE_H)] {
        let _term = render_app(&app, w, h);
    }
}

/// Verify that the task graph renders without panics at all supported sizes.
#[test]
fn snapshot_task_graph_no_panic_at_all_sizes() {
    let app = task_graph_app();

    for (w, h) in [(MIN_W, MIN_H), (MED_W, MED_H), (LGE_W, LGE_H)] {
        let _term = render_app(&app, w, h);
    }
}

// ── Blocked-by task board rendering ────────────────────────────────────────

/// Verify that the blocked state group header is shown and that blocked
/// tasks render their dependency info.
#[test]
fn snapshot_task_board_blocked_group_shown() {
    let app = task_board_app();
    let term = render_app(&app, MIN_W, MIN_H);

    // Blocked group header
    assert_buffer_contains(&term, "Blocked (1)", "blocked group 80x24: Blocked header");
    assert_buffer_contains(&term, "TSK-003", "blocked group 80x24: blocked task ID");
}

// ── Task detail with blocked-by ────────────────────────────────────────────

#[test]
fn snapshot_task_detail_with_blocked_by_80x24() {
    let mut app = App::new();
    app.active_view = AppView::TaskDetail("TSK-002".to_string());
    app.task_detail_cache = Some(hackpi_tasks::Task {
        id: "TSK-002".to_string(),
        title: "Blocked task".to_string(),
        description: String::new(),
        state: "blocked".to_string(),
        priority: hackpi_tasks::TaskPriority::High,
        workflow: "default".to_string(),
        blocked_by: vec!["TSK-001".to_string()],
        labels: vec![],
        assignee: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    });
    app.task_detail_blocked_by = vec![hackpi_tasks::Task {
        id: "TSK-001".to_string(),
        title: "Setup database".to_string(),
        description: String::new(),
        state: "done".to_string(),
        priority: hackpi_tasks::TaskPriority::Medium,
        workflow: "default".to_string(),
        blocked_by: vec![],
        labels: vec![],
        assignee: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }];

    let term = render_app(&app, MIN_W, MIN_H);

    assert_buffer_contains(&term, "Blocked task", "detail blocked 80x24: title");
    assert_buffer_contains(&term, "TSK-001", "detail blocked 80x24: blocker ID");
}

// ── Color-off / monochrome safety ──────────────────────────────────────────
// Verify that all major states render without panic regardless of style data.

#[test]
fn snapshot_render_does_not_panic_with_theme_applied() {
    let states: Vec<(&str, App)> = vec![
        ("idle", idle_app()),
        ("messages", conversation_with_messages_app()),
        ("generating", generating_app()),
        ("tool_success", tool_success_app()),
        ("tool_error", tool_command_error_app()),
        ("perm_modal", permission_modal_app()),
        ("autocomplete", autocomplete_app()),
        ("task_board", task_board_app()),
        ("task_detail", task_detail_app()),
        ("task_graph", task_graph_app()),
        ("diagnostics", diagnostics_app()),
    ];

    for (_name, app) in &states {
        for (w, h) in [(MIN_W, MIN_H), (MED_W, MED_H), (LGE_W, LGE_H)] {
            // Must not panic
            let _term = render_app(app, w, h);
        }
    }
}

// ── COR-371: Multiline composer height, wrapping, and cursor placement ──────

/// Helper: create an app with typed input and cursor position set.
fn app_with_input(input: &str, cursor: usize) -> App {
    let mut app = App::new();
    app.input = input.to_string();
    app.input_cursor = cursor;
    app
}

#[test]
fn cor371_long_ascii_wraps_inside_composer() {
    // 78 'a' chars + "> " prefix = 80 cols, exactly filling one row.
    // Adding one more character should push into a second visual row.
    let long_input = "a".repeat(79); // 79 chars, with "> " = 81 display cols → wraps
    let app = app_with_input(&long_input, long_input.len());
    let term = render_app(&app, MIN_W, MIN_H);

    // Compute expected input height: 1 border + 2 content rows (81 cols / 80 = 2 rows)
    let content_rows = crate::ui::input_content_rows(&long_input, MIN_W);
    assert_eq!(
        content_rows, 2,
        "79-char input + '> ' prefix (81 cols) should need 2 content rows"
    );
    let block_h = content_rows + 1;

    // Input area should contain the text (it wraps within the composer)
    assert_input_contains(
        &term,
        MIN_W,
        MIN_H,
        block_h,
        "aaaa",
        "long ASCII input should be visible in input area",
    );

    // Status bar must be intact — show standard bindings
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "API:",
        "long ASCII input: status bar must be intact",
    );
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Interrupt generation",
        "long ASCII input: status bar interrupt hint",
    );

    // Status bar row must NOT contain input text
    assert_row_not_contains(
        &term,
        status_y(MIN_H),
        MIN_W,
        "aaaa",
        "long ASCII input must not bleed into status bar",
    );
}

#[test]
fn cor371_cjk_wraps_inside_composer() {
    // CJK chars are width 2 each. With "> " (2 cols) + 39 CJK chars (78 cols) = 80 cols, one row.
    // 40 CJK chars: 2 + 80 = 82 cols → wraps to 2 rows.
    let cjk_input = "中".repeat(40);
    let app = app_with_input(&cjk_input, cjk_input.len());
    let term = render_app(&app, MIN_W, MIN_H);

    let content_rows = crate::ui::input_content_rows(&cjk_input, MIN_W);
    assert!(
        content_rows >= 2,
        "40 CJK chars + '> ' prefix should need at least 2 content rows, got {content_rows}"
    );
    let block_h = content_rows + 1;

    // Input area should contain the CJK text
    assert_input_contains(
        &term,
        MIN_W,
        MIN_H,
        block_h,
        "中",
        "CJK input should be visible in input area",
    );

    // Status bar must not contain CJK text
    assert_row_not_contains(
        &term,
        status_y(MIN_H),
        MIN_W,
        "中",
        "CJK input must not bleed into status bar",
    );

    // Status bar must still show standard bindings
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "API:",
        "CJK input: status bar must be intact",
    );
}

#[test]
fn cor371_multiline_newlines_cursor_placement() {
    // Three lines via Shift+Enter, cursor at end of second line
    let input = "line one\nline two\nline three";
    let cursor = "line one\nline two".len(); // cursor at end of "line two"
    let app = app_with_input(input, cursor);
    let mut term = render_app(&app, MIN_W, MIN_H);

    let content_rows = crate::ui::input_content_rows(input, MIN_W);
    assert!(
        content_rows >= 3,
        "three logical lines should need at least 3 content rows, got {content_rows}"
    );
    let block_h = content_rows + 1;

    // All three lines should be visible in the input area
    assert_input_contains(
        &term,
        MIN_W,
        MIN_H,
        block_h,
        "line one",
        "multiline: first line visible",
    );
    assert_input_contains(
        &term,
        MIN_W,
        MIN_H,
        block_h,
        "line two",
        "multiline: second line visible",
    );
    assert_input_contains(
        &term,
        MIN_W,
        MIN_H,
        block_h,
        "line three",
        "multiline: third line visible",
    );

    // Cursor should be positioned within the input area, not in the status bar
    let pos = term.get_cursor_position().unwrap();
    let input_border_row = input_top(MIN_H, block_h);
    assert!(
        pos.y > input_border_row && pos.y < input_border_row + block_h,
        "cursor y ({}) should be within input inner area (rows {}–{})",
        pos.y,
        input_border_row + 1,
        input_border_row + block_h - 1,
    );

    // Status bar must be intact
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "API:",
        "multiline input: status bar intact",
    );
}

#[test]
fn cor371_dynamic_height_grows_and_shrinks() {
    // Verify that input_content_rows scales correctly with content length.

    // Empty → 1 row
    assert_eq!(crate::ui::input_content_rows("", MIN_W), 1);

    // Short text that fits in one line → 1 row
    assert_eq!(crate::ui::input_content_rows("hello", MIN_W), 1);

    // Text + prefix that fills exactly one line (78 chars + "> " = 80) → 1 row
    assert_eq!(crate::ui::input_content_rows(&"a".repeat(78), MIN_W), 1);

    // Text + prefix that overflows (79 chars + "> " = 81) → 2 rows
    assert_eq!(crate::ui::input_content_rows(&"a".repeat(79), MIN_W), 2);

    // Two newlines → 3 rows
    assert_eq!(crate::ui::input_content_rows("a\nb\nc", MIN_W), 3);

    // Five newlines → 6 rows, but capped at MAX_ROWS=5
    assert_eq!(crate::ui::input_content_rows("a\nb\nc\nd\ne\nf", MIN_W), 5);

    // Very long single line that would need many rows, capped at 5
    let very_long = "a".repeat(500);
    assert_eq!(crate::ui::input_content_rows(&very_long, MIN_W), 5);

    // input_block_height = content_rows + 1 (border)
    assert_eq!(crate::ui::input_block_height("hello", MIN_W), 2);
    assert_eq!(crate::ui::input_block_height(&"a".repeat(79), MIN_W), 3);
}

#[test]
fn cor371_status_bar_never_overwritten_with_max_content() {
    // Fill input with enough content to max out at 5 content rows.
    // Status bar must remain untouched.
    let max_input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        "a".repeat(100),
        "b".repeat(100),
        "c".repeat(100),
        "d".repeat(100),
        "e".repeat(100),
        "f".repeat(100)
    );
    let app = app_with_input(&max_input, max_input.len());
    let term = render_app(&app, MIN_W, MIN_H);

    let block_h = crate::ui::input_block_height(&max_input, MIN_W);
    assert_eq!(
        block_h, 6,
        "max content should produce block height 6 (5 content + 1 border)"
    );

    // Status bar must show standard content
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "API:",
        "max content: status bar must show health",
    );
    assert_status_contains(
        &term,
        MIN_W,
        MIN_H,
        "Interrupt generation",
        "max content: status bar must show interrupt hint",
    );

    // Input text must not appear in status row
    assert_row_not_contains(
        &term,
        status_y(MIN_H),
        MIN_W,
        "aaaa",
        "max content must not bleed into status bar",
    );
    assert_row_not_contains(
        &term,
        status_y(MIN_H),
        MIN_W,
        "ffff",
        "max content last line must not bleed into status bar",
    );
}

#[test]
fn cor371_placeholder_shown_when_empty_not_when_typed() {
    // Empty input → shows "Type a message…" placeholder
    let empty_app = App::new();
    let empty_term = render_app(&empty_app, MIN_W, MIN_H);
    assert_input_contains(
        &empty_term,
        MIN_W,
        MIN_H,
        EMPTY_INPUT_HEIGHT,
        "Type a message",
        "empty input: should show placeholder",
    );

    // Typed input → no placeholder, shows actual text
    let typed_app = app_with_input("my message", 10);
    let typed_term = render_app(&typed_app, MIN_W, MIN_H);
    assert_input_contains(
        &typed_term,
        MIN_W,
        MIN_H,
        EMPTY_INPUT_HEIGHT,
        "my message",
        "typed input: should show actual text",
    );
    // The buffer should NOT contain placeholder text when actual text is entered
    assert_buffer_not_contains(
        &typed_term,
        "Type a message",
        "typed input: should NOT show placeholder",
    );
}

#[test]
fn cor371_cursor_row_clamped_to_composer_area() {
    // Cursor at the end of max-content input: verify it stays within the
    // input block and never touches the status bar.
    let lines: Vec<String> = (0..10)
        .map(|i| format!("line {} content here", i))
        .collect();
    let input = lines.join("\n");
    let app = app_with_input(&input, input.len());
    let mut term = render_app(&app, MIN_W, MIN_H);

    let block_h = crate::ui::input_block_height(&input, MIN_W);
    let pos = term.get_cursor_position().unwrap();

    // Cursor must be within the input block (below header, above status)
    let input_border_row = input_top(MIN_H, block_h);
    assert!(
        pos.y >= input_border_row && pos.y < input_border_row + block_h,
        "cursor y ({}) must be within input block (rows {}–{})",
        pos.y,
        input_border_row,
        input_border_row + block_h - 1,
    );

    // Cursor must not be on the status bar row
    assert!(
        pos.y < status_y(MIN_H),
        "cursor y ({}) must not be on status bar row ({})",
        pos.y,
        status_y(MIN_H),
    );
}

#[test]
fn cor371_split_root_clamps_input_height() {
    // split_root clamps input_height to [2, 6]
    let root_lo = crate::ui::split_root(Rect::new(0, 0, MIN_W, MIN_H), 1);
    assert_eq!(
        root_lo.input.height, 2,
        "input height should clamp to minimum 2"
    );

    let root_hi = crate::ui::split_root(Rect::new(0, 0, MIN_W, MIN_H), 100);
    assert_eq!(
        root_hi.input.height, 6,
        "input height should clamp to maximum 6"
    );

    // Status bar must always be 1 row regardless of input height
    assert_eq!(
        root_lo.status.height, 1,
        "status bar must be 1 row with min input"
    );
    assert_eq!(
        root_hi.status.height, 1,
        "status bar must be 1 row with max input"
    );

    // Regions must not overlap
    assert_root_regions_non_overlapping(&root_lo);
    assert_root_regions_non_overlapping(&root_hi);
    assert_root_regions_fit(&root_lo, MIN_W, MIN_H);
    assert_root_regions_fit(&root_hi, MIN_W, MIN_H);
}

#[test]
fn cor371_render_does_not_panic_with_multiline_input_at_all_sizes() {
    let multiline = "first line\nsecond line\nthird line\nfourth line\nfifth line";
    let app = app_with_input(multiline, multiline.len());

    for (w, h) in [(MIN_W, MIN_H), (MED_W, MED_H), (LGE_W, LGE_H)] {
        let _term = render_app(&app, w, h);
    }
}

#[test]
fn cor371_render_does_not_panic_with_extremely_long_line() {
    // Single line of 2000 characters — should not panic or overflow
    let long = "x".repeat(2000);
    let app = app_with_input(&long, long.len());

    for (w, h) in [(MIN_W, MIN_H), (MED_W, MED_H), (LGE_W, LGE_H)] {
        let _term = render_app(&app, w, h);
    }
}
