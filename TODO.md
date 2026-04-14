# TODO

## Dep

- [ ] R instrumentation

## Plugin runners

- [ ] Review manually
- [ ] What do we assume there?

## Release blockers (v0.1)

- [ ] Air-based R dep-map parser (replace tree-sitter-r with `air_r_parser`)
- [ ] cargo-dist, Homebrew tap

## Tool plugins

- [ ] pandera (Python): `mod.rs` + `plugin.rs` + runner under `python/pandera/`
- [ ] pydantic (Python): same pattern under `python/pydantic/`

## Markers and test metadata

- [ ] Tag-based filtering (`-t TAG`): companions extract tags, scrutin filters
- [ ] `xfail` for testthat/tinytest (pytest has native support)

## Coverage

- [ ] Dispatch to `covr` (R) and `coverage.py` (pytest), aggregate, emit summary
- [ ] Config: `[coverage]` `enabled`, `report` (`"term"`, `"html"`, `"lcov"`)

## Infrastructure

- [ ] Expand internal test suite
- [ ] Review DuckDB schema
- [ ] `xfail`-leak warning in `scrutin stats`
- [ ] Profiler infrastructure (warm vs. cold)
- [ ] Minimize dependencies

## History dashboard

- [ ] Flaky test report
- [ ] Historical benchmark

## CTRF

Producer-angle opportunity: no pytest or R CTRF producer exists in the ecosystem (Apr 2026). Reporter work is tracked on the roadmap page; the items below are DB schema + capture changes that stand on their own (they improve `scrutin stats` and history queries) and happen to make a future CTRF reporter a straight rename-at-emit projection. Authoritative schema: `github.com/ctrf-io/ctrf:schema/ctrf.schema.json`.

**Naming convention**: snake_case in the DB, grouped by prefix (`git_`, `build_`, `os_`, `app_`, `scrutin_`, `tool_`). The CTRF reporter handles camelCase projection at emit time.

- [ ] Promote every scrutin-captured provenance field to a `test_runs` column (today they live in the `run_metadata` key/value table). Rule: if scrutin knows the field name, it's a column. `run_metadata` is reserved for user-supplied `[metadata.extra]` labels only.
  - `git_commit` (rename existing `git_sha`), `git_branch`
  - `build_number` (**TEXT**, not INTEGER: Azure Pipelines emits `"20260414.3"`), `build_id`, `build_name`, `build_url`
  - `os_platform` (`linux`/`darwin`/`win32`), `os_release`, `os_version`
  - `scrutin_version` (the scrutin binary that produced the run)
  - `test_environment` (free-form: `ci` / `local` / `staging`)

- [ ] New table `project_info(git_repo_name, git_repo_url)`: single row, populated at DB init. The DB is scoped to one project, so these values never vary across runs; storing them per-row is waste. Anything else that's constant across all runs in the DB also lives here.

- [ ] Narrow `run_metadata` to user-supplied extras only: `[metadata.extra.*]` from config and `--set metadata.extra.*` from CLI. Remove any scrutin-captured keys still living there after the column promotion above.

- [ ] Extend `metadata::capture_provenance` (crates/scrutin-core/src/metadata.rs:48) to populate `build_*` from CI env vars. `detect_ci` already recognizes 7 providers; refactor to return `(provider, build_fields)`. Per-provider sources:
  - GitHub: `GITHUB_RUN_NUMBER` / `GITHUB_RUN_ID` / `GITHUB_WORKFLOW` / derived `$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID`
  - GitLab: `CI_PIPELINE_IID` / `CI_PIPELINE_ID` / `CI_JOB_NAME` / `CI_PIPELINE_URL`
  - Buildkite: `BUILDKITE_BUILD_NUMBER` / `_ID` / `_PIPELINE_SLUG` / `_URL`
  - CircleCI: `CIRCLE_BUILD_NUM` / `CIRCLE_WORKFLOW_ID` / `CIRCLE_JOB` / `CIRCLE_BUILD_URL`
  - Jenkins: `BUILD_NUMBER` / `BUILD_ID` / `JOB_NAME` / `BUILD_URL`
  - Azure: `BUILD_BUILDNUMBER` / `BUILD_BUILDID` / `BUILD_DEFINITIONNAME` / derived from `SYSTEM_TEAMFOUNDATIONCOLLECTIONURI`
  - Travis: `TRAVIS_BUILD_NUMBER` / `TRAVIS_BUILD_ID` / `TRAVIS_REPO_SLUG` / `TRAVIS_BUILD_WEB_URL`
  - Local: all NULL. `[metadata.extra]` + `--set metadata.extra.build_number=...` still work as overrides.

- [ ] New table `tools(run_id, tool, version, app_name, app_version)`: one row per active tool (testthat 3.2.1, pytest 8.1.0, ruff 0.4.0, ...). Captured once per run, not per file. `tool` is the routing key already present on `Package::test_suites`. `app_name` / `app_version` live here rather than globally because multi-language projects (R + Python side-by-side, like `demo/`) have one app per tool: `scrutindemo` from `DESCRIPTION`, `scrutindemo_py` from `pyproject.toml`. This also matches CTRF, which has singular `appName` / `appVersion` / `tool` per report, so a multi-tool scrutin run emits one CTRF file per tool.

- [ ] Add `retries INTEGER NOT NULL DEFAULT 0` column to `test_runs` (the count of extra attempts the file needed within a single invocation; data already exists in `RunAccumulator`). Makes `GROUP BY file ORDER BY AVG(retries) DESC` rank retry-prone files without a join. The existing `rerun_flaky` boolean becomes redundant (`retries > 0 AND outcome = 'pass'`) : drop it. Skip per-attempt detail (duration, message, trace) for now : cross-invocation aggregation over `test_runs` already answers the flaky-test report question; per-attempt rows are a future addition only if users actually ask for diagnostic depth.

- [ ] `scrutin stats` diff view: "what changed since last run" off existing DuckDB history. CTRF's `baseline` object (`reportId`, `commit`, `buildNumber`, `timestamp`) is the target shape: keep internal field names snake_case (`baseline_commit`, `baseline_build_number`, ...).

- [ ] `scrutin stats` insights: pass/fail/flaky rates, average and p95 test duration, executed-in-N-runs, each as a `{current, baseline, change}` delta. Good schema target for the flaky-test report + historical benchmark items under **History dashboard** above.

## Future

- [ ] Windows support
- [ ] Auto-`devtools::document()` on NAMESPACE changes (opt-in)
