/// Check whether a command string matches a pattern (case-insensitive substring match).
pub fn command_matches_pattern(command: &str, pattern: &str) -> bool {
    let command_lower = command.to_lowercase();
    let pattern_lower = pattern.to_lowercase();
    command_lower.contains(&pattern_lower)
}

/// Check whether a command matches a pattern that may contain `*` wildcards.
///
/// `*` matches any sequence of characters (including an empty sequence).
/// Matching is case-insensitive.
///
/// When the pattern contains no `*`, falls back to substring matching
/// (same as [`command_matches_pattern`]).
///
/// # Examples
///
/// - `curl *` matches `curl https://example.com`
/// - `cargo *` matches `cargo build`
/// - `*` matches any command
/// - `curl * | sh` matches `curl https://example.com | sh`
///
/// A leading non-empty segment must match the start of the command.
/// A trailing non-empty segment must match the end of the command.
/// Middle segments are found anywhere (in order).
pub fn command_matches_wildcard(command: &str, pattern: &str) -> bool {
    let command_lower = command.to_lowercase();
    let pattern_lower = pattern.to_lowercase();

    // No wildcard — fall back to substring match
    if !pattern_lower.contains('*') {
        return command_lower.contains(&pattern_lower);
    }

    let segments: Vec<&str> = pattern_lower.split('*').collect();

    // Bare `*` matches everything
    if segments.iter().all(|s| s.is_empty()) {
        return true;
    }

    // Check leading: first non-empty segment must match at start
    if let Some(first) = segments.first() {
        if !first.is_empty() && !command_lower.starts_with(first) {
            return false;
        }
    }

    // Check trailing: last non-empty segment must match at end
    if let Some(last) = segments.last() {
        if !last.is_empty() && !command_lower.ends_with(last) {
            return false;
        }
    }

    // Verify each non-empty segment appears in order
    let mut pos = 0;
    for segment in &segments {
        if segment.is_empty() {
            continue;
        }
        match command_lower[pos..].find(segment) {
            Some(found) => pos += found + segment.len(),
            None => return false,
        }
    }

    true
}

/// Check whether a command string matches a pattern at word boundaries.
///
/// The pattern must be surrounded by word boundaries (start-of-string,
/// end-of-string, or non-alphanumeric characters excluding `_`). When
/// `case_sensitive` is true, the match is case-sensitive; otherwise it's
/// case-insensitive.
///
/// This prevents false positives like `dd` matching inside `git add .`
/// (where "dd" appears within "add") or `su` matching inside `source` or
/// `issue`.
pub fn command_matches_at_word_boundary(
    command: &str,
    pattern: &str,
    case_sensitive: bool,
) -> bool {
    let (cmd, pat) = if case_sensitive {
        (command.as_bytes().to_vec(), pattern.as_bytes().to_vec())
    } else {
        (
            command.to_lowercase().into_bytes(),
            pattern.to_lowercase().into_bytes(),
        )
    };

    if pat.is_empty() {
        return true;
    }

    let mut i = 0;
    while i + pat.len() <= cmd.len() {
        if cmd[i..i + pat.len()] == pat[..] {
            // Check left word boundary
            let left_ok = i == 0 || !is_word_char(cmd[i - 1]);
            // Check right word boundary
            let right_ok = i + pat.len() == cmd.len() || !is_word_char(cmd[i + pat.len()]);

            if left_ok && right_ok {
                return true;
            }
        }
        i += 1;
    }

    false
}

/// Returns `true` if `b` is an ASCII word character (alphanumeric or underscore).
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
