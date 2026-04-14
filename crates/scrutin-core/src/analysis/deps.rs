use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::project::package::Package;
use crate::python::imports as pyimports;
use crate::r::depmap;

/// Build a unified source->tests dependency map covering every active suite
/// in `pkg`. R suites contribute `R/foo.R` keys (loaded from the cached map
/// populated by runtime instrumentation); pytest suites contribute
/// `pkg/foo.py` keys (via line-based import scanning). The two key spaces
/// never collide because R sources end in `.R` / `.r` and Python sources
/// in `.py`.
///
/// This is the canonical "build me the dep map for this project" function.
/// Both the binary's plain mode and the TUI's post-run rebuild path call it
/// so they cannot drift on which languages contribute.
pub fn build_unified_dep_map(pkg: &Package) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    if pkg.test_suites.iter().any(|s| s.plugin.language() == "r")
        && let Ok(r_map) = depmap::build_dep_map(pkg) {
            map.extend(r_map);
        }
    if pkg.test_suites.iter().any(|s| s.plugin.name() == "pytest") {
        let py_map = pyimports::build_import_map(pkg);
        map.extend(py_map);
    }
    map
}

/// Filename heuristic (Tier 1): given a changed source file, return matching test files.
///
/// Walks every active suite — a `.R` source change consults the R suites'
/// `test_file_candidates`, a `.py` change consults the pytest suite, and so
/// on. Files are matched by lowercased basename across the union of test
/// files in `pkg`.
pub fn heuristic_test_files(changed: &Path, pkg: &Package) -> Vec<PathBuf> {
    let stem = match changed.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s.to_lowercase(),
        None => return Vec::new(),
    };

    let mut candidates: Vec<String> = Vec::new();
    for suite in &pkg.test_suites {
        if !suite.plugin.is_source_file(changed) {
            continue;
        }
        for c in suite.plugin.test_file_candidates(&stem) {
            candidates.push(c.to_lowercase());
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }

    let test_files = pkg.test_files().unwrap_or_default();
    let mut matches = Vec::new();
    for path in test_files {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if candidates.contains(&name.to_lowercase()) {
            matches.push(path);
        }
    }
    matches
}

/// If the changed file is itself a test file (according to any active
/// suite), return true.
pub fn is_test_file(changed: &Path, pkg: &Package) -> bool {
    pkg.is_any_test_file(changed)
}

/// Resolve which test files to run given a changed file.
/// Uses Tier 2 (dep map) first, falls back to Tier 1 (filename heuristic).
pub fn resolve_tests(
    changed: &Path,
    pkg: &Package,
    dep_map: Option<&HashMap<String, Vec<String>>>,
) -> TestAction {
    let mut out: Vec<PathBuf> = Vec::new();

    if is_test_file(changed, pkg) {
        out.push(changed.to_path_buf());
    }

    // Dep-map lookup (Tier 2). Runs even when the changed file is already
    // claimed as a test file — a file under R/ can be both a jarl lint
    // target and a testthat source dependency, so we need the union.
    if let Some(map) = dep_map {
        let relative = changed
            .strip_prefix(&pkg.root)
            .unwrap_or(changed)
            .to_string_lossy()
            .to_string();

        if let Some(test_names) = map.get(&relative) {
            for name in test_names {
                'suite: for suite in &pkg.test_suites {
                    for td in &suite.test_dirs {
                        let p = td.join(name);
                        if p.exists() && !out.contains(&p) {
                            out.push(p);
                            break 'suite;
                        }
                    }
                }
            }
        }
    }

    if !out.is_empty() {
        return TestAction::Run(out);
    }

    let matches = heuristic_test_files(changed, pkg);
    if !matches.is_empty() {
        return TestAction::Run(matches);
    }

    TestAction::FullSuite
}

pub enum TestAction {
    Run(Vec<PathBuf>),
    FullSuite,
}
