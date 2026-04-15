//! ruff tool support: plugin impl.
//!
//! ruff (<https://docs.astral.sh/ruff/>) is a fast Python linter/formatter
//! written in Rust. Like the jarl (R linter) plugin, lint diagnostics map
//! to `warn` events, clean files produce a synthetic `pass`, and fix
//! actions are exposed as keyboard shortcuts.
//!
//! Opt-in only: users enable the ruff suite by adding an explicit
//! `[[suite]] tool = "ruff"` entry to `.scrutin/config.toml`. Presence
//! of `ruff.toml` / `.ruff.toml` / `[tool.ruff]` in `pyproject.toml` is
//! not sufficient, since ruff is usually already wired into an editor
//! or pre-commit hook and shouldn't automatically pile onto a scrutin
//! run.

pub mod plugin;
