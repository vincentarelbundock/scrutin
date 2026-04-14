# Test Filtering

Filtering controls which test files are included in a run. All glob patterns match against test file **basenames** (not full paths) and are evaluated before files are dispatched to a worker.

## .scrutin/config.toml

The base filters live in `[filter]`:

```toml
[filter]
include = ["test-model*", "test-plot*"]
exclude = ["test-slow*"]
```

- Empty `include` means "include everything".
- A file is kept iff it matches any include pattern (or `include` is empty) AND no exclude pattern. Exclude wins on ties.
- Override at invocation time with `-s/--set`:

```bash
scrutin --set 'filter.include=["test-model*"]'
scrutin -s 'filter.exclude=["test-slow*"]'
```

## Named Groups

Define reusable filter presets under `[filter.groups.<name>]`. Each group carries its own `include`, `exclude`, and an optional `tools` restrictor:

```toml
[filter.groups.fast]
include = ["test-unit*", "test-pure*"]
exclude = ["test-slow*"]

[filter.groups.py_integration]
tools   = ["pytest"]
include = ["test_integration_*"]
```

Groups are selected at runtime with `-g/--group`:

```bash
scrutin -g fast
scrutin -g fast,py_integration   # comma-separated (repeatable)
scrutin --group fast --group py_integration
```

Selecting any group **replaces** the top-level `[filter]` include/exclude/tools entirely; the top-level lists apply only when no `-g` is passed. Multiple groups union their `include`, `exclude`, and `tools` lists (so `-g group1,group2` runs the union of both). A group with an empty `tools` list lifts the tool restriction for the whole selection. Unknown group names error out and list the known groups. `--set filter.include=[...]` remains available for one-off overrides that aren't worth naming.

## TUI filter

Press `/` to open the filter palette. The file list narrows as you type. Enter confirms, Esc cancels. The TUI filter is session-only and stacks with any `[filter]` rules from config.

`t` / `T` cycles the status filter (all → failures → errors → passes → ...). `p` / `P` cycles the suite filter when a project has more than one tool.

## Interaction with dependency tracking

Filtering is applied **after** dependency resolution:

1. The dep tracker computes affected test files from the change.
2. The include / exclude lists narrow that set.
3. Only the intersection runs.

## Glob dialect

Filters use the `globset` matcher (same dialect as ripgrep / `.gitignore`):

- `*` matches any run of non-`/` characters
- `?` matches exactly one non-`/` character
- `[abc]` / `[!abc]` character classes
- `{foo,bar}` alternation
- `\` escapes a metacharacter

Matching is anchored at both ends and runs against the **basename only**, so `/` in a pattern has no effect (the filename has no `/` in it). `test-model*` matches `test-model-fits.R`; `test-{model,plot}*.R` matches either prefix; `test-pkg-[!m]*.R` matches everything except the `m`-prefixed pkg suites. Invalid patterns (e.g. unclosed `[`) are treated as no-match rather than erroring.
