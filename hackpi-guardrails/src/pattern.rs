use crate::{RuleAction, ToolPattern};

/// Parse a permission string in the format `ToolName(pattern)`.
///
/// If no tool prefix is present, returns a pattern that applies to all tools.
pub fn parse_permission(_s: &str) -> Result<(Option<ToolPattern>, RuleAction), String> {
    // TODO: Implement pattern parsing (Phase 2)
    Err("not yet implemented".into())
}

#[cfg(test)]
mod tests {
    // TODO: Pattern parsing tests (Phase 2)
}
