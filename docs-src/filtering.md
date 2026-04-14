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

Groups are selected at runtime via `--set filter.active_group=<name>` (or any other suitable `-s` override into `filter`). When `tools` is non-empty, the group applies only to suites whose tool matches.

## TUI filter

Press `/` to open the filter palette. The file list narrows as you type. Enter confirms, Esc cancels. The TUI filter is session-only and stacks with any `[filter]` rules from config.

`t` / `T` cycles the status filter (all → failures → errors → passes → ...). `p` / `P` cycles the suite filter when a project has more than one tool.

## Interaction with dependency tracking

Filtering is applied **after** dependency resolution:

1. The dep tracker computes affected test files from the change.
2. The include / exclude lists narrow that set.
3. Only the intersection runs.

## Glob dialect

A deliberately narrow shell-style glob, implemented in `scrutin_core::filter`:

- `*` matches any (possibly empty) run of characters
- `?` matches exactly one character
- any other character matches literally

There are no `**` recursive wildcards, no `[...]` character classes, and no escape sequences. Matching is anchored at both ends and runs against the **basename only** : patterns never see directory components. So `test-model*` matches `test-model-fits.R`; `*/snapshot/*` matches nothing because the filename has no `/` in it.
