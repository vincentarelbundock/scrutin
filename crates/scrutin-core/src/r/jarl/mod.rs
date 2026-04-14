//! jarl tool support: command-mode plugin for the jarl R linter.
//!
//! jarl (https://jarl.etiennebacher.com) is a fast R linter written in
//! Rust. It isn't a unit-test framework: a jarl "test file" is an `.R`
//! source file under `R/`, and a "test" is a lint diagnostic. Clean files
//! produce a synthetic pass event; every diagnostic becomes a `warn`.
//!
//! Unlike the worker-mode R plugins (testthat, tinytest, ...), jarl runs
//! as a command plugin: the engine spawns `jarl check --output-format json`
//! directly per file and the plugin parses the JSON in Rust. No Rscript
//! subprocess, no runner script.
//!
//! Opt-in: the plugin's `detect()` only fires when `jarl.toml` exists at
//! the project root, so adding jarl to scrutin's plugin registry does not
//! silently enable it for every R package.

pub mod plugin;

/// Marker file that opts a project into the jarl suite.
pub const MARKER: &str = "jarl.toml";

/// Source (= lint target) directory jarl scans.
pub const LINT_DIR: &str = "R";
