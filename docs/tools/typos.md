# typos

A source-code spell checker that *Scrutin* drives as a plugin. Unlike a
dictionary-based checker (which treats every unfamiliar token as a
misspelling), [typos](https://github.com/crate-ci/typos) works from a
curated list of known-wrong-to-right corrections, so it produces
essentially zero false positives on identifiers, package names, and
other code tokens. That makes it a much better fit than skyspell for
scanning R / Python / Rust source files.

Like the linters (jarl, ruff), diagnostics map to `warn` events and
clean files produce a synthetic `pass`, so typo findings appear
alongside test results in the TUI and web dashboard. typos is opt-in:
enable it with an explicit `[[suite]] tool = "typos"` entry in
`.scrutin/config.toml`, or pass files on the command line with
`-t typos` in [file mode](../project-discovery.md#file-mode).

## Installing typos

typos is not shipped with *Scrutin*. Follow the upstream install instructions at [github.com/crate-ci/typos](https://github.com/crate-ci/typos).

## typos vs. skyspell

Both are spell checkers. They have opposite philosophies:

| | skyspell | typos |
|---|---|---|
| Model | system dictionary (enchant) | curated misspelling list |
| False positives on code | many (flags identifiers) | ~none by construction |
| Catches new misspellings | yes (anything not in the dictionary) | only ones typos has seen |
| Good for | prose: README, docs, Markdown | source code + prose |
| Output | JSON with ranked suggestions | NDJSON with a single correction |

Use skyspell for prose that needs a real dictionary check. Use typos
for source trees where you want to catch `recieve` / `teh` /
`occurence` without a flood of noise. They coexist: declare both as
`[[suite]]` entries and *Scrutin* runs them in the same invocation
(suites run one at a time; files within each suite run in parallel).

## Directory structure

```
myproject/
├── .scrutin/
│   └── config.toml        # [[suite]] tool = "typos"
├── _typos.toml            # optional: typos' own config (custom words, excludes)
├── README.md
└── R/
    └── utils.R
```

## Minimal example

**.scrutin/config.toml**

```toml
[[suite]]
tool = "typos"
```

**_typos.toml** (optional)

`_typos.toml` (or `typos.toml` / `.typos.toml`) is read by typos itself (not by *Scrutin*) for custom words, locale, and excludes:

```toml
[default.extend-words]
# Words that look like typos but aren't (project-specific):
arange = "arange"

[files]
extend-exclude = ["vendor/", "fixtures/"]
```

**R/utils.R**

```r
foo <- function() {
  # this comment has a recieve typo
  x <- "defintely broken"
  x
}
```

Running *Scrutin* flags `recieve` and `defintely` but ignores `foo`,
`function`, `x`, and every other R identifier.

## Running

```bash
scrutin myproject              # TUI
scrutin -r plain myproject     # text output
```

typos runs as its own suite alongside any other suites you've declared;
suites run one at a time, but within the typos suite every matched
file is checked in parallel. It uses command mode (calling
`typos --format json <file>` directly), so no subprocess protocol is
needed.

[File mode](../project-discovery.md#file-mode) works the same way without
any config file at all:

```bash
scrutin R/utils.R -t typos -r plain
scrutin R/*.R -t typos
```

## Fix flow

In Detail view, each typo shows its suggested correction inline as a numbered chip. Hotkeys:

| Key | Action |
|-----|--------|
| `1` | Accept the suggested correction: *Scrutin* rewrites the file on disk and triggers a rerun so the warning disappears. |
| `j`/`k` / `↑↓` | Move between typos. |

In the web dashboard, two additional chip buttons are exposed per file:

- **Typos: fix (this file)** runs `typos --write-changes <file>`.
- **Typos: fix all (suite)** runs `typos --write-changes` on every file in the suite, after include / exclude filters.

Affected files are re-checked automatically after any fix action.

## Configuration

The minimal suite entry is just `tool = "typos"`. typos' own configuration
(`_typos.toml`, `typos.toml`, or `.typos.toml` at the suite root)
controls custom words, exclusions, and locale; *Scrutin* doesn't
interpret it. See the
[typos config reference](https://github.com/crate-ci/typos/blob/master/docs/reference.md)
for the full schema.

To override *Scrutin*'s default suite targets in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "typos"
# Default `run` covers common source + prose extensions (md, R, py, rs,
# js, ts, go, ...). Override to scope:
# run = ["README.md", "R/**/*.R", "src/**/*.py"]
```

typos runs with `cwd = suite.root` so it picks up the local
`_typos.toml`. Each file is checked independently, with no dependency
tracking.
