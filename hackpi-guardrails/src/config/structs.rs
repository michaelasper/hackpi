/// Extra metadata parsed from the `command_gate` config block.
pub struct CommandGateExtras {
    /// When true, inject allow rules for `git` and `gh` so they bypass
    /// the built-in VCS deny patterns.
    pub allow_git_in_bash: bool,
}
