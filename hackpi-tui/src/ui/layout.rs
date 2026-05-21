use crate::theme::current_theme;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    widgets::Paragraph,
    Frame,
};

// ── Minimum terminal size gate ──────────────────────────────────────────────

/// Minimum terminal width required for the TUI to render properly.
pub const MIN_TERMINAL_WIDTH: u16 = 80;
/// Minimum terminal height required for the TUI to render properly.
pub const MIN_TERMINAL_HEIGHT: u16 = 24;

/// Returns `true` if the terminal is too small to render the full TUI layout.
///
/// When too small, callers should render [`render_too_small`] instead of the
/// normal layout to avoid clipping, overlapping, or panics.
pub fn is_too_small(area: Rect) -> bool {
    area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT
}

/// Renders a centered "terminal too small" message.
///
/// Displays the minimum required dimensions and the current terminal size so
/// the user knows how much they need to resize.
pub fn render_too_small(frame: &mut Frame, area: Rect) {
    let theme = current_theme();
    let text = format!(
        "Terminal too small\n\
         Minimum: {MIN_TERMINAL_WIDTH}x{MIN_TERMINAL_HEIGHT}\n\
         Current: {}x{}",
        area.width, area.height
    );
    let paragraph = Paragraph::new(text)
        .style(theme.status_error)
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

// ── Root layout helper ─────────────────────────────────────────────────────

/// The four major screen regions produced by [`split_root`].
pub struct RootLayout {
    /// Tab header row (height 1).
    pub header: Rect,
    /// Main content area (fills remaining vertical space).
    pub main: Rect,
    /// Input area (height 3).
    pub input: Rect,
    /// Status bar row (height 1).
    pub status: Rect,
}

/// Split the full terminal area into the four standard TUI regions.
///
/// Layout (top to bottom):
/// 1. Tab header (1 line)
/// 2. Main content (fills remaining)
/// 3. Input area (3 lines)
/// 4. Status bar (1 line)
pub fn split_root(area: Rect) -> RootLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab header
            Constraint::Min(1),    // main content
            Constraint::Length(3), // input
            Constraint::Length(1), // status bar
        ])
        .split(area);

    RootLayout {
        header: chunks[0],
        main: chunks[1],
        input: chunks[2],
        status: chunks[3],
    }
}

// ── Responsive modal helpers ───────────────────────────────────────────────

/// Return a rect of the given percentage of `area`, centered within it.
///
/// Both `width_pct` and `height_pct` are clamped to `[1, 100]`. The returned
/// rect is guaranteed to fit within `area`.
pub fn centered_rect(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let w_pct = width_pct.clamp(1, 100);
    let h_pct = height_pct.clamp(1, 100);

    let width = (area.width * w_pct / 100).max(1);
    let height = (area.height * h_pct / 100).max(1);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Return a responsive modal rect that is a percentage of `area`, capped at
/// `preferred_{width,height}`, and never exceeds `area`'s dimensions.
///
/// Use this for modals that should scale with the terminal but not exceed a
/// comfortable maximum size.
pub fn modal_rect(
    area: Rect,
    preferred_width: u16,
    preferred_height: u16,
    width_pct: u16,
    height_pct: u16,
) -> Rect {
    let max_w = (area.width * width_pct / 100).max(1);
    let max_h = (area.height * height_pct / 100).max(1);

    let width = preferred_width.min(max_w).min(area.width);
    let height = preferred_height.min(max_h).min(area.height);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}
