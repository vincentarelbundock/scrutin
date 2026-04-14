//! Local persistence via embedded SQLite (`rusqlite`, bundled).
//!
//! One file: `.scrutin/state.db`. Five tables per `SCHEMA.md`:
//! `runs`, `results`, `extras`, `dependencies`, `hashes`.
//!
//! All writes are best-effort: callers treat a failed write as a warning,
//! never as a hard error. Schema creation is idempotent (`CREATE TABLE
//! IF NOT EXISTS`); on legacy artifacts (old `depmap.json`, `hashes.json`,
//! or a non-SQLite `state.db`) [`open`] deletes them so the new DB is
//! created from scratch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::engine::protocol::{Message, Outcome};
use crate::metadata::Provenance;

// -- Tunables -----------------------------------------------------------------

const FLAKY_RECENT_RUNS: i64 = 10;
const FLAKY_MIN_RUNS: i64 = 3;
const FLAKY_RATE_LOW: f64 = 0.10;
const FLAKY_RATE_HIGH: f64 = 0.90;

const SLOW_MIN_RUNS: i64 = 3;
const SLOW_MIN_AVG_MS: f64 = 500.0;
const SLOW_LIMIT: i64 = 10;

// -- Public types -------------------------------------------------------------

pub struct FlakyTest {
    pub file: String,
    pub test: String,
    pub failures: i64,
    pub total: i64,
    pub flake_rate: f64,
}

pub struct SlowTest {
    pub file: String,
    pub test: String,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub runs: i64,
}

/// One row's worth of per-file, per-subject data for [`record_run`]. The
/// caller (reporter) supplies `tool` (the plugin name that owns the file),
/// plus optional per-tool metadata. `retries` is the count of re-executions
/// within the run; 0 means the file succeeded on its first attempt.
pub struct ResultRow {
    pub file: String,
    pub tool: String,
    pub tool_version: Option<String>,
    pub app_name: Option<String>,
    pub app_version: Option<String>,
    pub messages: Vec<Message>,
    pub retries: u32,
}

// -- Helpers ------------------------------------------------------------------

fn scrutin_dir(root: &Path) -> PathBuf {
    root.join(".scrutin")
}

fn db_path(root: &Path) -> PathBuf {
    scrutin_dir(root).join("state.db")
}

fn outcome_label(o: Outcome) -> &'static str {
    match o {
        Outcome::Pass => "pass",
        Outcome::Fail => "fail",
        Outcome::Error => "error",
        Outcome::Skip => "skip",
        Outcome::Xfail => "xfail",
        Outcome::Warn => "warn",
    }
}

/// Try to open a SQLite file and verify the magic header. Returns true if
/// the file exists and is a valid SQLite database. Used for legacy cleanup
/// so a stale DuckDB-format `state.db` gets replaced on first launch.
fn is_sqlite_file(path: &Path) -> bool {
    // SQLite files begin with the 16-byte magic string
    // "SQLite format 3\0". Any mismatch means a different format.
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return false,
    };
    bytes.len() >= 16 && &bytes[..16] == b"SQLite format 3\0"
}

fn cleanup_legacy(root: &Path) {
    let dir = scrutin_dir(root);
    // Old JSON sidecars.
    let _ = std::fs::remove_file(dir.join("depmap.json"));
    let _ = std::fs::remove_file(dir.join("hashes.json"));
    // Old DuckDB-format state.db.
    let db = dir.join("state.db");
    if db.exists() && !is_sqlite_file(&db) {
        let _ = std::fs::remove_file(&db);
    }
}

// -- Schema -------------------------------------------------------------------
//
// SQL is kept in sibling `.sql` files so it can be read, diffed, and
// round-tripped into the user-facing docs. Embedded at compile time via
// `include_str!`.

const SCHEMA_SQL: &str = include_str!("sql/schema.sql");
const INSERT_RUN_SQL: &str = include_str!("sql/insert_run.sql");
const INSERT_RESULT_SQL: &str = include_str!("sql/insert_result.sql");
const UPSERT_EXTRA_SQL: &str = include_str!("sql/upsert_extra.sql");
const SELECT_DEPENDENCIES_SQL: &str = include_str!("sql/select_dependencies.sql");
const DELETE_TEST_DEPENDENCIES_SQL: &str = include_str!("sql/delete_test_dependencies.sql");
const INSERT_DEPENDENCY_SQL: &str = include_str!("sql/insert_dependency.sql");
const DELETE_DEPENDENCIES_SQL: &str = include_str!("sql/delete_dependencies.sql");
const SELECT_HASHES_SQL: &str = include_str!("sql/select_hashes.sql");
const DELETE_HASHES_SQL: &str = include_str!("sql/delete_hashes.sql");
const UPSERT_HASH_SQL: &str = include_str!("sql/upsert_hash.sql");
const FLAKY_TESTS_SQL: &str = include_str!("sql/flaky_tests.sql");
const SLOW_TESTS_SQL: &str = include_str!("sql/slow_tests.sql");

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)
        .context("initializing SQLite schema")?;
    Ok(())
}

// -- Public API ---------------------------------------------------------------

/// Open (or create) the SQLite database at `<root>/.scrutin/state.db`.
/// Runs legacy cleanup (old JSON sidecars, old DuckDB-format `state.db`)
/// and initializes the schema. Idempotent.
pub fn open(root: &Path) -> Result<Connection> {
    let dir = scrutin_dir(root);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    cleanup_legacy(root);

    let path = db_path(root);
    let conn = Connection::open(&path)
        .with_context(|| format!("opening SQLite database at {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    conn.pragma_update(None, "foreign_keys", 1).ok();
    init_schema(&conn)?;
    Ok(conn)
}

/// Insert one `runs` row and the corresponding `results` rows in a single
/// transaction. `provenance` supplies every non-extras column of the `runs`
/// table; `timestamp` is captured by the caller so it can match any
/// `started_at` stamped elsewhere.
pub fn record_run(
    conn: &mut Connection,
    run_id: &str,
    timestamp: &str,
    provenance: &Provenance,
    results: &[ResultRow],
) -> Result<()> {
    let tx = conn.transaction().context("begin record_run tx")?;
    tx.execute(
        INSERT_RUN_SQL,
        params![
            run_id,
            timestamp,
            provenance.hostname,
            provenance.ci,
            provenance.scrutin_version,
            provenance.git_commit,
            provenance.git_branch,
            provenance.git_dirty.map(|b| b as i64),
            provenance.repo_name,
            provenance.repo_url,
            provenance.repo_root,
            provenance.build_number,
            provenance.build_id,
            provenance.build_name,
            provenance.build_url,
            provenance.os_platform,
            provenance.os_release,
            provenance.os_version,
            provenance.os_arch,
        ],
    )
    .context("insert runs row")?;

    let mut seq: i64 = 0;
    {
        let mut stmt = tx.prepare(INSERT_RESULT_SQL)?;
        for row in results {
            for msg in &row.messages {
                if let Message::Event(e) = msg {
                    seq += 1;
                    let outcome = outcome_label(e.outcome);
                    let (total, failed, fraction) = match &e.metrics {
                        Some(m) => (
                            m.total.map(|v| v as i64),
                            m.failed.map(|v| v as i64),
                            m.fraction,
                        ),
                        None => (None, None, None),
                    };
                    stmt.execute(params![
                        run_id,
                        seq,
                        row.file,
                        row.tool,
                        row.tool_version,
                        row.app_name,
                        row.app_version,
                        e.subject.kind,
                        e.subject.name,
                        e.subject.parent,
                        outcome,
                        e.duration_ms as i64,
                        row.retries as i64,
                        total,
                        failed,
                        fraction,
                    ])
                    .context("insert results row")?;
                }
            }
        }
    }
    tx.commit().context("commit record_run tx")?;
    Ok(())
}

/// Insert user-supplied `[extras]` labels for a run. Upserts on conflict.
pub fn record_extras(
    conn: &mut Connection,
    run_id: &str,
    extras: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    if extras.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(UPSERT_EXTRA_SQL)?;
        for (k, v) in extras {
            stmt.execute(params![run_id, k, v])?;
        }
    }
    tx.commit()?;
    Ok(())
}

// -- Dep map ------------------------------------------------------------------

/// Load the full dep map as `source_file -> [test_file, ...]`.
pub fn load_dep_map(conn: &Connection) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = match conn.prepare(SELECT_DEPENDENCIES_SQL) {
        Ok(s) => s,
        Err(_) => return map,
    };
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    });
    if let Ok(iter) = rows {
        for row in iter.flatten() {
            map.entry(row.0).or_default().push(row.1);
        }
    }
    for v in map.values_mut() {
        v.sort();
        v.dedup();
    }
    map
}

/// Replace one test file's edges: delete every row with `test_file = ?`,
/// then insert one row per source in `sources`. Transactional.
pub fn store_dep_map_for_test(
    conn: &mut Connection,
    test_file: &str,
    sources: &[String],
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(DELETE_TEST_DEPENDENCIES_SQL, params![test_file])?;
    {
        let mut stmt = tx.prepare(INSERT_DEPENDENCY_SQL)?;
        for src in sources {
            stmt.execute(params![src, test_file])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Full replace of the dep map: wipe every row and reinsert from `map`.
/// Used by `build_unified_dep_map` / the full-rebuild path.
pub fn replace_dep_map(
    conn: &mut Connection,
    map: &HashMap<String, Vec<String>>,
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(DELETE_DEPENDENCIES_SQL, [])?;
    {
        let mut stmt = tx.prepare(INSERT_DEPENDENCY_SQL)?;
        for (src, tests) in map {
            for t in tests {
                stmt.execute(params![src, t])?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// Merge one test file's newly-observed source deps into the dep map.
///
/// Mirrors the semantics of the old `json_cache::merge_deps`: the test file
/// now claims every source in `sources`; any previous edge
/// `(old_source, test_file)` for a source *not* in `sources` is removed.
pub fn merge_deps_for_test(
    conn: &mut Connection,
    test_file: &str,
    sources: &[String],
) -> Result<()> {
    // Cheapest correct implementation: delete all edges for this test, then
    // insert the new set. Since the set is authoritative for the test file,
    // there's no information to preserve.
    store_dep_map_for_test(conn, test_file, sources)
}

// -- Hashes -------------------------------------------------------------------

/// Load every `(file, hash)` row. The u64 bit pattern is preserved across
/// the i64 cast in the column.
pub fn load_hashes(conn: &Connection) -> HashMap<PathBuf, u64> {
    let mut out = HashMap::new();
    let mut stmt = match conn.prepare(SELECT_HASHES_SQL) {
        Ok(s) => s,
        Err(_) => return out,
    };
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    });
    if let Ok(iter) = rows {
        for (path, h) in iter.flatten() {
            out.insert(PathBuf::from(path), h as u64);
        }
    }
    out
}

/// Upsert every `(file, hash)` pair. The provided map is authoritative:
/// rows for files not in `hashes` are removed first so deletes on disk
/// propagate.
pub fn store_hashes(
    conn: &mut Connection,
    hashes: &HashMap<PathBuf, u64>,
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute(DELETE_HASHES_SQL, [])?;
    {
        let mut stmt = tx.prepare(UPSERT_HASH_SQL)?;
        for (path, h) in hashes {
            let Some(s) = path.to_str() else {
                // Skip non-UTF-8 paths. scrutin works on text paths anyway;
                // these never show up in real fixtures.
                continue;
            };
            stmt.execute(params![s, *h as i64])?;
        }
    }
    tx.commit()?;
    Ok(())
}

// -- Stats queries ------------------------------------------------------------

/// Flaky tests: subjects that both passed and failed across the most recent
/// runs. Uses the `retries > 0 AND outcome = 'pass'` signal from `results`
/// to surface tests that needed retries within a single run.
pub fn flaky_tests(conn: &Connection) -> Result<Vec<FlakyTest>> {
    let mut stmt = conn.prepare(FLAKY_TESTS_SQL)?;
    let rows = stmt.query_map(params![FLAKY_RECENT_RUNS, FLAKY_MIN_RUNS], |r| {
        let failures: i64 = r.get(2)?;
        let retry_passes: i64 = r.get(3)?;
        let total: i64 = r.get(4)?;
        // Define flake rate as max(failures, retry_passes) / total so
        // retries-on-pass count toward flakiness too.
        let observed = failures.max(retry_passes);
        let flake_rate = if total > 0 {
            observed as f64 / total as f64
        } else {
            0.0
        };
        Ok(FlakyTest {
            file: r.get(0)?,
            test: r.get(1)?,
            failures,
            total,
            flake_rate,
        })
    })?;
    let mut out: Vec<FlakyTest> = rows
        .filter_map(Result::ok)
        .filter(|t| t.flake_rate > FLAKY_RATE_LOW && t.flake_rate < FLAKY_RATE_HIGH)
        .collect();
    out.sort_by(|a, b| {
        b.flake_rate
            .partial_cmp(&a.flake_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

/// Tests with the highest average duration across all recorded runs.
pub fn slow_tests(conn: &Connection) -> Result<Vec<SlowTest>> {
    let mut stmt = conn.prepare(SLOW_TESTS_SQL)?;
    let rows = stmt.query_map(params![SLOW_MIN_RUNS, SLOW_MIN_AVG_MS, SLOW_LIMIT], |r| {
        Ok(SlowTest {
            file: r.get(0)?,
            test: r.get(1)?,
            avg_ms: r.get(2)?,
            max_ms: r.get::<_, i64>(3)? as f64,
            runs: r.get(4)?,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

// -- Convenience wrappers for callers that only hold an `&Path` ---------------

/// Open a short-lived connection, invoke `f`, and drop the connection. Most
/// callers (reporter persistence, TUI dep-map merges) hold only the project
/// root, so this wrapper keeps their call-sites ergonomic.
pub fn with_open<T, F: FnOnce(&mut Connection) -> Result<T>>(
    root: &Path,
    f: F,
) -> Result<T> {
    let mut conn = open(root)?;
    f(&mut conn)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::protocol::{Event, Message, Outcome, Subject};

    fn ev(file: &str, outcome: Outcome, name: &str) -> Message {
        Message::Event(Event {
            file: file.into(),
            outcome,
            subject: Subject {
                kind: "test".into(),
                name: name.into(),
                parent: None,
            },
            metrics: None,
            failures: Vec::new(),
            message: None,
            line: None,
            duration_ms: 12,
        })
    }

    #[test]
    fn dep_map_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut conn = open(root).unwrap();
        store_dep_map_for_test(
            &mut conn,
            "tests/test-a.R",
            &["R/a.R".into(), "R/b.R".into()],
        )
        .unwrap();
        store_dep_map_for_test(
            &mut conn,
            "tests/test-c.R",
            &["R/b.R".into()],
        )
        .unwrap();
        let map = load_dep_map(&conn);
        assert_eq!(map["R/a.R"], vec!["tests/test-a.R"]);
        let mut bs = map["R/b.R"].clone();
        bs.sort();
        assert_eq!(bs, vec!["tests/test-a.R", "tests/test-c.R"]);
    }

    #[test]
    fn hashes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut conn = open(root).unwrap();
        let mut hashes = HashMap::new();
        hashes.insert(PathBuf::from("R/a.R"), u64::MAX - 7);
        hashes.insert(PathBuf::from("R/b.R"), 42u64);
        store_hashes(&mut conn, &hashes).unwrap();
        let loaded = load_hashes(&conn);
        assert_eq!(loaded[&PathBuf::from("R/a.R")], u64::MAX - 7);
        assert_eq!(loaded[&PathBuf::from("R/b.R")], 42);
    }

    #[test]
    fn fresh_db_returns_empty_maps() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(dir.path()).unwrap();
        assert!(load_dep_map(&conn).is_empty());
        assert!(load_hashes(&conn).is_empty());
    }

    #[test]
    fn record_run_inserts_and_queries_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        let mut prov = Provenance::default();
        prov.scrutin_version = "0.0.1-test".into();
        prov.hostname = Some("host".into());
        prov.os_platform = Some("linux".into());
        prov.git_commit = Some("abcdef".into());
        prov.git_dirty = Some(false);

        let rows = vec![ResultRow {
            file: "tests/test-a.R".into(),
            tool: "testthat".into(),
            tool_version: Some("3.2.0".into()),
            app_name: Some("demo".into()),
            app_version: Some("0.1".into()),
            messages: vec![
                ev("tests/test-a.R", Outcome::Pass, "passes"),
                ev("tests/test-a.R", Outcome::Fail, "breaks"),
            ],
            retries: 0,
        }];
        record_run(&mut conn, "run-1", "2026-04-14T00:00:00Z", &prov, &rows).unwrap();

        // runs row present
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 1);
        // two result rows (one per event)
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM results", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 2);
        // tool column populated
        let tool: String = conn
            .query_row("SELECT tool FROM results LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tool, "testthat");
    }

    #[test]
    fn record_extras_upserts() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        // Insert a run so the foreign key resolves.
        let mut prov = Provenance::default();
        prov.scrutin_version = "0.0.1".into();
        record_run(&mut conn, "run-1", "ts", &prov, &[]).unwrap();
        let mut m = std::collections::BTreeMap::new();
        m.insert("build".into(), "4521".into());
        m.insert("deploy".into(), "staging".into());
        record_extras(&mut conn, "run-1", &m).unwrap();
        let v: String = conn
            .query_row(
                "SELECT value FROM extras WHERE run_id=?1 AND key=?2",
                params!["run-1", "build"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "4521");
    }

    #[test]
    fn legacy_json_sidecars_removed_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".scrutin")).unwrap();
        std::fs::write(root.join(".scrutin/depmap.json"), b"{}").unwrap();
        std::fs::write(root.join(".scrutin/hashes.json"), b"{}").unwrap();
        let _ = open(root).unwrap();
        assert!(!root.join(".scrutin/depmap.json").exists());
        assert!(!root.join(".scrutin/hashes.json").exists());
    }

    #[test]
    fn non_sqlite_state_db_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".scrutin")).unwrap();
        std::fs::write(root.join(".scrutin/state.db"), b"not a sqlite file").unwrap();
        let _ = open(root).unwrap();
        assert!(is_sqlite_file(&root.join(".scrutin/state.db")));
    }

    // ── Dep map: full-wipe and merge semantics (spec §3.10) ─────────────────

    #[test]
    fn replace_dep_map_wipes_existing_rows() {
        // Seed the dep-map with some rows, then call replace_dep_map with a
        // different map. Nothing from the original should survive: full
        // replace is authoritative.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        store_dep_map_for_test(&mut conn, "tests/test-old.R", &["R/old.R".into()]).unwrap();

        let mut fresh: HashMap<String, Vec<String>> = HashMap::new();
        fresh.insert("R/new.R".into(), vec!["tests/test-new.R".into()]);
        replace_dep_map(&mut conn, &fresh).unwrap();

        let loaded = load_dep_map(&conn);
        assert_eq!(loaded.len(), 1, "replace must wipe pre-existing rows; got {loaded:?}");
        assert_eq!(loaded["R/new.R"], vec!["tests/test-new.R"]);
        assert!(
            !loaded.contains_key("R/old.R"),
            "old source key must not survive replace; got {loaded:?}"
        );
    }

    #[test]
    fn merge_deps_for_test_replaces_prior_edges_for_same_test() {
        // The test file's dep set is authoritative: running it again with a
        // smaller dep list must *remove* sources it no longer uses.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();

        // First run: test-a uses both R/a.R and R/b.R.
        merge_deps_for_test(
            &mut conn,
            "tests/test-a.R",
            &["R/a.R".into(), "R/b.R".into()],
        )
        .unwrap();
        // Another test also depends on R/b.R (so R/b.R has two edges).
        merge_deps_for_test(&mut conn, "tests/test-c.R", &["R/b.R".into()]).unwrap();

        // Second run of test-a: it now only uses R/a.R. The R/b.R edge
        // *from test-a* must be removed, but R/b.R's edge from test-c
        // must survive.
        merge_deps_for_test(&mut conn, "tests/test-a.R", &["R/a.R".into()]).unwrap();

        let map = load_dep_map(&conn);
        assert_eq!(
            map.get("R/a.R").map(|v| v.as_slice()),
            Some(&["tests/test-a.R".to_string()][..]),
        );
        // R/b.R should still map to test-c but NOT test-a anymore.
        let b_tests = map.get("R/b.R").cloned().unwrap_or_default();
        assert!(
            b_tests.contains(&"tests/test-c.R".to_string()),
            "R/b.R must still map to test-c after test-a drops it; got {map:?}"
        );
        assert!(
            !b_tests.contains(&"tests/test-a.R".to_string()),
            "R/b.R must NOT map to test-a after it's merged away; got {map:?}"
        );
    }

    // ── Hashes: authoritative replace semantics ─────────────────────────────

    #[test]
    fn store_hashes_removes_rows_for_deleted_files() {
        // If a file was in the DB but is missing from the new hashes map,
        // it's been deleted on disk and its row must go away. Otherwise
        // dep-map staleness checks would keep thinking the file exists.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();

        let mut first = HashMap::new();
        first.insert(PathBuf::from("R/a.R"), 100u64);
        first.insert(PathBuf::from("R/b.R"), 200u64);
        store_hashes(&mut conn, &first).unwrap();
        assert_eq!(load_hashes(&conn).len(), 2);

        // Second store: only a.R; b.R was deleted on disk.
        let mut second = HashMap::new();
        second.insert(PathBuf::from("R/a.R"), 100u64);
        store_hashes(&mut conn, &second).unwrap();

        let loaded = load_hashes(&conn);
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key(&PathBuf::from("R/a.R")));
        assert!(
            !loaded.contains_key(&PathBuf::from("R/b.R")),
            "store_hashes must delete rows absent from the new map"
        );
    }

    // ── flaky_tests algorithm ──────────────────────────────────────────────

    /// Seed a run with one ResultRow per (file, outcome) pair. Each row
    /// owns one event whose subject name is `test` (so flaky stats
    /// aggregate across runs).
    fn seed_run(
        conn: &mut Connection,
        run_id: &str,
        timestamp: &str,
        file: &str,
        test: &str,
        outcome: Outcome,
        retries: u32,
    ) {
        let prov = Provenance {
            scrutin_version: "0.0.0-test".into(),
            ..Default::default()
        };
        let rows = vec![ResultRow {
            file: file.into(),
            tool: "testthat".into(),
            tool_version: None,
            app_name: None,
            app_version: None,
            messages: vec![Message::Event(Event {
                file: file.into(),
                outcome,
                subject: Subject {
                    kind: "test".into(),
                    name: test.into(),
                    parent: None,
                },
                metrics: None,
                failures: Vec::new(),
                message: None,
                line: None,
                duration_ms: 10,
            })],
            retries,
        }];
        record_run(conn, run_id, timestamp, &prov, &rows).unwrap();
    }

    #[test]
    fn flaky_detects_test_with_mixed_outcomes() {
        // 5 runs: 3 pass, 2 fail. Rate 2/5 = 0.4, inside (LOW=0.10, HIGH=0.90).
        // Min runs = 3 is satisfied. Must be flagged flaky.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        let outcomes = [Outcome::Pass, Outcome::Fail, Outcome::Pass, Outcome::Fail, Outcome::Pass];
        for (i, o) in outcomes.iter().enumerate() {
            seed_run(
                &mut conn,
                &format!("run-{i}"),
                &format!("2026-04-{:02}", i + 1),
                "tests/test-x.R",
                "flappy",
                *o,
                0,
            );
        }
        let flakies = flaky_tests(&conn).unwrap();
        assert!(
            flakies.iter().any(|f| f.test == "flappy"),
            "mixed-outcome test must be flagged flaky; got {} entries",
            flakies.len()
        );
    }

    #[test]
    fn flaky_does_not_detect_always_pass() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        for i in 0..5 {
            seed_run(
                &mut conn,
                &format!("run-{i}"),
                &format!("2026-04-{:02}", i + 1),
                "tests/test-x.R",
                "stable",
                Outcome::Pass,
                0,
            );
        }
        let flakies = flaky_tests(&conn).unwrap();
        assert!(
            flakies.iter().all(|f| f.test != "stable"),
            "always-pass test must not be flaky"
        );
    }

    #[test]
    fn flaky_does_not_detect_always_fail() {
        // Rate 1.0 is outside (LOW, HIGH) -- a broken test, not flaky.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        for i in 0..5 {
            seed_run(
                &mut conn,
                &format!("run-{i}"),
                &format!("2026-04-{:02}", i + 1),
                "tests/test-x.R",
                "broken",
                Outcome::Fail,
                0,
            );
        }
        let flakies = flaky_tests(&conn).unwrap();
        assert!(
            flakies.iter().all(|f| f.test != "broken"),
            "always-fail test must not be flaky (it's just broken); got {} entries",
            flakies.len()
        );
    }

    #[test]
    fn flaky_requires_minimum_run_count() {
        // Only 2 runs of a mixed-outcome test: below FLAKY_MIN_RUNS=3.
        // Must not be flagged, even though the outcomes are mixed.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        seed_run(&mut conn, "run-0", "2026-04-01", "tests/test-x.R", "mixed", Outcome::Pass, 0);
        seed_run(&mut conn, "run-1", "2026-04-02", "tests/test-x.R", "mixed", Outcome::Fail, 0);
        let flakies = flaky_tests(&conn).unwrap();
        assert!(
            flakies.is_empty(),
            "below min-runs threshold must not flag flaky; got {} entries",
            flakies.len()
        );
    }

    #[test]
    fn flaky_detects_retry_passes() {
        // A test that needed retries to pass counts as flaky even if the
        // final outcome is always pass: the retry itself is the flake
        // signal, per the docstring on the flaky_tests SQL.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        // Three passes, with retries > 0 on at least one: flake signal.
        seed_run(&mut conn, "run-0", "2026-04-01", "tests/test-x.R", "retried", Outcome::Pass, 1);
        seed_run(&mut conn, "run-1", "2026-04-02", "tests/test-x.R", "retried", Outcome::Pass, 0);
        seed_run(&mut conn, "run-2", "2026-04-03", "tests/test-x.R", "retried", Outcome::Pass, 0);
        let flakies = flaky_tests(&conn).unwrap();
        assert!(
            flakies.iter().any(|f| f.test == "retried"),
            "retries-to-pass should still surface as flaky; got {} entries",
            flakies.len()
        );
    }

    // ── slow_tests algorithm ──────────────────────────────────────────────

    fn seed_run_with_duration(
        conn: &mut Connection,
        run_id: &str,
        timestamp: &str,
        file: &str,
        test: &str,
        duration_ms: u64,
    ) {
        let prov = Provenance {
            scrutin_version: "0.0.0-test".into(),
            ..Default::default()
        };
        let rows = vec![ResultRow {
            file: file.into(),
            tool: "testthat".into(),
            tool_version: None,
            app_name: None,
            app_version: None,
            messages: vec![Message::Event(Event {
                file: file.into(),
                outcome: Outcome::Pass,
                subject: Subject {
                    kind: "test".into(),
                    name: test.into(),
                    parent: None,
                },
                metrics: None,
                failures: Vec::new(),
                message: None,
                line: None,
                duration_ms,
            })],
            retries: 0,
        }];
        record_run(conn, run_id, timestamp, &prov, &rows).unwrap();
    }

    #[test]
    fn slow_tests_requires_minimum_run_count() {
        // Single run of a slow test: below SLOW_MIN_RUNS=3.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        seed_run_with_duration(&mut conn, "r0", "2026-04-01", "tests/a.R", "big", 2000);
        seed_run_with_duration(&mut conn, "r1", "2026-04-02", "tests/a.R", "big", 2000);
        let slow = slow_tests(&conn).unwrap();
        assert!(
            slow.is_empty(),
            "below SLOW_MIN_RUNS threshold must not surface; got {} entries",
            slow.len()
        );
    }

    #[test]
    fn slow_tests_filters_below_avg_threshold() {
        // 3 runs averaging 100ms is well under SLOW_MIN_AVG_MS=500.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        for i in 0..3 {
            seed_run_with_duration(
                &mut conn,
                &format!("r{i}"),
                &format!("2026-04-{:02}", i + 1),
                "tests/fast.R",
                "quick",
                100,
            );
        }
        let slow = slow_tests(&conn).unwrap();
        assert!(
            slow.iter().all(|s| s.test != "quick"),
            "fast test must not surface in slow_tests"
        );
    }

    #[test]
    fn slow_tests_surfaces_high_avg_duration() {
        // 3 runs averaging 2000ms: well above the 500ms threshold.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        for i in 0..3 {
            seed_run_with_duration(
                &mut conn,
                &format!("r{i}"),
                &format!("2026-04-{:02}", i + 1),
                "tests/heavy.R",
                "churn",
                2000,
            );
        }
        let slow = slow_tests(&conn).unwrap();
        let entry = slow.iter().find(|s| s.test == "churn");
        assert!(entry.is_some(), "slow test must surface; got {} entries", slow.len());
        let entry = entry.unwrap();
        assert!(entry.avg_ms >= 500.0);
        assert_eq!(entry.runs, 3);
    }

    #[test]
    fn slow_tests_respects_limit() {
        // Seed far more than SLOW_LIMIT=10 slow tests; result must be capped.
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(dir.path()).unwrap();
        for i in 0..15 {
            for j in 0..3 {
                seed_run_with_duration(
                    &mut conn,
                    &format!("r-{i}-{j}"),
                    &format!("2026-04-{:02}", (i * 3 + j) + 1),
                    &format!("tests/heavy-{i}.R"),
                    &format!("slow-{i}"),
                    // Distinct avg_ms per test so ordering is stable.
                    1000 + (i as u64 * 50),
                );
            }
        }
        let slow = slow_tests(&conn).unwrap();
        assert!(
            slow.len() <= SLOW_LIMIT as usize,
            "slow_tests must cap result count at SLOW_LIMIT={SLOW_LIMIT}; got {}",
            slow.len()
        );
        // Results are sorted by avg_ms DESC: the slowest goes first.
        for window in slow.windows(2) {
            assert!(
                window[0].avg_ms >= window[1].avg_ms,
                "slow_tests must sort by avg_ms DESC"
            );
        }
    }
}
