use super::Scenario;
use std::path::Path;

/// Load a scenario from a JSON file.
///
/// Reads the file and deserializes it into a [`Scenario`].
///
/// # Errors
///
/// Returns an error if the file cannot be read or the JSON is malformed.
pub(super) async fn load_scenario(path: &Path) -> anyhow::Result<Scenario> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read scenario file '{}': {e}", path.display()))?;

    let scenario: Scenario = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse scenario '{}': {e}", path.display()))?;

    Ok(scenario)
}

/// Parse command-line arguments for `--script`.
///
/// Returns `Some(path)` if `--script <path>` is found, removing those
/// arguments from the list. Returns `None` if `--script` is not present.
pub fn parse_script_args(args: &mut Vec<String>) -> Option<String> {
    let script_flag_pos = args.iter().position(|a| a == "--script")?;
    // Remove the flag first.
    args.remove(script_flag_pos);
    // The value that was after the flag is now at `script_flag_pos`.
    let script_path = args.get(script_flag_pos)?;
    let path = script_path.clone();
    args.remove(script_flag_pos);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_script_args tests ────────────────────────────────────────

    #[test]
    fn test_parse_script_args_found() {
        let mut args: Vec<String> = vec![
            "hackpi".to_string(),
            "--script".to_string(),
            "scenario.json".to_string(),
            "--god".to_string(),
        ];
        let path = parse_script_args(&mut args);
        assert_eq!(path, Some("scenario.json".to_string()));
        assert_eq!(args, vec!["hackpi".to_string(), "--god".to_string()]);
    }

    #[test]
    fn test_parse_script_args_not_found() {
        let mut args: Vec<String> = vec!["hackpi".to_string(), "--god".to_string()];
        let path = parse_script_args(&mut args);
        assert!(path.is_none());
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn test_parse_script_args_only_flag_no_value() {
        // If --script is last arg, there's no value following.
        let mut args: Vec<String> = vec!["hackpi".to_string(), "--script".to_string()];
        let path = parse_script_args(&mut args);
        assert!(path.is_none());
        // The flag should still be removed since we found it
        assert_eq!(args, vec!["hackpi".to_string()]);
    }
}
