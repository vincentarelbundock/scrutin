//! End-to-end tests for the source-to-tests dependency map (spec §3.4).
//!
//! These tests build on-disk tempdir fixtures and drive the real
//! `build_import_map`, `build_unified_dep_map`, and `resolve_tests`
//! entry points. They do not spawn Rscript or pytest: the Python
//! dep map is pure static analysis and the R dep map's static entry
//! point just loads the cached JSON, so both are testable without
//! any subprocess.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use scrutin_core::analysis::deps::{build_unified_dep_map, resolve_tests, TestAction};
use scrutin_core::project::package::{Package, TestSuite, WorkerHookPaths};
use scrutin_core::project::plugin::plugin_by_name;
use scrutin_core::python::imports::build_import_map;
use scrutin_core::storage::sqlite;
use tempfile::TempDir;

// ── Fixture helpers ─────────────────────────────────────────────────────────

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Build a Package with a single pytest suite rooted at `root`. The caller
/// writes the pyproject.toml, source, and test files before calling this.
/// `test_dirs` is `[root/tests]` to match scrutin's default discovery.
fn pytest_package(root: &Path) -> Package {
    let plugin = plugin_by_name("pytest").expect("pytest plugin must be registered");
    let suite = TestSuite::new(
        plugin,
        root.to_path_buf(),
        vec![
            "tests/**/test_*.py".into(),
            "tests/**/*_test.py".into(),
        ],
        // Mirror the pytest plugin default: src/, lib/, plus a flat-layout
        // fallback that catches `pkg/` at the project root.
        vec![
            "src/**/*.py".into(),
            "lib/**/*.py".into(),
            "**/*.py".into(),
        ],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    Package {
        name: "demo".into(),
        root: root.to_path_buf(),
        test_suites: vec![suite],
        pytest_extra_args: Vec::new(),
        python_interpreter: Vec::new(),
        env: BTreeMap::new(),
    }
}

/// Shape a tempdir into a minimal valid pytest project layout. All tests
/// that exercise `build_import_map` need this baseline.
fn pytest_project() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    write(&tmp.path().join("pyproject.toml"), "[project]\nname = \"demo\"\n");
    fs::create_dir_all(tmp.path().join("tests")).unwrap();
    tmp
}

// ── build_import_map: direct imports ────────────────────────────────────────

#[test]
fn import_map_src_layout_direct_hit() {
    // tests/test_math.py imports `from pkg.math import add`.
    // Editing src/pkg/math.py must invalidate test_math.py.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/math.py"), "def add(a, b): return a + b\n");
    write(
        &root.join("tests/test_math.py"),
        "from pkg.math import add\n\ndef test_add(): assert add(1, 2) == 3\n",
    );

    let pkg = pytest_package(root);
    let map = build_import_map(&pkg);

    assert_eq!(
        map.get("src/pkg/math.py").map(|v| v.as_slice()),
        Some(&["test_math.py".to_string()][..]),
        "editing src/pkg/math.py must trigger test_math.py; got map={map:?}"
    );
}

#[test]
fn import_map_flat_layout_direct_hit() {
    // No src/ dir: pkg sits at the project root. Same contract must hold.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("pkg/__init__.py"), "");
    write(&root.join("pkg/math.py"), "def add(a, b): return a + b\n");
    write(
        &root.join("tests/test_math.py"),
        "from pkg.math import add\n",
    );

    let map = build_import_map(&pytest_package(root));
    assert_eq!(
        map.get("pkg/math.py").map(|v| v.as_slice()),
        Some(&["test_math.py".to_string()][..]),
    );
}

#[test]
fn import_map_package_init_is_a_valid_dep_target() {
    // `import pkg` maps to pkg/__init__.py when that file exists.
    // This locks the "package-level import" behavior: editing __init__.py
    // invalidates any test that imports the package itself.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "from .math import add\n");
    write(&root.join("src/pkg/math.py"), "def add(a, b): return a + b\n");
    write(&root.join("tests/test_pkg.py"), "import pkg\n");

    let map = build_import_map(&pytest_package(root));
    let init_tests = map.get("src/pkg/__init__.py");
    assert!(
        init_tests.is_some_and(|v| v.contains(&"test_pkg.py".to_string())),
        "editing src/pkg/__init__.py must trigger test_pkg.py; got map={map:?}"
    );
}

// ── Transitive resolution (documented in the module docstring) ──────────────

#[test]
fn import_map_is_transitive() {
    // test_x.py imports helpers.py which imports core.py.
    // Editing core.py must invalidate test_x.py.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/core.py"), "VALUE = 1\n");
    write(
        &root.join("src/pkg/helpers.py"),
        "from pkg.core import VALUE\n\ndef h(): return VALUE\n",
    );
    write(
        &root.join("tests/test_x.py"),
        "from pkg.helpers import h\n\ndef test_h(): assert h() == 1\n",
    );

    let map = build_import_map(&pytest_package(root));

    assert!(
        map.get("src/pkg/helpers.py")
            .is_some_and(|v| v.contains(&"test_x.py".to_string())),
        "direct dep src/pkg/helpers.py must map to test_x.py; got map={map:?}"
    );
    assert!(
        map.get("src/pkg/core.py")
            .is_some_and(|v| v.contains(&"test_x.py".to_string())),
        "transitive dep src/pkg/core.py must map to test_x.py; got map={map:?}"
    );
}

#[test]
fn import_map_is_cycle_safe() {
    // a.py imports from b, b.py imports from a. The BFS in build_import_map
    // must not hang or recurse forever. Test times out if it does.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/a.py"), "from pkg.b import y\nx = 1\n");
    write(&root.join("src/pkg/b.py"), "from pkg.a import x\ny = 2\n");
    write(&root.join("tests/test_cycle.py"), "from pkg.a import x\n");

    let map = build_import_map(&pytest_package(root));
    // Both src files are (transitively) reachable from test_cycle.py.
    assert!(
        map.get("src/pkg/a.py")
            .is_some_and(|v| v.contains(&"test_cycle.py".to_string())),
        "a.py must map to test_cycle.py even though a imports b imports a; got map={map:?}"
    );
    assert!(
        map.get("src/pkg/b.py")
            .is_some_and(|v| v.contains(&"test_cycle.py".to_string())),
    );
}

// ── Edge cases: test file handling and unreferenced files ───────────────────

#[test]
fn import_map_does_not_key_on_test_files() {
    // Editing a test file must not appear as a dep-map key pointing to
    // other test files: test files aren't source, they're the leaves.
    // The watcher handles "I edited a test, rerun it" via a separate path.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/math.py"), "def add(a, b): return a + b\n");
    write(
        &root.join("tests/conftest.py"),
        "import pytest\n",
    );
    write(
        &root.join("tests/test_math.py"),
        "from pkg.math import add\n",
    );

    let map = build_import_map(&pytest_package(root));
    for key in map.keys() {
        assert!(
            !key.starts_with("tests/test_") && !key.ends_with("/conftest.py"),
            "dep-map key {key:?} looks like a test file; map={map:?}"
        );
    }
}

#[test]
fn import_map_omits_unreferenced_source_files() {
    // src/pkg/unused.py isn't imported by anything. It should not appear
    // as a map key (or if it does, with an empty value) so a watch-mode
    // edit to it falls through to the filename heuristic or FullSuite,
    // not a silent no-op that runs zero tests.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/math.py"), "def add(a, b): return a + b\n");
    write(&root.join("src/pkg/unused.py"), "def never_called(): pass\n");
    write(
        &root.join("tests/test_math.py"),
        "from pkg.math import add\n",
    );

    let map = build_import_map(&pytest_package(root));
    assert!(
        !map.contains_key("src/pkg/unused.py") || map["src/pkg/unused.py"].is_empty(),
        "unreferenced source must not claim any tests; got map={map:?}"
    );
}

// ── Relative imports ───────────────────────────────────────────────────────

#[test]
fn import_map_relative_import_within_package() {
    // A test file inside a package using `from .sibling import x`. The
    // relative resolver must pop to the package and land on the sibling.
    //
    // Note: the "test file" predicate is pattern-based (`test_*.py`,
    // `*_test.py`), so a non-conforming file inside `tests/` (like
    // `helpers.py` or `conftest.py`) is treated as a *source* file by
    // the dep map. This is intentional: editing a test helper should
    // invalidate the tests that import it. This test locks both the
    // relative-resolution behavior AND the "tests/helpers.py is
    // source-ish" semantics.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("tests/__init__.py"), "");
    write(&root.join("tests/helpers.py"), "def mk(): return 1\n");
    write(
        &root.join("tests/test_rel.py"),
        "from .helpers import mk\n\ndef test_mk(): assert mk() == 1\n",
    );

    let map = build_import_map(&pytest_package(root));
    assert!(
        map.get("tests/helpers.py")
            .is_some_and(|v| v.contains(&"test_rel.py".to_string())),
        "relative import must resolve: editing tests/helpers.py should invalidate test_rel.py; \
         got map={map:?}"
    );
    // Conversely, files whose basenames *do* match the test pattern must
    // never be keys: that's the leaves-are-not-sources invariant from
    // `import_map_does_not_key_on_test_files`.
    for key in map.keys() {
        assert!(
            !key.starts_with("tests/test_"),
            "test_*.py files must never be dep-map keys; got {key:?} in {map:?}"
        );
    }
}

// ── Known limitation: attribute-access imports ──────────────────────────────

#[test]
fn import_map_documents_attribute_access_limitation() {
    // `import pkg` followed by `pkg.math.add(...)` is common Python.
    // The current static scanner only records the top-level import
    // (`pkg`), not the attribute chain. So editing src/pkg/math.py in
    // isolation does NOT invalidate test_attr.py.
    //
    // This is a deliberate limitation (short of running Python); the
    // filename heuristic + `pkg/__init__.py` re-export fallback cover
    // most real cases. Lock the behavior so any future change (e.g.
    // switching to a real AST parser) is an intentional choice, not an
    // accidental one.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/math.py"), "def add(a, b): return a + b\n");
    write(
        &root.join("tests/test_attr.py"),
        "import pkg\n\ndef test_add(): assert pkg.math.add(1, 2) == 3\n",
    );

    let map = build_import_map(&pytest_package(root));
    let math_tests = map.get("src/pkg/math.py").cloned().unwrap_or_default();
    assert!(
        !math_tests.contains(&"test_attr.py".to_string()),
        "documented limitation: bare `import pkg` does not invalidate pkg/math.py; \
         if this test fails, the scanner was upgraded and the spec should be updated. \
         map={map:?}"
    );
    // `src/pkg/__init__.py` SHOULD map to test_attr.py (the top-level
    // import was observed). Editing the package init triggers rerun;
    // editing individual submodules does not.
    let init_tests = map.get("src/pkg/__init__.py").cloned().unwrap_or_default();
    assert!(
        init_tests.contains(&"test_attr.py".to_string()),
        "the __init__.py fallback is what saves us here; got map={map:?}"
    );
}

// ── analysis::deps::resolve_tests dispatch ──────────────────────────────────

#[test]
fn resolve_tests_prefers_dep_map_over_heuristic() {
    // Dep map says math.py -> [test_math.py]. resolve_tests must use
    // that directly rather than falling through to the filename heuristic.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/math.py"), "def add(a,b): return a+b\n");
    write(&root.join("tests/test_math.py"), "from pkg.math import add\n");

    let pkg = pytest_package(root);
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    map.insert("src/pkg/math.py".into(), vec!["test_math.py".into()]);

    let action = resolve_tests(&root.join("src/pkg/math.py"), &pkg, Some(&map));
    match action {
        TestAction::Run(files) => {
            assert_eq!(files.len(), 1);
            assert!(
                files[0].ends_with("tests/test_math.py"),
                "expected tests/test_math.py, got {:?}",
                files[0]
            );
        }
        TestAction::FullSuite => panic!("expected Run, got FullSuite"),
    }
}

#[test]
fn resolve_tests_falls_back_to_heuristic_when_dep_map_misses() {
    // No dep map entry, but the filename heuristic matches
    // (`math.py` -> `test_math.py`). resolve_tests must pick it up.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/math.py"), "def add(a,b): return a+b\n");
    write(&root.join("tests/test_math.py"), "from pkg.math import add\n");

    let pkg = pytest_package(root);
    let empty: HashMap<String, Vec<String>> = HashMap::new();

    let action = resolve_tests(&root.join("src/pkg/math.py"), &pkg, Some(&empty));
    match action {
        TestAction::Run(files) => {
            assert_eq!(files.len(), 1);
            assert!(files[0].ends_with("tests/test_math.py"));
        }
        TestAction::FullSuite => panic!("expected Run via heuristic, got FullSuite"),
    }
}

#[test]
fn resolve_tests_full_suite_when_nothing_matches() {
    // A random file that isn't a test, isn't in the dep map, and doesn't
    // match the filename heuristic should yield FullSuite, not an empty Run.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("tests/test_math.py"), "def test_noop(): pass\n");

    let pkg = pytest_package(root);
    let empty: HashMap<String, Vec<String>> = HashMap::new();

    let action = resolve_tests(
        &root.join("src/pkg/no_matching_test_name.py"),
        &pkg,
        Some(&empty),
    );
    assert!(
        matches!(action, TestAction::FullSuite),
        "unmatched file should trigger FullSuite"
    );
}

#[test]
fn resolve_tests_changed_test_file_runs_itself() {
    // Editing a test file directly must invalidate that file itself,
    // independent of any dep map entry.
    let tmp = pytest_project();
    let root = tmp.path();
    write(&root.join("tests/test_alpha.py"), "def test_a(): pass\n");
    write(&root.join("tests/test_beta.py"), "def test_b(): pass\n");

    let pkg = pytest_package(root);
    let empty: HashMap<String, Vec<String>> = HashMap::new();

    let action = resolve_tests(&root.join("tests/test_alpha.py"), &pkg, Some(&empty));
    match action {
        TestAction::Run(files) => {
            assert_eq!(files.len(), 1);
            assert!(files[0].ends_with("test_alpha.py"));
            assert!(
                !files.iter().any(|p| p.ends_with("test_beta.py")),
                "sibling tests must not be triggered"
            );
        }
        TestAction::FullSuite => panic!("expected Run containing self"),
    }
}

// ── R dep map load contract ─────────────────────────────────────────────────

#[test]
fn r_dep_map_returns_empty_when_no_cache() {
    // No .scrutin/depmap.json on disk. build_dep_map must return an empty
    // map, not an error. This is the stable contract: runtime
    // instrumentation populates the cache incrementally, so the first
    // run always starts from empty.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Construct a Package shaped like an R project. The R dep map
    // loader doesn't actually inspect the suites, only pkg.root, so we
    // can reuse the pytest package here.
    let pkg = pytest_package(root);

    let map = scrutin_core::r::depmap::build_dep_map(&pkg).expect("build_dep_map");
    assert!(
        map.is_empty(),
        "fresh project must yield empty R dep map; got {map:?}"
    );
}

#[test]
fn r_dep_map_loads_cached_file() {
    // A pre-existing .scrutin/depmap.json must round-trip through
    // build_dep_map. This is how watch-mode cold starts get their
    // dep graph on a second invocation.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let mut cached: HashMap<String, Vec<String>> = HashMap::new();
    cached.insert("R/math.R".into(), vec!["test-math.R".into()]);
    cached.insert(
        "R/strings.R".into(),
        vec!["test-strings.R".into(), "test-fuzz.R".into()],
    );
    sqlite::with_open(root, |c| sqlite::replace_dep_map(c, &cached)).expect("store");

    let pkg = pytest_package(root);
    let loaded = scrutin_core::r::depmap::build_dep_map(&pkg).expect("load");
    // Normalize ordering: load_dep_map sorts values internally.
    let mut norm = cached.clone();
    for v in norm.values_mut() {
        v.sort();
        v.dedup();
    }
    assert_eq!(loaded, norm);
}

// ── build_unified_dep_map: multi-language fan-in ────────────────────────────

#[test]
fn unified_dep_map_merges_r_cache_and_python_imports() {
    // A project with both an R suite (cached dep map) and a pytest suite
    // (live static scan). The unified map must contain keys from both,
    // without either language's keys clobbering the other. R keys end in
    // .R/.r; Python keys end in .py, so collisions are structurally
    // impossible, but the merge logic still has to run both paths.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // R side: populate the cache directly.
    let mut r_cached: HashMap<String, Vec<String>> = HashMap::new();
    r_cached.insert("R/math.R".into(), vec!["test-math.R".into()]);
    sqlite::with_open(root, |c| sqlite::replace_dep_map(c, &r_cached)).unwrap();

    // Python side: write a real project layout.
    write(&root.join("pyproject.toml"), "[project]\nname = \"demo\"\n");
    write(&root.join("src/pkg/__init__.py"), "");
    write(&root.join("src/pkg/math.py"), "def add(a,b): return a+b\n");
    fs::create_dir_all(root.join("tests")).unwrap();
    write(
        &root.join("tests/test_math.py"),
        "from pkg.math import add\n",
    );

    // Construct a Package that holds BOTH an R suite (minimal: just tells
    // the unified builder to load the cache) and a pytest suite.
    let r_plugin = plugin_by_name("testthat").expect("testthat plugin must be registered");
    let py_plugin = plugin_by_name("pytest").expect("pytest plugin must be registered");
    let r_suite = TestSuite::new(
        r_plugin,
        root.to_path_buf(),
        vec!["tests/testthat/**/test-*.R".into()],
        vec!["R/**/*.R".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let py_suite = TestSuite::new(
        py_plugin,
        root.to_path_buf(),
        vec![
            "tests/**/test_*.py".into(),
            "tests/**/*_test.py".into(),
        ],
        vec!["src/**/*.py".into(), "lib/**/*.py".into()],
        WorkerHookPaths::default(),
        None,
    )
    .expect("compile globs");
    let pkg = Package {
        name: "demo".into(),
        root: root.to_path_buf(),
        test_suites: vec![r_suite, py_suite],
        pytest_extra_args: Vec::new(),
        python_interpreter: Vec::new(),
        env: BTreeMap::new(),
    };

    let unified = build_unified_dep_map(&pkg);
    assert_eq!(
        unified.get("R/math.R").map(|v| v.as_slice()),
        Some(&["test-math.R".to_string()][..]),
        "R cache entries must survive merge; got {unified:?}"
    );
    assert_eq!(
        unified.get("src/pkg/math.py").map(|v| v.as_slice()),
        Some(&["test_math.py".to_string()][..]),
        "Python scanner entries must survive merge; got {unified:?}"
    );
}

