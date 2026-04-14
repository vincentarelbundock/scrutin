//! ruff tool support: plugin impl + tool constants.
//!
//! ruff (<https://docs.astral.sh/ruff/>) is a fast Python linter/formatter
//! written in Rust. Like the jarl (R linter) plugin, lint diagnostics map
//! to `warn` events, clean files produce a synthetic `pass`, and fix
//! actions are exposed as keyboard shortcuts.
//!
//! Opt-in: the plugin's `detect()` only fires when a ruff config marker
//! exists (`ruff.toml`, `.ruff.toml`, or `[tool.ruff]` in
//! `pyproject.toml`), so adding ruff to the plugin registry does not
//! silently enable it for every Python project.

pub mod plugin;

/// Check whether the project root contains a ruff configuration marker.
pub(crate) fn has_ruff_config(root: &std::path::Path) -> bool {
    if root.join("ruff.toml").is_file() || root.join(".ruff.toml").is_file() {
        return true;
    }
    // Check for [tool.ruff] in pyproject.toml.
    let Ok(contents) = std::fs::read_to_string(root.join("pyproject.toml")) else {
        return false;
    };
    let Ok(value) = contents.parse::<toml::Value>() else {
        return false;
    };
    value
        .get("tool")
        .and_then(|t| t.get("ruff"))
        .is_some()
}
