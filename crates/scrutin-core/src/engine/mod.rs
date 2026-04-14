//! Run engine: subprocess workers, the pool that drives them, the
//! `run_events` seam consumed by the TUI/CI frontends, the NDJSON wire
//! protocol, and the filesystem watcher that triggers re-runs.
//!
//! Anything outside this module should drive runs through `run_events`,
//! not by reaching into `pool` or `runner` directly.

pub mod command_pool;
pub mod pool;
pub mod protocol;
pub mod run_events;
pub mod runner;
pub mod watcher;
