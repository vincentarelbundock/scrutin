# Filter

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

Groups are activated at startup via the generic `-s/--set` override, by pinning `filter.group` to a name:

```bash
scrutin -s filter.group=fast
scrutin --set filter.group=py_integration
```

Or set it persistently in `.scrutin/config.toml`:

```toml
[filter]
group = "fast"
```

Selecting a group **replaces** the top-level `[filter]` include/exclude entirely, and applies the group's `tools` restrictor. The top-level lists apply only when no group is active. Unknown group names error out and list the known groups. If you want the union of two presets, define a third preset that spells out the union. `--set filter.include=[...]` remains available for one-off overrides that aren't worth naming.

## TUI and web filters

Press `/` to open the filter palette. The file list narrows as you type. Enter confirms, Esc cancels. The TUI filter is session-only and stacks with any `[filter]` rules from config.

`o` / `O` cycles the status filter (all → failures → errors → passes → running → ...). `t` / `T` cycles the tool filter when a project has more than one tool. `f` / `F` cycles the named filter group when at least one group is defined; the web frontend renders the same control as a dropdown next to the suite and status selects. Switching the active group on the fly re-applies its include/exclude/tools to the visible set; startup-style activation via `-s filter.group=NAME` just seeds this same control.

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
