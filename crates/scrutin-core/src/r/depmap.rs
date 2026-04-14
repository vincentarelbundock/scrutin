//! R source->tests dependency map (Tier 2).
//!
//! The dep map is built incrementally from runtime instrumentation: each test
//! run emits a `deps` message listing source files whose functions were called
//! via `trace()`. The engine merges these observations into the cached map in
//! the `dependencies` table of `.scrutin/state.db`.
//!
//! This module's `build_dep_map` entry point simply loads the cached map. It
//! is called by `analysis::deps::build_unified_dep_map` so that the unified
//! map can combine R instrumentation data with Python static import scanning.

use std::collections::HashMap;

use anyhow::Result;

use crate::project::package::Package;
use crate::storage::sqlite;

/// Load the cached R dep map. Returns empty if no cached map exists.
///
/// With instrumentation, the map is populated incrementally by the engine
/// as tests run, rather than computed from static analysis.
pub fn build_dep_map(pkg: &Package) -> Result<HashMap<String, Vec<String>>> {
    match sqlite::with_open(&pkg.root, |c| Ok(sqlite::load_dep_map(c))) {
        Ok(m) => Ok(m),
        Err(_) => Ok(HashMap::new()),
    }
}
