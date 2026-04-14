# History

Every scrutin invocation records its outcome to an embedded SQLite database at `.scrutin/state.db` (created and managed by scrutin, gitignored by default). The same file also holds the source-to-test dependency map and the file-hash fingerprints that drive watch mode, so a single delete gives you a clean slate.

No external binary is required: scrutin links SQLite in via `rusqlite` (with the `bundled` feature), so the DB always works regardless of what's installed on the host.

## Schema

The DDL below is the authoritative schema, loaded verbatim at startup via `CREATE TABLE IF NOT EXISTS`. The same file lives at `crates/scrutin-core/src/storage/sql/schema.sql` and is embedded into the binary with `include_str!`.

<!-- BEGIN schema.sql -->
<!-- END schema.sql -->

### `runs`

One row per scrutin invocation. Holds all run-level provenance so `results` rows don't repeat it.

| Column | Source | Notes |
|---|---|---|
| `run_id` | scrutin-generated (UUID) | primary key, referenced by all child tables |
| `timestamp` | `chrono::Utc::now().to_rfc3339()` at run start | ISO 8601 |
| `hostname` | `hostname` command (unix) / `%COMPUTERNAME%` (windows) | |
| `ci` | `github` / `gitlab` / `buildkite` / `circleci` / `jenkins` / `azure-pipelines` / `travis` / `ci` / NULL | |
| `scrutin_version` | `CARGO_PKG_VERSION` | version of the scrutin binary that wrote this row |
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

One row per `(run, file, subject)`. The fact table. Tool metadata (`tool`, `tool_version`, `app_name`, `app_version`) is denormalized here rather than stored in a separate `tools` table; at scrutin's scale the repetition is negligible and queries stay flat.

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

User-supplied key/value labels, populated from `[extras]` in `.scrutin/config.toml` and `--set extras.KEY=VALUE` on the CLI. The only table where key names are not known at schema time. Everything scrutin captures itself has a dedicated column on `runs` or `results`. Typical contents:

```toml
[extras]
environment  = "staging"
experiment   = "new_parser"
feature_flag = "async_pool"
reviewer     = "vincent"
```

### `dependencies`

Source-to-test dependency edges, one row per edge. Each row says "if source file X changes, test file Y is affected." Populated at run start by the dep-map builder and consulted by watch mode to pick affected test files on each save. Has no `run_id`: the dep map is a project-wide cache, not per-run state.

Writes are full-replace per `test_file`: when a test file's imports change, scrutin deletes all existing rows where `test_file = ?` and inserts fresh ones in one transaction.

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
<!-- END flaky_tests.sql -->

Slow tests over all recorded runs:

<!-- BEGIN slow_tests.sql -->
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
