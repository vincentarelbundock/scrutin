//! `scrutin-core`: language-agnostic test runner engine.
//!
//! This crate is the public boundary that all scrutin frontends
//! (`scrutin-bin`, `scrutin-tui`, future `scrutin-web`) depend on. It owns
//! project discovery, the run engine, the dep-map analyzer, NDJSON
//! protocol, JUnit/DB persistence, and the embedded R/pytest companion
//! scripts. See `CLAUDE.md` at the workspace root for the architecture
//! overview and the per-crate split rationale.

pub mod agent;
pub mod analysis;
pub mod engine;
pub mod filter;
pub mod git;
pub mod keymap;
pub mod logbuf;
pub mod noticebuf;
pub mod metadata;
pub mod preflight;
pub mod project;
pub mod prose;
pub mod python;
pub mod r;
pub mod report;
pub mod storage;
