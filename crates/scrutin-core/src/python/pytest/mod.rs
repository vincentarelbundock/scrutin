//! pytest tool support: plugin impl + tool constants.
//!
//! The plugin lives in `plugin.rs`. The runner companion (`runner.py`) is
//! embedded into the binary via `include_str!` in `plugin.rs` and lives
//! next to the plugin file rather than under a separate `runners/` tree.

pub mod plugin;

/// Conventional path (relative to the project root) where pytest tests
/// commonly live. The plugin's `test_dirs()` returns this *and* `"test"`.
pub const TEST_DIR: &str = "tests";
