# History

Every *Scrutin* invocation records its outcome to an embedded SQLite database at `.scrutin/state.db` (created and managed by *Scrutin*, gitignored by default). The same file also holds the source-to-test dependency map and the file-hash fingerprints that drive watch mode, so a single delete gives you a clean slate.

No external binary is required: *Scrutin* links SQLite in via `rusqlite` (with the `bundled` feature), so the DB always works regardless of what's installed on the host.

## Reruns and flaky tests

Set `run.reruns` to re-execute failing files:

```bash
scrutin --set run.reruns=2
```

A file that fails and then passes on rerun is marked **flaky**. Flaky results are persisted in the `results` table (via the `retries` column, plus the final `outcome`), surfaced in the plain-mode summary, tagged `scrutin.flaky="true"` in JUnit XML, and queryable via `scrutin stats` (whose flaky-test query is embedded below under [Typical queries](#typical-queries)).

## Run metadata

*Scrutin* records provenance for every run: version, OS, hostname, git SHA, branch, dirty state, CI provider, and CI build identifiers. These values populate the `runs` table (see [Schema](#schema)) and also land in the JUnit `<properties>` block.

Add custom labels with `--set extras.key=value` on the command line, or an `[extras]` section in `.scrutin/config.toml`. Disable provenance capture entirely with `[metadata] enabled = false`.

## Schema

The DDL below is the authoritative schema, loaded verbatim at startup via `CREATE TABLE IF NOT EXISTS`. The same file lives at `crates/scrutin-core/src/storage/sql/schema.sql` and is embedded into the binary with `include_str!`.

<!-- BEGIN schema.sql -->
```sql
CREATE TABLE IF NOT EXISTS runs (
    run_id          TEXT PRIMARY KEY,
    timestamp       TEXT NOT NULL,
    hostname        TEXT,
    ci              TEXT,
    scrutin_version TEXT NOT NULL,

    git_commit      TEXT,
    git_branch      TEXT,
    git_dirty       INTEGER,

    repo_name       TEXT,
    repo_url        TEXT,
    repo_root       TEXT,

    build_number    TEXT,
    build_id        TEXT,
    build_name      TEXT,
    build_url       TEXT,

    os_platform     TEXT,
    os_release      TEXT,
    os_version      TEXT,
    os_arch         TEXT
);

CREATE TABLE IF NOT EXISTS results (
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

CREATE INDEX IF NOT EXISTS idx_results_run_id       ON results(run_id);
CREATE INDEX IF NOT EXISTS idx_results_run_seq      ON results(run_seq);
CREATE INDEX IF NOT EXISTS idx_results_outcome      ON results(outcome);
CREATE INDEX IF NOT EXISTS idx_results_file_subject ON results(file, subject_name);
CREATE INDEX IF NOT EXISTS idx_results_tool         ON results(tool);

CREATE TABLE IF NOT EXISTS extras (
    run_id TEXT NOT NULL,
    key    TEXT NOT NULL,
    value  TEXT NOT NULL,
    PRIMARY KEY (run_id, key),
    FOREIGN KEY (run_id) REFERENCES runs(run_id)
);

CREATE TABLE IF NOT EXISTS dependencies (
    source_file TEXT NOT NULL,
    test_file   TEXT NOT NULL,
    PRIMARY KEY (source_file, test_file)
);

CREATE INDEX IF NOT EXISTS idx_dependencies_test ON dependencies(test_file);

CREATE TABLE IF NOT EXISTS hashes (
    file TEXT PRIMARY KEY,
    hash INTEGER NOT NULL
);
```
<!-- END schema.sql -->

### `runs`

One row per *Scrutin* invocation. Holds all run-level provenance so `results` rows don't repeat it.

| Column | Source | Notes |
|---|---|---|
| `run_id` | scrutin-generated (UUID) | primary key, referenced by all child tables |
| `timestamp` | `chrono::Utc::now().to_rfc3339()` at run start | ISO 8601 |
| `hostname` | `hostname` command (unix) / `%COMPUTERNAME%` (windows) | |
| `ci` | `github` / `gitlab` / `buildkite` / `circleci` / `jenkins` / `azure-pipelines` / `travis` / `ci` / NULL | |
| `scrutin_version` | `CARGO_PKG_VERSION` | version of the *Scrutin* binary that wrote this row |
| `git_commit` | `git rev-parse HEAD` | full 40-char SHA |
| `git_branch` | `git rev-parse --abbrev-ref HEAD`, or `(detached)` | |
| `git_dirty` | `git status --porcelain` non-empty | 1 = uncommitted changes, 0 = clean |
| `repo_name` | derived from `remote.origin.url`, fallback to repo directory name | e.g. `vincentarelbundock/scrutin` |
| `repo_url` | `git config --get remote.origin.url` | NULL on repos with no `origin` remote |
| `repo_root` | absolute path returned by `git rev-parse --show-toplevel` | disambiguates multiple checkouts on the same host |
| `build_number` | CI env var | TEXT, not INTEGER: Azure emits `"20260414.3"` |
| `build_id` | CI env var | opaque stable identifier for the CI run |
| `build_name` | CI env var | workflow / job name |
| `build_url` | CI env var or derived | link to the CI run |
| `os_platform` | `std::env::consts::OS` | `linux` / `macos` / `windows` |
| `os_release` | `uname -r` equivalent | |
| `os_version` | `sw_vers` / `/etc/os-release` / registry | |
| `os_arch` | `std::env::consts::ARCH` | `x86_64` / `aarch64` / ... |

CI env var sources for `build_*`:

| Provider | `build_number` | `build_id` | `build_name` | `build_url` |
|---|---|---|---|---|
| GitHub | `GITHUB_RUN_NUMBER` | `GITHUB_RUN_ID` | `GITHUB_WORKFLOW` | derived |
| GitLab | `CI_PIPELINE_IID` | `CI_PIPELINE_ID` | `CI_JOB_NAME` | `CI_PIPELINE_URL` |
| Buildkite | `BUILDKITE_BUILD_NUMBER` | `BUILDKITE_BUILD_ID` | `BUILDKITE_PIPELINE_SLUG` | `BUILDKITE_BUILD_URL` |
| CircleCI | `CIRCLE_BUILD_NUM` | `CIRCLE_WORKFLOW_ID` | `CIRCLE_JOB` | `CIRCLE_BUILD_URL` |
| Jenkins | `BUILD_NUMBER` | `BUILD_ID` | `JOB_NAME` | `BUILD_URL` |
| Azure Pipelines | `BUILD_BUILDNUMBER` | `BUILD_BUILDID` | `BUILD_DEFINITIONNAME` | derived |
| Travis | `TRAVIS_BUILD_NUMBER` | `TRAVIS_BUILD_ID` | `TRAVIS_REPO_SLUG` | `TRAVIS_BUILD_WEB_URL` |
| Local | NULL | NULL | NULL | NULL |

### `results`

One row per `(run, file, subject)`. The fact table. Tool metadata (`tool`, `tool_version`, `app_name`, `app_version`) is denormalized here rather than stored in a separate `tools` table; at *Scrutin*'s scale the repetition is negligible and queries stay flat.

| Column | Source | Notes |
|---|---|---|
| `run_id` | FK to `runs(run_id)` | groups all rows from one invocation |
| `run_seq` | monotonic counter per run | stable ordering within a run |
| `file` | test file path, repo-relative | e.g. `tests/testthat/test-math.R` |
| `tool` | `Package::suite_for(file)`, plugin identifier | `testthat`, `tinytest`, `pytest`, `jarl`, `ruff`, ... |
| `tool_version` | captured by the plugin at run start | e.g. `packageVersion("testthat")` / `pytest.__version__` |
| `app_name` | `Package:` from `DESCRIPTION` or `[project] name` from `pyproject.toml` | varies per tool in multi-language projects |
| `app_version` | `Version:` from `DESCRIPTION` or `[project] version` | same source as `app_name` |
| `subject_kind` | `Subject::kind` | `file` / `test` / `expectation` |
| `subject_name` | `Subject::name` | e.g. `"addition works"` |
| `subject_parent` | `Subject::parent` | optional parent `describe` / test name |
| `outcome` | six-value taxonomy | `pass` / `fail` / `error` / `skip` / `xfail` / `warn` |
| `duration_ms` | from `summary` NDJSON message | file-level authoritative wall time |
| `retries` | count of re-executions within the run | 0 = first attempt succeeded |
| `total` | data-validation plugins | total expectation count |
| `failed` | data-validation plugins | failed expectation count |
| `fraction` | data-validation plugins | `(total - failed) / total` |

### `extras`

User-supplied key/value labels, populated from `[extras]` in `.scrutin/config.toml` and `--set extras.KEY=VALUE` on the CLI. The only table where key names are not known at schema time. Everything *Scrutin* captures itself has a dedicated column on `runs` or `results`. Typical contents:

```toml
[extras]
environment  = "staging"
experiment   = "new_parser"
feature_flag = "async_pool"
reviewer     = "vincent"
```

### `dependencies`

Source-to-test dependency edges, one row per edge. Each row says "if source file X changes, test file Y is affected." Populated at run start by the dep-map builder and consulted by watch mode to pick affected test files on each save. Has no `run_id`: the dep map is a project-wide cache, not per-run state.

Writes are full-replace per `test_file`: when a test file's imports change, *Scrutin* deletes all existing rows where `test_file = ?` and inserts fresh ones in one transaction.

### `hashes`

Per-file content fingerprints (`xxhash_rust::xxh64`) used to decide whether the dep map is stale. The `u64` hash is stored via an `as i64` cast (bit pattern preserved, equality checks work correctly). No arithmetic is ever performed on these values.

## Typical queries

History of a single file, ordered by time:

```sql
SELECT r.timestamp, r.git_commit, res.outcome, res.duration_ms
FROM results res JOIN runs r USING (run_id)
WHERE res.file = 'tests/testthat/test-math.R'
ORDER BY r.timestamp DESC;
```

Flaky tests over the last 10 runs (`scrutin stats` runs a version of this query; the embedded copy is reproduced below):

<!-- BEGIN flaky_tests.sql -->
```sql
WITH recent AS (
    SELECT run_id FROM runs ORDER BY timestamp DESC LIMIT ?1
),
stats AS (
    SELECT file, subject_name,
           SUM(CASE WHEN outcome IN ('fail','error') THEN 1 ELSE 0 END) AS failures,
           SUM(CASE WHEN retries > 0 AND outcome = 'pass' THEN 1 ELSE 0 END) AS retry_passes,
           COUNT(*) AS total
    FROM results
    WHERE run_id IN (SELECT run_id FROM recent) AND subject_name != ''
    GROUP BY file, subject_name
)
SELECT file, subject_name, failures, retry_passes, total
FROM stats
WHERE total >= ?2
  AND ((failures > 0 AND failures < total) OR retry_passes > 0)
```
<!-- END flaky_tests.sql -->

Slow tests over all recorded runs:

<!-- BEGIN slow_tests.sql -->
```sql
SELECT file, subject_name,
       AVG(duration_ms) AS avg_ms,
       MAX(duration_ms) AS max_ms,
       COUNT(*) AS runs
FROM results
WHERE subject_name != '' AND duration_ms > 0
GROUP BY file, subject_name
HAVING runs >= ?1 AND avg_ms > ?2
ORDER BY avg_ms DESC
LIMIT ?3
```
<!-- END slow_tests.sql -->

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

## What is not in this DB

- **Per-attempt retry detail** (attempt N's duration, failure message, trace): not persisted. `results.retries` gives the count; cross-invocation aggregation over `results` covers the flaky-test question.
- **Computed insights** (pass rate, flaky rate, p95 duration, baseline deltas): not materialized. Computed on demand by `scrutin stats`.
- **Schema version**: no `schemas` table. On schema change during development, delete `.scrutin/state.db` and re-run.
