//! Content fingerprints for dep-map staleness detection.
//!
//! `is_dep_map_stale` snapshots a hash for every source/test file across
//! every active suite, then compares against the previous snapshot stored
//! in the `hashes` table of `.scrutin/state.db`. Any add, delete, or
//! content change marks the dep map stale so the next run rebuilds it
//! instead of trusting the cached edges.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use xxhash_rust::xxh64;

use crate::analysis::walk;
use crate::project::package::Package;
use crate::storage::sqlite;

/// Hash a single file's contents with xxhash64.
pub fn hash_file(path: &Path) -> Result<u64> {
    let contents = std::fs::read(path)?;
    Ok(xxh64::xxh64(&contents, 0))
}

/// Hash every source and test file across every active suite in `pkg`.
pub fn hash_package_files(pkg: &Package) -> Result<HashMap<PathBuf, u64>> {
    let mut hashes: HashMap<PathBuf, u64> = HashMap::new();

    for src_dir in pkg.resolved_source_dirs() {
        for path in walk::collect_files(&src_dir, |p| pkg.is_any_source_file(p)) {
            if let Ok(h) = hash_file(&path) {
                hashes.insert(path, h);
            }
        }
    }

    for test_dir in pkg.resolved_test_dirs() {
        for path in walk::collect_files(&test_dir, |p| pkg.is_any_test_file(p)) {
            if let Ok(h) = hash_file(&path) {
                hashes.insert(path, h);
            }
        }
    }

    Ok(hashes)
}

/// Check if the dep map is stale by comparing current file hashes against
/// stored ones. Returns true on any add, delete, or content change.
pub fn is_dep_map_stale(pkg: &Package) -> Result<bool> {
    let stored = match sqlite::with_open(&pkg.root, |c| Ok(sqlite::load_hashes(c))) {
        Ok(h) => h,
        Err(_) => return Ok(true),
    };
    if stored.is_empty() {
        return Ok(true);
    }

    let current = hash_package_files(pkg)?;

    if current.len() != stored.len() {
        return Ok(true);
    }

    for (path, hash) in &current {
        match stored.get(path) {
            Some(stored_hash) if stored_hash == hash => continue,
            _ => return Ok(true),
        }
    }

    Ok(false)
}

/// Snapshot current file hashes and persist.
pub fn snapshot_hashes(pkg: &Package) -> Result<()> {
    let hashes = hash_package_files(pkg)?;
    sqlite::with_open(&pkg.root, |c| sqlite::store_hashes(c, &hashes))?;
    Ok(())
}
