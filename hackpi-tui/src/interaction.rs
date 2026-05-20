/// Focus model and contextual help system for the hackpi TUI.
///
/// This module defines the explicit focus targets, overlay states, key binding
/// table, and helper functions for generating context-aware footer hints and
/// help overlays. It replaces the ad-hoc conditional routing in the event loop
/// and the hard-coded status bar strings in the renderer.
use crate::app::App;

/// Which region of the TUI has active focus for keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// Text input composer for the conversation.
    ConversationInput,
    /// Scrollback area in the conversation view.
    ConversationScrollback,
    /// Task board list navigation.
    TaskBoard,
    /// Task detail field navigation.
    TaskDetail,
}

/// Transient overlay that traps focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    /// Slash-command autocomplete popover above the input.
    SlashCommandPalette,
    /// Permission prompt modal (Allow/Deny).
    PermissionPrompt,
    /// Inline task creation prompt in the input area.
    TaskCreatePrompt,
    /// Contextual key-binding help overlay.
    HelpOverlay,
}

/// The key-binding context determines which set of bindings applies.
///
/// Derived from the current focus target, active overlay (if any), and app
/// state (Resting vs Generating).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    /// Bindings available in every state (Ctrl+C, Ctrl+L, Ctrl+D, ?).
    Global,
    /// Text composer is focused and the app is resting.
    Composer,
    /// Conversation scrollback is focused (arrow keys for scrolling).
    Conversation,
    /// Task board list is focused (up/down to navigate, Enter for detail, n to create).
    TaskBoard,
    /// Task detail view is focused (up/down to navigate fields).
    TaskDetail,
    /// Slash-command autocomplete popover is open.
    SlashCommandPalette,
    /// Permission prompt modal is open.
    PermissionPrompt,
    /// Inline task creation prompt is active.
    TaskCreatePrompt,
    /// Help overlay is open.
    HelpOverlay,
}

/// A single entry in the static key binding table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    /// The context in which this binding is active.
    pub context: KeyContext,
    /// Human-readable key name (e.g. "Ctrl+C", "Enter", "?").
    pub key: &'static str,
    /// Short description of the action.
    pub action: &'static str,
    /// Whether this binding should appear in the one-line footer.
    pub footer: bool,
}

/// Master key binding table.
///
/// This is the single source of truth for all keyboard shortcuts in the TUI.
/// The footer rendering and help overlay both derive their content from this
/// table, filtered by the current [`KeyContext`].
pub const KEY_BINDINGS: &[KeyBinding] = &[
    // ── Global ──────────────────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::Global,
        key: "Ctrl+C",
        action: "Interrupt generation",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::Global,
        key: "Ctrl+L",
        action: "Clear conversation",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::Global,
        key: "Ctrl+D",
        action: "Exit",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::Global,
        key: "?",
        action: "Show context help",
        footer: false,
    },
    // ── Composer ────────────────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::Composer,
        key: "Enter",
        action: "Submit message",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::Composer,
        key: "Esc",
        action: "Clear input",
        footer: true,
    },
    // ── Conversation scrollback ─────────────────────────────────────────
    KeyBinding {
        context: KeyContext::Conversation,
        key: "Up/Down",
        action: "Scroll",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::Conversation,
        key: "PgUp/PgDn",
        action: "Scroll faster",
        footer: false,
    },
    KeyBinding {
        context: KeyContext::Conversation,
        key: "Home",
        action: "Scroll to top",
        footer: false,
    },
    KeyBinding {
        context: KeyContext::Conversation,
        key: "End",
        action: "Scroll to bottom",
        footer: false,
    },
    // ── TaskBoard ───────────────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::TaskBoard,
        key: "Up/Down",
        action: "Navigate tasks",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::TaskBoard,
        key: "Enter",
        action: "View task detail",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::TaskBoard,
        key: "n",
        action: "Create task",
        footer: true,
    },
    // ── TaskDetail ──────────────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::TaskDetail,
        key: "Up/Down",
        action: "Navigate fields",
        footer: true,
    },
    KeyBinding {
        context: KeyContext::TaskDetail,
        key: "Esc",
        action: "Go back",
        footer: true,
    },
    // ── HelpOverlay ─────────────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::HelpOverlay,
        key: "Esc",
        action: "Close help",
        footer: false,
    },
    // ── SlashCommandPalette ─────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::SlashCommandPalette,
        key: "Up/Down",
        action: "Navigate commands",
        footer: false,
    },
    KeyBinding {
        context: KeyContext::SlashCommandPalette,
        key: "Tab",
        action: "Select command",
        footer: false,
    },
    KeyBinding {
        context: KeyContext::SlashCommandPalette,
        key: "Enter",
        action: "Submit command",
        footer: false,
    },
    KeyBinding {
        context: KeyContext::SlashCommandPalette,
        key: "Esc",
        action: "Close palette",
        footer: false,
    },
    // ── PermissionPrompt ────────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::PermissionPrompt,
        key: "1-5",
        action: "Choose decision",
        footer: false,
    },
    KeyBinding {
        context: KeyContext::PermissionPrompt,
        key: "Esc",
        action: "Deny",
        footer: false,
    },
    // ── TaskCreatePrompt ────────────────────────────────────────────────
    KeyBinding {
        context: KeyContext::TaskCreatePrompt,
        key: "Enter",
        action: "Create task",
        footer: false,
    },
    KeyBinding {
        context: KeyContext::TaskCreatePrompt,
        key: "Esc",
        action: "Cancel",
        footer: false,
    },
];

/// Return the subset of [`KEY_BINDINGS`] that should appear in the one-line
/// footer for the given context.
///
/// Only bindings marked with `footer: true` are included.
pub fn footer_bindings(context: KeyContext) -> Vec<&'static KeyBinding> {
    KEY_BINDINGS
        .iter()
        .filter(|kb| {
            // Include bindings whose context matches, or Global bindings that are
            // always relevant (they're always in scope).
            (kb.context == context || kb.context == KeyContext::Global) && kb.footer
        })
        .collect()
}

/// Return ALL key bindings relevant to the given context, for display in
/// the help overlay. This includes both footer and non-footer bindings,
/// sorted with footer bindings first.
pub fn help_bindings(context: KeyContext) -> Vec<&'static KeyBinding> {
    let mut result: Vec<&'static KeyBinding> = KEY_BINDINGS
        .iter()
        .filter(|kb| kb.context == context || kb.context == KeyContext::Global)
        .collect();
    // Sort: footer-first, then by key name for stable ordering.
    result.sort_by(|a, b| b.footer.cmp(&a.footer).then_with(|| a.key.cmp(b.key)));
    result
}

/// Determine the active [`KeyContext`] from the app state.
///
/// Overlays take precedence over focus targets. If no overlay is active,
/// the context is derived from the current focus target and app state.
pub fn app_key_context(app: &App) -> KeyContext {
    // Check overlays first (they trap focus).
    if app.autocomplete_visible {
        return KeyContext::SlashCommandPalette;
    }
    if app.pending_permission.is_some() {
        return KeyContext::PermissionPrompt;
    }
    if app.creating_task {
        return KeyContext::TaskCreatePrompt;
    }

    // No overlay — derive from focus target and state.
    match focus_target(app) {
        FocusTarget::ConversationInput => {
            if !app.ui_status.is_active() {
                KeyContext::Composer
            } else {
                KeyContext::Conversation
            }
        }
        FocusTarget::ConversationScrollback => KeyContext::Conversation,
        FocusTarget::TaskBoard => KeyContext::TaskBoard,
        FocusTarget::TaskDetail => KeyContext::TaskDetail,
    }
}

/// Determine the active [`FocusTarget`] from the app state.
///
/// Does NOT consider overlays — use [`active_overlay`] for that.
pub fn focus_target(app: &App) -> FocusTarget {
    match &app.active_view {
        crate::app::AppView::Conversation => {
            // If the app is generating, the focus is on the scrollback
            // (input is disabled). If idle, focus is on the input.
            if app.ui_status.is_active() {
                FocusTarget::ConversationScrollback
            } else {
                FocusTarget::ConversationInput
            }
        }
        crate::app::AppView::TaskBoard => FocusTarget::TaskBoard,
        crate::app::AppView::TaskDetail(_) => FocusTarget::TaskDetail,
        crate::app::AppView::TaskGraph => FocusTarget::ConversationInput,
    }
}

/// Determine the active [`OverlayKind`] if any overlay is visible.
pub fn active_overlay(app: &App) -> Option<OverlayKind> {
    if app.help_visible {
        return Some(OverlayKind::HelpOverlay);
    }
    if app.pending_permission.is_some() {
        return Some(OverlayKind::PermissionPrompt);
    }
    if app.autocomplete_visible {
        return Some(OverlayKind::SlashCommandPalette);
    }
    if app.creating_task {
        return Some(OverlayKind::TaskCreatePrompt);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AppView, UiStatus};

    // ── FocusTarget tests ──────────────────────────────────────────────

    #[test]
    fn test_focus_target_conversation_resting_is_input() {
        let app = App::new();
        assert_eq!(
            focus_target(&app),
            FocusTarget::ConversationInput,
            "resting conversation should focus input"
        );
    }

    #[test]
    fn test_focus_target_conversation_generating_is_scrollback() {
        let mut app = App::new();
        app.ui_status = UiStatus::Generating;
        assert_eq!(
            focus_target(&app),
            FocusTarget::ConversationScrollback,
            "generating conversation should focus scrollback"
        );
    }

    #[test]
    fn test_focus_target_conversation_running_tool_is_scrollback() {
        let mut app = App::new();
        app.ui_status = UiStatus::RunningTool {
            name: "bash".into(),
        };
        assert_eq!(
            focus_target(&app),
            FocusTarget::ConversationScrollback,
            "running tool should focus scrollback"
        );
    }

    #[test]
    fn test_focus_target_task_board_is_task_board() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        assert_eq!(
            focus_target(&app),
            FocusTarget::TaskBoard,
            "task board view should focus task board"
        );
    }

    #[test]
    fn test_focus_target_task_detail_is_task_detail() {
        let mut app = App::new();
        app.active_view = AppView::TaskDetail("TSK-001".to_string());
        assert_eq!(
            focus_target(&app),
            FocusTarget::TaskDetail,
            "task detail view should focus task detail"
        );
    }

    #[test]
    fn test_focus_target_task_graph_falls_back_to_input() {
        let mut app = App::new();
        app.active_view = AppView::TaskGraph;
        assert_eq!(
            focus_target(&app),
            FocusTarget::ConversationInput,
            "graph placeholder should fall back to input focus"
        );
    }

    // ── ActiveOverlay tests ────────────────────────────────────────────

    #[test]
    fn test_active_overlay_none_when_idle() {
        let app = App::new();
        assert_eq!(active_overlay(&app), None, "idle app has no overlay");
    }

    #[test]
    fn test_active_overlay_permission_prompt() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason: hackpi_guardrails::GuardReason {
                guard: hackpi_guardrails::GuardType::CommandGate,
                tool: "bash".into(),
                details: "rm -rf /".into(),
            },
            response: Some(tx),
        });
        assert_eq!(active_overlay(&app), Some(OverlayKind::PermissionPrompt));
    }

    #[test]
    fn test_active_overlay_help_overlay() {
        let mut app = App::new();
        app.help_visible = true;
        assert_eq!(active_overlay(&app), Some(OverlayKind::HelpOverlay));
    }

    #[test]
    fn test_active_overlay_autocomplete() {
        let mut app = App::new();
        app.input = "/".to_string();
        // Simulate autocomplete being visible
        app.autocomplete_visible = true;
        assert_eq!(active_overlay(&app), Some(OverlayKind::SlashCommandPalette));
    }

    #[test]
    fn test_active_overlay_task_create() {
        let mut app = App::new();
        app.creating_task = true;
        assert_eq!(active_overlay(&app), Some(OverlayKind::TaskCreatePrompt));
    }

    #[test]
    fn test_active_overlay_help_takes_priority() {
        // Help overlay has highest priority
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut app = App::new();
        app.help_visible = true;
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason: hackpi_guardrails::GuardReason {
                guard: hackpi_guardrails::GuardType::CommandGate,
                tool: "bash".into(),
                details: "test".into(),
            },
            response: Some(tx),
        });
        assert_eq!(
            active_overlay(&app),
            Some(OverlayKind::HelpOverlay),
            "help overlay should take priority over permission prompt"
        );
    }

    // ── KeyContext tests ───────────────────────────────────────────────

    #[test]
    fn test_app_key_context_permission_overlay() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason: hackpi_guardrails::GuardReason {
                guard: hackpi_guardrails::GuardType::CommandGate,
                tool: "bash".into(),
                details: "test".into(),
            },
            response: Some(tx),
        });
        assert_eq!(app_key_context(&app), KeyContext::PermissionPrompt);
    }

    #[test]
    fn test_app_key_context_task_create_overlay() {
        let mut app = App::new();
        app.creating_task = true;
        assert_eq!(app_key_context(&app), KeyContext::TaskCreatePrompt);
    }

    #[test]
    fn test_app_key_context_resting_composer() {
        let app = App::new();
        assert_eq!(app_key_context(&app), KeyContext::Composer);
    }

    #[test]
    fn test_app_key_context_generating_conversation() {
        let mut app = App::new();
        app.ui_status = UiStatus::Generating;
        assert_eq!(app_key_context(&app), KeyContext::Conversation);
    }

    #[test]
    fn test_app_key_context_task_board() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        assert_eq!(app_key_context(&app), KeyContext::TaskBoard);
    }

    #[test]
    fn test_app_key_context_task_detail() {
        let mut app = App::new();
        app.active_view = AppView::TaskDetail("TSK-001".to_string());
        assert_eq!(app_key_context(&app), KeyContext::TaskDetail);
    }

    // ── footer_bindings tests ──────────────────────────────────────────

    #[test]
    fn test_footer_bindings_composer_includes_global_and_composer() {
        let bindings = footer_bindings(KeyContext::Composer);
        let keys: Vec<&str> = bindings.iter().map(|b| b.key).collect();
        assert!(
            keys.contains(&"Ctrl+L"),
            "global binding Ctrl+L should be in composer footer: {keys:?}"
        );
        assert!(
            keys.contains(&"Enter"),
            "composer binding Enter should be in composer footer: {keys:?}"
        );
        // Ctrl+D should be there (global)
        assert!(
            keys.contains(&"Ctrl+D"),
            "global binding Ctrl+D should be in composer footer: {keys:?}"
        );
    }

    #[test]
    fn test_footer_bindings_task_board_includes_task_board_keys() {
        let bindings = footer_bindings(KeyContext::TaskBoard);
        let keys: Vec<&str> = bindings.iter().map(|b| b.key).collect();
        assert!(
            keys.contains(&"Up/Down"),
            "Up/Down should be in task board footer: {keys:?}"
        );
        assert!(
            keys.contains(&"n"),
            "'n' should be in task board footer: {keys:?}"
        );
    }

    #[test]
    fn test_footer_bindings_excludes_non_footer() {
        let bindings = footer_bindings(KeyContext::Conversation);
        let keys: Vec<&str> = bindings.iter().map(|b| b.key).collect();
        assert!(
            !keys.contains(&"PgUp/PgDn"),
            "PgUp/PgDn should not be in footer bindings: {keys:?}"
        );
        assert!(
            !keys.contains(&"?"),
            "'?' should not be in footer bindings: {keys:?}"
        );
    }

    #[test]
    fn test_footer_bindings_help_overlay_shows_global() {
        let bindings = footer_bindings(KeyContext::HelpOverlay);
        let keys: Vec<&str> = bindings.iter().map(|b| b.key).collect();
        // Global bindings (Ctrl+C, Ctrl+L, Ctrl+D) are always in scope
        assert!(
            keys.contains(&"Ctrl+D"),
            "Ctrl+D global binding should appear even during help: {keys:?}"
        );
        assert!(
            !keys.contains(&"Esc"),
            "Esc (help-specific) should not be a footer binding: {keys:?}"
        );
    }

    // ── help_bindings tests ────────────────────────────────────────────

    #[test]
    fn test_help_bindings_include_all_context_bindings() {
        let bindings = help_bindings(KeyContext::TaskBoard);
        let keys: Vec<&str> = bindings.iter().map(|b| b.key).collect();
        assert!(keys.contains(&"Up/Down"), "should include context bindings");
        assert!(keys.contains(&"Ctrl+L"), "should include global bindings");
        assert!(keys.contains(&"?"), "should include '?' binding");
    }

    #[test]
    fn test_help_bindings_footer_first() {
        let bindings = help_bindings(KeyContext::Conversation);
        // The first few should be footer bindings
        let first = bindings.first().expect("should have at least one binding");
        assert!(
            first.footer,
            "first help binding should be a footer binding, got: {} {}",
            first.key, first.action
        );
    }

    #[test]
    fn test_help_bindings_empty_context_returns_global_only() {
        // HelpOverlay context has no global footer bindings
        let bindings = help_bindings(KeyContext::HelpOverlay);
        let keys: Vec<&str> = bindings.iter().map(|b| b.key).collect();
        // Should have at least "?" (global), "Esc" (help overlay)
        assert!(
            keys.contains(&"Esc"),
            "HelpOverlay bindings should include Esc: {keys:?}"
        );
    }
}
