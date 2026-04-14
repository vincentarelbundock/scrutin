//! Cross-language analysis utilities.
//!
//! Anything language-specific lives under `crate::r::*` or `crate::python::*`.
//! This module is the home for code that operates over *every* active suite
//! at once, regardless of language:
//!
//! - `walk` — shared filesystem walker with a centralized noise-dir ignore
//!   list. Used by every analyzer.
//! - `deps` — `resolve_tests`, the cross-language test resolver the watcher
//!   and TUI consume.
//! - `hashing` — content fingerprints used for dep-map staleness detection.
//!   Multi-suite-aware: hashes every active suite's source + test dirs.

pub mod deps;
pub mod hashing;
pub mod walk;
