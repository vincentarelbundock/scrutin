//! typos tool support: plugin impl.
//!
//! typos (<https://github.com/crate-ci/typos>) is a source-code aware
//! spell-checker written in Rust. Unlike skyspell (which flags every
//! token not in the system dictionary), typos checks against a curated
//! list of known-misspellings, so it produces essentially zero false
//! positives on identifiers and code tokens. This makes it a much
//! better fit than skyspell for scanning R / Python / Rust source.
//!
//! Opt-in only: users enable the typos suite by adding an explicit
//! `[[suite]] tool = "typos"` entry to `.scrutin/config.toml`. The
//! presence of a typos config file at the project root (`_typos.toml`,
//! `typos.toml`, or `.typos.toml`) is not sufficient, since `typos` is
//! usually wired into an editor or pre-commit hook and shouldn't
//! automatically pile onto a `scrutin` run.

pub mod plugin;
