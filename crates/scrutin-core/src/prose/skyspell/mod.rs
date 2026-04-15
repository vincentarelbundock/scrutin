//! skyspell spell-check plugin (command mode).
//!
//! skyspell (https://github.com/your-tools/skyspell) is a source-aware CLI
//! spell checker: it tokenizes code + prose, skips CamelCase / identifiers
//! by default, and ships a personal + per-project ignore list. It prints
//! machine-readable JSON via `--output-format json`, which we parse in Rust
//! exactly like the jarl and ruff plugins.
//!
//! Opt-in: detection only fires when `skyspell.toml` exists at the project
//! root. The marker is purely an on/off flag; per-suite configuration lives
//! in `.scrutin/config.toml [skyspell]`, mirroring the `[pytest]` pattern:
//!
//! ```toml
//! [skyspell]
//! extra_args = ["--lang", "en_US"]   # args before the subcommand
//! add_args = ["--project"]            # args after `add`, before <WORD>
//! ```

pub mod plugin;

/// Marker file that opts a project into the skyspell suite. Mirrors the
/// `jarl.toml` convention.
pub const MARKER: &str = "skyspell.toml";
