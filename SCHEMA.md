# SQLite schema proposal

Implementation plan for scrutin's local persistence, covering run history, the dep-map, and file-hash fingerprints. Supersedes the current split (DuckDB CLI for history, JSON sidecars for caches) with a single embedded SQLite database at `.scrutin/state.db`, accessed via `rusqlite` with the `bundled` feature so no external binary is required.

scrutin is unreleased, so this is a clean break: on first use after the switch, any existing `.scrutin/state.db`, `.scrutin/depmap.json`, `.scrutin/hashes.json` are deleted and recreated.

## Why SQLite

1. **No external binary dependency.** Current `duckdb_cli.rs` shells out to a `duckdb` CLI that most users don't have installed, so `duckdb_available()` silently no-ops and history capture does nothing for them. `rusqlite` (with the `bundled` Cargo feature) compiles SQLite directly into the scrutin binary. Always works.
2. **Right tool for the workload.** scrutin's queries are point lookups, moderate aggregations, window functions for trend analysis. None of that needs DuckDB's columnar engine at the volumes a personal history DB produces (tens of thousands of rows).
3. **One store instead of two.** Folding the dep-map and file-hash caches into the same DB removes the JSON sidecar layer and its atomic-write dance. One file to delete for a clean slate; one file to back up.
4. **Remote sharing is not a design goal for the local store.** Neither DuckDB nor SQLite supports safe concurrent writes over network filesystems. Team sharing goes through reporters (JUnit, GitHub Actions, future CTRF) and out-of-band aggregation, not the local DB.

## Design principles

1. **snake_case everywhere**, grouped by prefix (`git_`, `build_`, `os_`, `app_`, `scrutin_`, `tool_`) for self-documenting columns and easy `SELECT prefix_*` queries.
2. **Plural nouns for table names.** No `_info`, `_metadata`, `_data` suffixes. No `test_` prefix: the whole DB is about tests, prefixing everything is noise.
3. **Normalized by scope, but denormalize when the parent table is tiny.** Run-level fields live on `runs`. File-level results live on `results`. Per-tool attributes (tool version, app name, app version) are denormalized onto `results` rather than living in a separate `tools` table: SQLite stores the repeated strings efficiently, queries stay flat, and the parent table had only three columns to begin with.
4. **If scrutin captures it, it's a column.** The `extras` table is reserved for user-supplied `[extras]` labels only. No mixed "some fields are columns, some are keys" ambiguity.
5. **No schema version table.** Unreleased software; on any schema change during development, delete `.scrutin/state.db` and re-run. When we cut v0.1, add migrations if needed.
6. **One store, no sidecars.** The dep-map and file-hash caches live in `dependencies` and `hashes` tables in the same DB. No more `.scrutin/depmap.json` or `.scrutin/hashes.json`.

## Tables

### `runs`

One row per scrutin invocation. Holds all run-level provenance so `results` rows don't repeat it.

```sql
CREATE TABLE runs (
    run_id            TEXT PRIMARY KEY,
    timestamp         TEXT NOT NULL,
    hostname          TEXT,
    ci                TEXT,
    scrutin_version   TEXT NOT NULL,

    git_commit        TEXT,
    git_branch        TEXT,
    git_dirty         INTEGER,

    build_number      TEXT,
    build_id          TEXT,
    build_name        TEXT,
    build_url         TEXT,

    os_platform       TEXT,
    os_release        TEXT,
    os_version        TEXT,
    os_arch           TEXT
);
```

| Column | Source | Notes |
|---|---|---|
| `run_id` | scrutin-generated (UUID) | primary key, referenced by all child tables |
| `timestamp` | `chrono::Utc::now().to_rfc3339()` at run start | ISO 8601 |
| `hostname` | `hostname` command (unix) / `%COMPUTERNAME%` (windows) | already captured in `metadata.rs:132` |
| `ci` | `github` / `gitlab` / `buildkite` / `circleci` / `jenkins` / `azure-pipelines` / `travis` / `ci` / NULL | from `metadata::detect_ci` (`metadata.rs:104`) |
| `scrutin_version` | `env!("CARGO_PKG_VERSION")` | version of the scrutin binary that wrote this row |
| `git_commit` | `git rev-parse HEAD` | full 40-char SHA |
| `git_branch` | `git rev-parse --abbrev-ref HEAD`, or `(detached)` | |
| `git_dirty` | `git status --porcelain` non-empty | 1 = uncommitted changes, 0 = clean (SQLite has no native BOOLEAN) |
| `build_number` | CI env var | **TEXT**, not INTEGER: Azure emits `"20260414.3"` |
| `build_id` | CI env var | opaque stable identifier for the CI run |
| `build_name` | CI env var | workflow / job name |
| `build_url` | CI env var or derived | link to the CI run |
| `os_platform` | `std::env::consts::OS` | `linux` / `macos` / `windows` |
| `os_release` | `uname -r` equivalent | e.g. `24.3.0`, `6.8.0-47-generic` |
| `os_version` | `sw_vers` / `/etc/os-release` / registry | e.g. `macOS 15.3.1`, `Ubuntu 24.04.1 LTS` |
| `os_arch` | `std::env::consts::ARCH` | `x86_64` / `aarch64` / ... |

CI env var sources for `build_*`:

| Provider | `build_number` | `build_id` | `build_name` | `build_url` |
|---|---|---|---|---|
| GitHub | `GITHUB_RUN_NUMBER` | `GITHUB_RUN_ID` | `GITHUB_WORKFLOW` | derived from `$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID` |
| GitLab | `CI_PIPELINE_IID` | `CI_PIPELINE_ID` | `CI_JOB_NAME` | `CI_PIPELINE_URL` |
| Buildkite | `BUILDKITE_BUILD_NUMBER` | `BUILDKITE_BUILD_ID` | `BUILDKITE_PIPELINE_SLUG` | `BUILDKITE_BUILD_URL` |
| CircleCI | `CIRCLE_BUILD_NUM` | `CIRCLE_WORKFLOW_ID` | `CIRCLE_JOB` | `CIRCLE_BUILD_URL` |
| Jenkins | `BUILD_NUMBER` | `BUILD_ID` | `JOB_NAME` | `BUILD_URL` |
| Azure Pipelines | `BUILD_BUILDNUMBER` | `BUILD_BUILDID` | `BUILD_DEFINITIONNAME` | derived from `SYSTEM_TEAMFOUNDATIONCOLLECTIONURI` + project + build_id |
| Travis | `TRAVIS_BUILD_NUMBER` | `TRAVIS_BUILD_ID` | `TRAVIS_REPO_SLUG` | `TRAVIS_BUILD_WEB_URL` |
| Local | NULL | NULL | NULL | NULL |

### `results`

One row per `(run, file, subject)`. The fact table. Tool metadata (`tool`, `tool_version`, `app_name`, `app_version`) is denormalized here rather than stored in a separate `tools` table; at scrutin's scale the repetition is negligible and queries stay flat.

```sql
CREATE TABLE results (
    run_id          TEXT NOT NULL,
    run_seq         INTEGER NOT NULL,
    file            TEXT NOT NULL,
    tool            TEXT NOT NULL,
    tool_version    TEXT,
    app_name        TEXT,
    app_version     TEXT,
    subject_kind    TEXT NOT NULL,
    subject_name    TEXT NOT NULL,
    subject_parent  TEXT,
    outcome         TEXT NOT NULL,
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    retries         INTEGER NOT NULL DEFAULT 0,
    total           INTEGER,
    failed          INTEGER,
    fraction        REAL,
    FOREIGN KEY (run_id) REFERENCES runs(run_id)
);

CREATE INDEX idx_results_run_id       ON results(run_id);
CREATE INDEX idx_results_run_seq      ON results(run_seq);
CREATE INDEX idx_results_outcome      ON results(outcome);
CREATE INDEX idx_results_file_subject ON results(file, subject_name);
CREATE INDEX idx_results_tool         ON results(tool);
```

| Column | Source | Notes |
|---|---|---|
| `run_id` | FK to `runs(run_id)` | groups all rows from one invocation |
| `run_seq` | monotonic counter per run | stable ordering within a run |
| `file` | test file path, repo-relative | e.g. `tests/testthat/test-math.R` |
| `tool` | `Package::suite_for(file)` → plugin identifier | routing key: `testthat`, `tinytest`, `pytest`, `jarl`, `ruff`, ... |
| `tool_version` | `packageVersion("testthat")` / `pytest.__version__` / `ruff --version` / ... | the tool's own version, captured by the plugin at run start |
| `app_name` | `Package:` from `DESCRIPTION` or `[project] name` from `pyproject.toml` | package under test; varies per tool in multi-language projects (`scrutindemo` vs `scrutindemo_py`) |
| `app_version` | `Version:` from `DESCRIPTION` or `[project] version` | same source as `app_name` |
| `subject_kind` | `Subject::kind` | `file` / `test` / `expectation` |
| `subject_name` | `Subject::name` | e.g. `"addition works"` |
| `subject_parent` | `Subject::parent` | optional parent `describe` / test name |
| `outcome` | six-value taxonomy | `pass` / `fail` / `error` / `skip` / `xfail` / `warn` |
| `duration_ms` | from `summary` NDJSON message | file-level authoritative wall time |
| `retries` | count of re-executions within the run | 0 = first attempt succeeded; replaces `rerun_flaky` boolean (derivable as `retries > 0 AND outcome = 'pass'`) |
| `total` | data-validation plugins (pointblank, validate, great_expectations) | total expectation count |
| `failed` | data-validation plugins | failed expectation count |
| `fraction` | data-validation plugins | `(total - failed) / total` |

### `extras`

User-supplied key/value labels attached to a run. Populated from `[extras]` in `.scrutin/config.toml` and `--set extras.KEY=VALUE` on the CLI.

```sql
CREATE TABLE extras (
    run_id  TEXT NOT NULL,
    key     TEXT NOT NULL,
    value   TEXT NOT NULL,
    PRIMARY KEY (run_id, key),
    FOREIGN KEY (run_id) REFERENCES runs(run_id)
);
```

This is the only table where key names are not known at schema time. Everything scrutin captures itself has a dedicated column on `runs` or `results`. Typical contents:

```toml
[extras]
environment  = "staging"
experiment   = "new_parser"
feature_flag = "async_pool"
reviewer     = "vincent"
```

Deployment tier (`environment = "staging"` / `"production"` / ...) lives here rather than on `runs` because scrutin has no way to detect it automatically: users supply it. CTRF's `testEnvironment` projects from `extras.key = 'environment'` at emit time.

### `dependencies`

Source-to-test dependency edges, one row per edge. Replaces `.scrutin/depmap.json`. Each row says "if source file X changes, test file Y is affected."

```sql
CREATE TABLE dependencies (
    source_file  TEXT NOT NULL,
    test_file    TEXT NOT NULL,
    PRIMARY KEY (source_file, test_file)
);

CREATE INDEX idx_dependencies_test ON dependencies(test_file);
```

Populated at run start by the dep-map builder (`r/depmap.rs`, `python/imports.rs`) and consulted by the watch-mode change detector to pick the affected test files on each save. The table has no `run_id`: the dep map is a project-wide cache, not per-run state.

| Column | Notes |
|---|---|
| `source_file` | repo-relative path to a source file (e.g. `R/math.R`, `src/scrutindemo_py/math.py`) |
| `test_file` | repo-relative path to a test file that imports / depends on `source_file` |

The index on `test_file` supports the reverse lookup used when a single test file changes and we need to prune outdated edges.

Writes are full-replace per `test_file`: when a test file's imports change, scrutin deletes all existing `dependencies` rows where `test_file = ?` and inserts fresh ones. Done in one transaction.

### `hashes`

Per-file content fingerprints used to decide whether the dep-map is stale. Replaces `.scrutin/hashes.json`.

```sql
CREATE TABLE hashes (
    file  TEXT PRIMARY KEY,
    hash  INTEGER NOT NULL
);
```

| Column | Notes |
|---|---|
| `file` | repo-relative path |
| `hash` | `xxhash_rust::xxh64` of the file's bytes. SQLite's `INTEGER` is a signed 64-bit value; `u64` hashes are stored via `as i64` cast (bit pattern preserved, equality checks work correctly). No arithmetic is ever performed on these values. |

Writes are upsert per file: `INSERT OR REPLACE INTO hashes (file, hash) VALUES (?, ?)`. Deletes happen when a previously-tracked file no longer exists on disk (cleaned up at dep-map rebuild time).

## What is NOT in this DB

- **Per-attempt retry detail** (attempt N's duration, failure message, trace): intentionally not persisted. `results.retries` gives the count; per-attempt diagnostics are only worth persisting if users request them later, and cross-invocation aggregation over `results` handles the flaky-test report question.
- **Computed insights** (pass rate, flaky rate, p95 duration, baseline deltas): not materialized tables. Computed on demand by `scrutin stats` queries; SQLite window functions + CTEs handle this at the volumes a personal history DB produces.
- **Schema version**: no `schemas` table. On schema change during development, delete and recreate.
- **Repo identity** (name, URL): derivable on demand from `git config`. The DB is per-checkout (`.scrutin/state.db` is `.gitignore`d), so "which repo is this?" is answered by the filesystem.

## Typical queries

History of a single file, ordered by time:

```sql
SELECT r.timestamp, r.git_commit, res.outcome, res.duration_ms
FROM results res JOIN runs r USING (run_id)
WHERE res.file = 'tests/testthat/test-math.R'
ORDER BY r.timestamp DESC;
```

Flaky tests over the last 10 runs:

```sql
WITH recent AS (
    SELECT run_id FROM runs ORDER BY timestamp DESC LIMIT 10
)
SELECT file, subject_name,
       SUM(CASE WHEN outcome IN ('fail','error') THEN 1 ELSE 0 END) AS failures,
       COUNT(*) AS total,
       CAST(SUM(CASE WHEN outcome IN ('fail','error') THEN 1 ELSE 0 END) AS REAL) / COUNT(*) AS flake_rate
FROM results WHERE run_id IN (SELECT run_id FROM recent)
GROUP BY file, subject_name
HAVING total >= 3 AND flake_rate BETWEEN 0.10 AND 0.90;
```

Tests that tend to need retries:

```sql
SELECT file, subject_name, AVG(retries) AS avg_retries, COUNT(*) AS runs
FROM results
WHERE subject_kind = 'file'
GROUP BY file, subject_name
HAVING avg_retries > 0
ORDER BY avg_retries DESC;
```

Tool-version matrix (when did each tool version run?):

```sql
SELECT tool, tool_version, MIN(r.timestamp) AS first_seen, MAX(r.timestamp) AS last_seen
FROM results res JOIN runs r USING (run_id)
GROUP BY tool, tool_version
ORDER BY tool, first_seen;
```

Runs tagged with an experiment label:

```sql
SELECT r.timestamp, r.git_commit, e.value AS experiment
FROM runs r JOIN extras e USING (run_id)
WHERE e.key = 'experiment' AND e.value = 'new_parser'
ORDER BY r.timestamp DESC;
```

Dep-map lookup (which test files does this source file affect?):

```sql
SELECT test_file FROM dependencies WHERE source_file = 'R/math.R';
```

## Implementation plan

1. **Dependencies**: add `rusqlite = { version = "0.32", features = ["bundled"] }` to workspace + scrutin-core Cargo.toml. Drop any DuckDB-CLI machinery.
2. **Replace `crates/scrutin-core/src/storage/duckdb_cli.rs`** with a new `sqlite.rs` that opens `.scrutin/state.db` through a shared `rusqlite::Connection` and runs the DDL above via `CREATE TABLE IF NOT EXISTS`. Public API surface:
   - `open(root: &Path) -> Result<Connection>` : opens + initializes schema.
   - `record_run(&Connection, run_id, timestamp, provenance, results) -> Result<()>` : inserts one row into `runs` and the corresponding rows into `results`, in a single transaction.
   - `record_extras(&Connection, run_id, extras) -> Result<()>`.
   - `load_dep_map(&Connection) -> HashMap<String, Vec<String>>`.
   - `store_dep_map_for_test(&Connection, test_file, sources: &[String])` : transactional delete-then-insert per test file.
   - `load_hashes(&Connection) -> HashMap<PathBuf, u64>`.
   - `store_hashes(&Connection, hashes: &HashMap<PathBuf, u64>)` : upsert all.
   - `flaky_tests(&Connection) -> Vec<FlakyTest>` and `slow_tests(&Connection) -> Vec<SlowTest>` : ports of the existing queries to SQLite syntax.
3. **Delete `crates/scrutin-core/src/storage/json_cache.rs`** and update `storage/mod.rs` to expose `sqlite` only.
4. **Migrate callers**:
   - `scrutin-core::analysis::hashing` : replace `json_cache::load_file_hashes` / `store_file_hashes` with the SQLite equivalents.
   - `scrutin-core::r::depmap` and `scrutin-core::python::imports` : same for dep-map.
   - `scrutin-bin/src/cli/reporter/plain.rs` and any other callers of `record_run` : pass a `Connection` reference.
5. **Extend `metadata::capture_provenance`** (`crates/scrutin-core/src/metadata.rs:48`):
   - Return a typed struct matching the `runs` columns instead of `BTreeMap<String, String>`.
   - Populate `build_*` from CI env vars per the provider table.
   - Populate `os_release` / `os_version` / `os_arch`.
   - Keep the `labels` half of `RunMetadata` as-is; that's what flows into `extras`.
6. **Config rename**: `[metadata.extra]` → `[extras]` in `.scrutin/config.toml` parsing (`crates/scrutin-core/src/project/config.rs`). CLI override `--set metadata.extra.KEY=VALUE` → `--set extras.KEY=VALUE`. Update `crates/scrutin-bin/src/cli/init_template.toml`.
7. **Drop `rerun_flaky` logic** from `flaky_tests` query: compute flakiness as `retries > 0 AND outcome = 'pass'` aggregated cross-run.
8. **Startup cleanup**: on `open()`, if any legacy artifact exists (`.scrutin/depmap.json`, `.scrutin/hashes.json`, or the old DuckDB-format `state.db`), delete it. Fresh DB is created from scratch.
9. **Tests**: port the existing `json_cache` round-trip tests to the SQLite layer. Add a test that covers the full `record_run` insert path and the flaky-test query.
10. **Docs touch-up**: `CLAUDE.md` mentions "DuckDB for history (overkill but cheap and queryable)" : replace with "SQLite (embedded via rusqlite, bundled) for history and local caches." Update the `storage/` directory listing in the architecture section.
