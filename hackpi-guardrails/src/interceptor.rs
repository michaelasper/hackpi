/// Append a permission string to a JSON config file's permissions array.
///
/// Reads the file at `file_path` (or uses a default structure if the file
/// doesn't exist), navigates to `permissions.{target_array}`, checks for
/// duplicates (case-insensitive), appends the `permission_string` if not
/// already present, and writes the file back with pretty formatting.
///
/// # Errors
///
/// Returns `Err` if the file exists but cannot be read or parsed, or if
/// the file cannot be written.
pub fn append_to_permissions(
    file_path: &std::path::Path,
    permission_string: &str,
    target_array: &str,
) -> Result<(), String> {
    // Default structure for new file
    let default_value = serde_json::json!({
        "permissions": {
            "allow": [],
            "deny": []
        }
    });

    // Read existing or use default
    let mut config: serde_json::Value = if file_path.exists() {
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| format!("Failed to read {}: {e}", file_path.display()))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {e}", file_path.display()))?
    } else {
        default_value.clone()
    };

    // Ensure permissions struct exists
    if !config.is_object() {
        config = default_value;
    }
    if !config
        .get("permissions")
        .and_then(|v| v.is_object().then_some(true))
        .unwrap_or(false)
    {
        config["permissions"] = serde_json::json!({"allow": [], "deny": []});
    }

    // Get target array
    let arr = config["permissions"][target_array]
        .as_array_mut()
        .ok_or_else(|| format!("permissions.{target_array} is not an array"))?;

    // Check for duplicate (case-insensitive for permission strings)
    if arr.iter().any(|v| {
        v.as_str()
            .map(|s| s.eq_ignore_ascii_case(permission_string))
            .unwrap_or(false)
    }) {
        return Ok(()); // Already exists, not an error
    }

    // Append
    arr.push(serde_json::Value::String(permission_string.to_string()));

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    }

    // Write with pretty formatting — use temp file + atomic rename for crash safety
    let content =
        serde_json::to_string_pretty(&config).map_err(|e| format!("Failed to serialize: {e}"))?;

    // Write to a temporary file in the same directory, then atomically rename.
    // On POSIX, rename() is atomic when source and destination are on the same
    // filesystem, so concurrent readers always see a consistent file.
    let tmp_path = file_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write temporary file {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, file_path).map_err(|e| {
        format!(
            "Failed to rename {} to {}: {e}",
            tmp_path.display(),
            file_path.display()
        )
    })?;

    Ok(())
}
