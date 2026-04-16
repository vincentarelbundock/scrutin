# Reporting generalization spec

## Goal

Replace *Scrutin*'s fixed pass/fail/skip/warn/error vocabulary with a single
outcome taxonomy that fits both unit-testing frameworks (testthat,
tinytest, pytest) and data-validation frameworks (pointblank, Great
Expectations, pandera, pydantic). The taxonomy must carry quantitative
metrics (e.g. "1234 of 1M rows failed") and structured per-row failure
detail, not just a free-form `message: String`.

This is a clean break: no compatibility shims, no deprecated fields. The
NDJSON wire protocol, the core types, the database schema, and the JUnit
writer are rewritten in one pass.

## Outcome taxonomy

Six outcomes. Every event a runner emits maps to exactly one.

| Outcome        | Meaning                                                                          | Example                                              |
| -------------- | -------------------------------------------------------------------------------- | ---------------------------------------------------- |
| `pass`         | Assertion held / validation step passed its threshold                            | `expect_equal(add(2,2), 4)`                          |
| `fail`         | Assertion broken / threshold violated                                            | `expect_equal(add(2,2), 5)`                          |
| `error`        | Could not evaluate (exception, missing column, broken setup)                     | `ImportError`, `KeyError`, R `stop()`                |
| `skip`         | Intentionally not run (user `skip()`, platform mismatch, precondition failed)    | `skip_on_cran()`, pointblank `active = FALSE`        |
| `xfail`        | Failed but predicted; does *not* count as a regression                           | `pytest.xfail`, GE `meta.expected_to_fail`           |
| `warn`         | Soft failure: surfaced to the user but does *not* break the build                | pointblank `warn` action, pandera `raise_warning`    |

`xfail` is the load-bearing addition. Every tool has *some*
form of "this failure was predicted, ignore it for CI gating," and today
*Scrutin* has nowhere to put it.

### Severity is derived, not transmitted

Severity is a property of the outcome, not an independent field.
Each consumer (TUI colors, JUnit type strings, DB queries) derives
severity from the outcome directly. Runners can't disagree with the
consumer about whether a warning is actually a warning.

The mapping:

| Outcome              | Severity |
|----------------------|----------|
| `pass`, `skip`, `xfail` | info     |
| `warn`               | warning  |
| `fail`               | error    |
| `error`              | critical |

### `bad_file` rule

`max_fail` and the process exit code key off "did this file have any
*unexpected* breakage". The rule is one line and lives in `tally_messages`:

```rust
t.bad_file = t.failed > 0 || t.errored > 0;
```

`xfailed` and `warned` are deliberately excluded. That's the
entire mechanism that makes `xfail` and "soft" warnings not break CI.

## Wire protocol

NDJSON, one message per line. Four message types : every per-test
observation goes through `Event`; `Deps` carries per-file runtime
dependency observations (R only); `Summary` carries authoritative wall
time; `Done` ends the stream.

### `event`

```json
{
  "type": "event",
  "file": "tests/test_users.py",
  "outcome": "fail",
  "subject": {
    "kind": "expectation",
    "name": "expect_column_values_to_not_be_null",
    "parent": "users.email"
  },
  "metrics": {
    "total": 1000000,
    "failed": 1234,
    "fraction": 0.001234,
    "observed": {"unique_unexpected_values": 17}
  },
  "failures": [
    {"row": 42, "value": null},
    {"row": 87, "value": null}
  ],
  "message": "1234 nulls in column users.email",
  "line": 17,
  "duration_ms": 230
}
```

Every field except `type`, `file`, `outcome`, and `subject` is optional.
A unit-test event collapses to:

```json
{
  "type": "event",
  "file": "tests/testthat/test-math.R",
  "outcome": "pass",
  "subject": {"kind": "function", "name": "add() returns 4"},
  "duration_ms": 3
}
```

Field reference:

| Field      | Type                          | Required | Notes                                                       |
| ---------- | ----------------------------- | -------- | ----------------------------------------------------------- |
| `type`     | `"event"`                     | yes      | Discriminant.                                               |
| `file`     | string                        | yes      | Test file basename.                                         |
| `outcome`  | one of the six values         | yes      | The taxonomy.                                               |
| `subject`  | `Subject`                     | yes      | What was tested. See below.                                 |
| `metrics`  | `Metrics`                     | no       | Quantitative outcome data (data validation).                |
| `failures` | array of `FailureDetail`      | no       | Structured per-row / per-field failure rows.                |
| `message`  | string                        | no       | Human-readable summary. May be multi-line.                  |
| `line`     | integer                       | no       | Line in `file` where the event was emitted.                 |
| `duration_ms` | integer                    | no       | Wall time for this single event, in milliseconds.           |

### `Subject`

The thing being tested. Decoupled from `test: String` because data
validation needs to identify a column inside a table inside a database,
not just a function name.

```json
{
  "kind": "expectation",      // freeform; see "Subject kinds" below
  "name": "rows_distinct",    // local identifier
  "parent": "users"           // optional containing scope
}
```

Subject kinds are freeform strings (not a closed enum) so new plugins
don't have to touch the core crate. Conventional values:

| Kind          | `name` example                | `parent` example   | Used by                |
| ------------- | ----------------------------- | ------------------ | ---------------------- |
| `function`    | `"add() returns 4"`           | `null`             | testthat, tinytest, pytest |
| `step`        | `"col_vals_not_null(email)"`  | `"users"`          | pointblank             |
| `expectation` | `"expect_column_values_to_not_be_null"` | `"users.email"` | Great Expectations     |
| `check`       | `"unique"`                    | `"users.id"`       | pandera                |
| `field`       | `"User.age"`                  | `"User"`           | pydantic               |

### `Metrics`

Optional; only populated when an event has a quantitative result. All
inner fields are independently optional.

```json
{
  "total":   1000000,    // rows / elements considered
  "failed":  1234,       // rows / elements that failed
  "fraction": 0.001234,   // failed / total, precomputed
  "observed": {           // freeform plugin-defined observations
    "min": 0, "max": 99, "null_count": 1234
  }
}
```

`fraction` is precomputed by the runner because the source library is
the authority on how to compute it (pandera and pointblank disagree on
how to count nulls vs total rows; we don't second-guess them).

### `FailureDetail`

A bag of plugin-defined fields per failing element. The TUI / JUnit
renderer treats it as opaque key/value rows.

```json
[
  {"row": 42, "column": "email", "value": null},
  {"row": 87, "column": "email", "value": null}
]
```

Runners SHOULD use stable, lower-snake-case keys so the TUI can render
columns consistently across rebuilds.

### `summary`

Emitted once per file at the end of that file's run. Carries the
authoritative wall time. The `counts` block is a debugging aid / sanity
check : consumers ignore it and tally events directly. This is the only
policy that satisfies the JUnit schema constraint that
`<testsuite tests=N>` equal the actual `<testcase>` count: each `event`
becomes one testcase, so the per-test totals are by-construction equal
across plain mode, JUnit, and the DB. A worker whose `counts` block
disagrees with its emitted events has a bug.

```json
{
  "type": "summary",
  "file": "tests/test_users.py",
  "duration_ms": 1234,
  "counts": {
    "pass": 12,
    "fail": 1,
    "error": 0,
    "skip": 2,
    "xfail": 1,
    "warn": 0
  }
}
```

### `deps`

Emitted by the R runner after each test file completes, listing the
source files whose functions were invoked during that file's tests.
The engine merges these edges into the persistent `source → [tests]`
dep map. Not emitted by Python runners (Python dep analysis is
static, done Rust-side).

```json
{"type": "deps", "file": "test-model.R", "sources": ["R/model.R", "R/utils.R"]}
```

### `done`

End-of-stream marker for the worker → engine channel. No payload.

```json
{"type": "done"}
```

### Cancellation

Cancellation is synthesized by the engine, not the worker. There is no
`cancelled` wire message : when the engine kills a worker mid-file, it
attaches a `cancelled: true` flag to the file's accumulator inside the
core types. Workers never need to know they were cancelled; they just
get SIGTERM.

## Core types

The wire protocol decodes into normalized Rust types in
`engine::protocol`. The TUI / JUnit / DB / plain mode all consume these,
not the wire shape.

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass, Fail, Error, Skip, Xfail, Warn,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Subject {
    pub kind: String,
    pub name: String,
    #[serde(default)] pub parent: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Metrics {
    #[serde(default)] pub total: Option<u64>,
    #[serde(default)] pub failed: Option<u64>,
    #[serde(default)] pub fraction: Option<f64>,
    #[serde(default)] pub observed: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FailureDetail {
    #[serde(flatten)]
    pub fields: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub file: String,
    pub outcome: Outcome,
    pub subject: Subject,
    #[serde(default)] pub metrics: Option<Metrics>,
    #[serde(default)] pub failures: Vec<FailureDetail>,
    #[serde(default)] pub message: Option<String>,
    #[serde(default)] pub line: Option<u32>,
    #[serde(default)] pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Counts {
    #[serde(default)] pub pass: u32,
    #[serde(default)] pub fail: u32,
    #[serde(default)] pub error: u32,
    #[serde(default)] pub skip: u32,
    #[serde(default)] pub xfail: u32,
    #[serde(default)] pub warn: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Summary {
    pub file: String,
    pub duration_ms: u64,
    pub counts: Counts,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Deps {
    pub file: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Event(Event),
    Summary(Summary),
    Deps(Deps),
    Done,
}
```

One per-test event type (`Event`), one per-file dependency observation
(`Deps`, R only), and `Summary` / `Done` framing the run.

## Tally rule

`tally_messages` becomes a six-arm match. This is the *only* place in
the codebase that classifies an event. Events are authoritative for
counts; the summary contributes only `duration_ms`.

```rust
fn tally_messages(messages: &[Message], file_name: &str, ...) -> (FileTally, u64) {
    let mut t = FileTally::default();
    let mut file_ms: u64 = 0;

    for msg in messages {
        match msg {
            Message::Event(e) => {
                match e.outcome {
                    Outcome::Pass  => t.passed  += 1,
                    Outcome::Fail  => t.failed  += 1,
                    Outcome::Error => t.errored += 1,
                    Outcome::Skip  => t.skipped += 1,
                    Outcome::Xfail => t.xfailed += 1,
                    Outcome::Warn  => t.warned  += 1,
                }
                // failure / warning / error details push into Findings
                // for later rendering, exactly as today.
            }
            Message::Summary(s) => file_ms = s.duration_ms,
            Message::Deps(_) | Message::Done => {}
        }
    }

    t.bad_file = t.failed > 0 || t.errored > 0;
    (t, file_ms)
}
```

`FileTally` grows one field:

```rust
struct FileTally {
    passed: u32,
    failed: u32,
    errored: u32,
    skipped: u32,
    xfailed: u32,           // NEW
    warned: u32,
    bad_file: bool,
    cancelled: bool,
}
```

## Plugin trait hooks for UI rendering

Plugins declare their outcome vocabulary so the TUI can hide bucket
filters that would always be empty (e.g. `xfail` is hidden for
plugins that don't support xfail) and so `scrutin stats` can render
per-plugin columns intelligently. Both methods have defaults on
`Plugin` and are overridden per-plugin as needed.

```rust
impl Plugin for SomePlugin {
    // Outcomes this plugin can emit. Default: [Pass, Fail, Error, Skip].
    fn supported_outcomes(&self) -> &'static [Outcome] {
        &[Outcome::Pass, Outcome::Fail, Outcome::Error, Outcome::Skip, Outcome::Warn]
    }

    // Short label for this plugin's notion of "subject". Default: "test".
    // Data validators override to "step" / "check" / "expectation" / etc.
    fn subject_label(&self) -> &'static str { "step" }
}
```

## Consumer mapping

| Consumer         | Behavior                                                                                  |
| ---------------- | ----------------------------------------------------------------------------------------- |
| **Plain mode**   | New colors: `xfail` dim green, `warn` yellow, `fail` red, `error` red+bold.               |
| **TUI list**     | New status filter chips for `xfail` and `warn`. `bad_file` rule unchanged.                |
| **TUI detail**   | New "Failures" sub-pane renders `failures: Vec<FailureDetail>` as a table when present.   |
| **JUnit**        | `pass` → `<testcase>`. `fail`, `error` → `<failure>` / `<error>` with CDATA body. `skip`, `xfail` → `<skipped>` (the latter with `message="expected"` so CI consumers can filter). `warn` → `<testcase>` with no child. `metrics` and `failures` serialize into `<system-out>` as JSON. |
| **DB**           | `test_runs.outcome TEXT NOT NULL` replaces `passed BOOLEAN`. New `metrics_json TEXT` and `subject_kind TEXT` columns for queryable history. |
| **Stats verb**   | New per-outcome breakdown columns. `flaky_tests` keys off `outcome IN ('fail','error')` instead of `passed = false`. |

## Database schema

Bumped to v3, drop-and-recreate (state.db is gitignored).

```sql
CREATE TABLE test_runs (
    run_seq      BIGINT NOT NULL,    -- monotonic, populated by sequence
    run_id       TEXT NOT NULL,
    timestamp    TEXT NOT NULL,
    file         TEXT NOT NULL,
    subject_kind TEXT NOT NULL,      -- "function" | "step" | "expectation" | …
    subject_name TEXT NOT NULL,
    subject_parent TEXT,
    outcome      TEXT NOT NULL,      -- one of the six taxonomy values
    duration_ms  INTEGER NOT NULL DEFAULT 0,
    total        BIGINT,             -- from Metrics, when present
    failed       BIGINT,
    fraction     DOUBLE,
    git_sha      TEXT,
    rerun_flaky  BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX idx_test_runs_run_seq    ON test_runs(run_seq);
CREATE INDEX idx_test_runs_outcome    ON test_runs(outcome);
CREATE INDEX idx_test_runs_file_subject ON test_runs(file, subject_name);
```

`flaky_tests` becomes:

```sql
WITH recent AS (
    SELECT file, subject_name, outcome,
           ROW_NUMBER() OVER (PARTITION BY file, subject_name ORDER BY run_seq DESC) AS rn
    FROM test_runs
    WHERE subject_name != ''
)
SELECT file, subject_name,
       COUNT(*) FILTER (WHERE outcome IN ('fail','error')) AS failures,
       COUNT(*) AS total
FROM recent
WHERE rn <= 10
GROUP BY file, subject_name
HAVING total >= 3
```

Note that `xfail` is excluded from the `failures` count : a test
that's been `xfail`-ed for 30 runs is not flaky, it's known-broken.

## Plugin implementations

Detection runs against project markers; the runner script imports the
target library and emits the wire format. R worker-mode plugins share a
data-driven `RPlugin` struct in `r/mod.rs` with flat `runner_<name>.R`
files; command-mode and Python plugins live in their own subdirectories.

```
crates/scrutin-core/src/
├── python/
│   ├── mod.rs
│   ├── imports.rs           ← line-based import scanner (dep map)
│   ├── pytest/              ← unit tests (worker)
│   ├── great_expectations/  ← data validation (worker)
│   └── ruff/                ← linter (command mode)
└── r/
    ├── mod.rs               ← RPlugin entries for testthat / tinytest / pointblank / validate
    ├── depmap.rs
    ├── runner_r.R           ← shared R companion (prepended to each per-tool runner at compile time)
    ├── runner_testthat.R
    ├── runner_tinytest.R
    ├── runner_pointblank.R
    ├── runner_validate.R
    └── jarl/                ← linter (command mode)
```

### pointblank (R)

- **Detect**: `DESCRIPTION` + `tests/pointblank/` directory.
- **Subject**: `{kind: "step", name: "<assertion>(<column>)", parent: "<table>"}`.
- **Outcome mapping**:
  - step passed threshold → `pass`
  - `warn` action triggered → `warn`
  - `notify` action triggered → `warn`
  - `stop` action triggered → `fail`
  - `active = FALSE` or precondition failed → `skip`
  - evaluation error → `error`
- **Metrics**: `total = n`, `failed = n_failed`, `fraction = f_failed`.
- **`supported_outcomes`**: `[pass, fail, error, skip, warn]`.
- **`subject_label`**: `"step"`.

### validate (R)

- **Detect**: `DESCRIPTION` + `tests/validate/` directory.
- **Subject**: `{kind: "rule", name: "<rule_name>", parent: "<validation_object_name>"}`.
- **Outcome mapping**:
  - `error == TRUE` in summary → `error`
  - `fails > 0` → `fail`
  - `warning == TRUE` and `fails == 0` → `warn`
  - `fails == 0` and `error == FALSE` → `pass`
- **Metrics**: `total = items`, `failed = fails`, `fraction = fails/items`, `na = nNA`. The `na` key is unique to validate: it counts rows where the rule evaluated to `NA` (inconclusive due to missing data), which is neither a pass nor a fail.
- **`supported_outcomes`**: `[pass, fail, error, warn]`.
- **`subject_label`**: `"rule"`.

### Great Expectations (Python)

- **Detect**: `tests/great_expectations/` directory.
- **Subject**: `{kind: "expectation", name: <expectation_type>, parent: <batch_id>}`.
- **Outcome mapping**:
  - `success: true` → `pass`
  - `success: false` and `meta.expected_to_fail` truthy → `xfail`
  - `success: false` → `fail`
  - `exception_info` populated → `error`
- **Metrics**: `total = result.element_count`, `failed = result.unexpected_count`, `observed = {observed_value: result.observed_value}`.
- **`supported_outcomes`**: `[pass, fail, error, xfail]`.
- **`subject_label`**: `"expectation"`.

### Future: pandera, pydantic

The wire protocol and `Plugin` trait are designed to accommodate
additional data-validation frameworks. Adding one requires a `mod.rs`
+ `plugin.rs` under the appropriate language directory and a runner
script. See the existing plugins for the pattern.

## Open design points

These are explicitly not decided by this spec; they need an answer
before implementation but reasonable people could pick differently.

1. **`Subject.kind` open vocabulary vs closed enum.** Spec uses
   freeform string. If the TUI ever wants per-kind icons, a closed enum
   would be cleaner : but locks the vocabulary, so new plugins need a
   core-crate change. 
    - Answer: stay open.
2. **`failures: Vec<FailureDetail>` schema hint.** Pandera failure cases
   are tabular. Should each `Event` carry `failure_columns: Vec<String>`
   so the TUI can render a table with stable column order? 
    - Answer: add it when the TUI table renderer lands, not before.
3. **`Subject.parent` granularity.** Pandera could go either
   `parent: "users"` (table) or `parent: "users.email"` (column). Spec
   currently leaves this to the plugin. *Recommendation*: document
   "deepest meaningful container" as the convention and let plugins
   interpret.
4. **`xfail` stats UX.** Should `scrutin stats` warn when an
   `xfail` test starts passing (the "xfail leak" case)? This is
   a real CI value-add but not part of this spec.
5. **Per-event `duration_ms` for data validation.** For a pandera schema
   with 100 columns, each `event` carries the time of *that column's
   check*, not the wall-clock time of the validation call. The runner
   has to either time each check individually (cheap; pandera supports
   it) or omit `duration_ms` and rely on the file-level
   `summary.duration_ms`.
6. **No `info` outcome.** Considered and rejected. The cases that seem
   to want a generic "informational" bucket are all better served by
   something the spec already provides:
   - "Column has 1234 distinct values" → `metrics.observed` on a `pass`
     event. The fact that the step ran is the pass; the observation
     rides along.
   - "Here are the failing rows in detail" → `failures: Vec<FailureDetail>`
     on a `fail`.
   - Arbitrary worker chatter ("collected 47 items", debug prints) →
     `LogBuffer`. It already exists, the TUI already renders it, and
     it doesn't pollute test history.
   - Run-level facts ("dataset version 4521", "schema hash abc123") →
     `RunMetadata.labels`.

   The sharpest counter-example is pointblank's `inform` action, which
   reads as "step ran, no action needed, here's what we observed." That
   maps cleanly to `pass` + `metrics.observed`; there's no need for a
   third outcome between "passed" and "warned." Concretely, an `info`
   bucket would have no meaningful TUI color, no JUnit projection (it's
   not a `<testcase>`), would defeat the "six outcomes, one source of
   truth" design, and would invite plugins to dump anything they don't
   feel like classifying. *Recommendation*: keep the taxonomy at six.

## Implementation status

1. ~~Spec types in `engine::protocol`.~~ **Done.**
2. ~~`tally_messages`, `FileTally`, all consumers (TUI, plain, JUnit, DB, stats) on the new shape.~~ **Done.**
3. ~~R and pytest runners emit the new wire format.~~ **Done.**
4. ~~pointblank and Great Expectations plugins.~~ **Done.**
5. `xfail`-leak warning in `scrutin stats`. **Not yet implemented.**
6. pandera and pydantic plugins. **Not yet implemented.**
