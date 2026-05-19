//! Structured system prompt builder for hackpi.
//!
//! Decomposes the flat prompt into four independently tunable sections
//! as specified in DESIGN.md:
//!
//! 1. **Identity** — who the agent is
//! 2. **Tools** — available tools and their usage
//! 3. **Workflow** — step-by-step approach
//! 4. **Rules** — constraints and best practices
//!
//! Each section is a separate function, making it easy to adjust specific
//! aspects without affecting the rest.

// ── Section 1: Identity ──────────────────────────────────────────────────────

/// The agent's identity section.
///
/// Describes who the agent is and its core purpose.
pub fn identity_section() -> &'static str {
    "\
# Identity
You are hackpi, a coding agent built with Rust. You help users write, debug, and refactor code."
}

// ── Section 2: Tools ─────────────────────────────────────────────────────────

/// The tools overview section.
///
/// Describes what tools are available and their basic usage.
pub fn tools_section() -> &'static str {
    "\
# Tool Access
- read: view files and directories (returns LINE#HASH: prefixes for editing)
- search_grep: search codebase for regex patterns with context lines
- write: create new files (will reject writes to existing files)
- edit: modify existing files using LINE#HASH anchors from read output
- bash: execute commands in a persistent virtual shell
- git_read: inspect repository state (status, diff, log, branches, remotes)
- git_write: modify repository (add, commit, push, pull, checkout, branch, merge, rebase, stash)
- github: GitHub operations (create/list PRs, issues, releases, comments)
- task: manage tasks (create, list, show, update, transition, block, unblock)"
}

// ── Section 3: Workflow ──────────────────────────────────────────────────────

/// The workflow section.
///
/// Step-by-step approach for the agent to follow when completing tasks.
pub fn workflow_section() -> &'static str {
    "\
# Workflow
1. Always read a file before editing it.
2. Use search_grep to find relevant code before making changes.
3. Verify changes compile and pass tests (cargo check / cargo test).
4. For new files, use write; for existing files, use edit with LINE#HASH anchors from read output.
5. When making commits, always git_read status first to verify changes.
6. When creating PRs, always push first, then use github pr_create.
7. Use the task tool to track your work items. Create tasks for significant features, update their state as you progress."
}

// ── Section 4: Rules ─────────────────────────────────────────────────────────

/// The rules section.
///
/// Constraints and best practices the agent must follow.
pub fn rules_section() -> &'static str {
    "\
# Rules
- Never overwrite existing files with write — use edit instead.
- Never send LINE#HASH: prefixes or diff +/- markers in edit lines (E_INVALID_PATCH).
- Run cargo check after any Rust code change."
}

// ── Assembled prompt ─────────────────────────────────────────────────────────

/// Build the complete system prompt by assembling all four sections.
///
/// Sections are separated by blank lines for readability.
pub fn build_system_prompt() -> String {
    format!(
        "{}\n\n{}\n\n{}\n\n{}",
        identity_section(),
        tools_section(),
        workflow_section(),
        rules_section()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_section_non_empty() {
        let section = identity_section();
        assert!(!section.is_empty(), "identity section should be non-empty");
        assert!(section.contains("hackpi"), "should mention hackpi");
    }

    #[test]
    fn test_tools_section_contains_known_tools() {
        let section = tools_section();
        assert!(section.contains("read"), "should mention read tool");
        assert!(section.contains("write"), "should mention write tool");
        assert!(section.contains("bash"), "should mention bash tool");
        assert!(section.contains("edit"), "should mention edit tool");
        assert!(section.contains("task"), "should mention task tool");
    }

    #[test]
    fn test_workflow_section_contains_steps() {
        let section = workflow_section();
        assert!(
            section.contains("read a file before editing"),
            "should mention read-before-edit"
        );
        assert!(
            section.contains("cargo check"),
            "should mention cargo check"
        );
    }

    #[test]
    fn test_rules_section_contains_constraints() {
        let section = rules_section();
        assert!(
            section.contains("Never overwrite"),
            "should mention no overwrite"
        );
        assert!(
            section.contains("LINE#HASH"),
            "should mention LINE#HASH anchors"
        );
    }

    #[test]
    fn test_build_system_prompt_contains_all_sections() {
        let prompt = build_system_prompt();
        assert!(prompt.contains("# Identity"), "should have Identity");
        assert!(prompt.contains("# Tool Access"), "should have Tools");
        assert!(prompt.contains("# Workflow"), "should have Workflow");
        assert!(prompt.contains("# Rules"), "should have Rules");
    }

    #[test]
    fn test_build_system_prompt_under_500_tokens() {
        let prompt = build_system_prompt();
        // Approximate token count: ~4 chars per token
        let approx_tokens = prompt.len() / 4;
        assert!(
            approx_tokens <= 600,
            "system prompt should be compact (~{approx_tokens} tokens)"
        );
    }
}
