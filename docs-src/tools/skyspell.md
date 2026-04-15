# skyspell

A dictionary-based spell checker *Scrutin* drives as a plugin. Like the linters (jarl, ruff) it maps diagnostics to `warn` events, so misspellings appear alongside test results in the TUI and web dashboard. skyspell is opt-in: enable it with an explicit `[[suite]] tool = "skyspell"` entry in `.scrutin/config.toml`, or pass files on the command line with `-t skyspell` in [file mode](../project-discovery.md#file-mode).

## Installing skyspell

skyspell is not shipped with *Scrutin*. Follow the upstream install instructions at [codeberg.org/your-tools/skyspell](https://codeberg.org/your-tools/skyspell).

## Directory structure

```
myproject/
├── .scrutin/
│   └── config.toml        # [[suite]] tool = "skyspell" + optional [skyspell] tuning
├── skyspell-ignore.toml   # auto-managed project whitelist (committed)
├── README.md
└── docs/
    └── guide.md
```

## Minimal example

**.scrutin/config.toml**

```toml
[[suite]]
tool = "skyspell"
```

Tuning (language, whitelist scope) lives under `[skyspell]` in the same file; see [Configuration](#configuration) below.

**README.md**

```markdown
# My project

This README intentionaly contains a mispeled word.
```

## Running

```bash
scrutin myproject              # TUI
scrutin -r plain myproject     # text output
```

skyspell runs as its own suite alongside any other suites you've declared; suites run sequentially (one at a time), but within the skyspell suite every matched file is checked in parallel. Default `run` patterns cover `**/*.md`, `**/*.markdown`, `**/*.txt`, `**/*.rst`, `**/*.qmd`, and `**/*.Rmd`.

## Fix flow (TUI)

In Detail view (Enter on a file with misspellings), each misspelling shows its suggestions inline below the message. Hotkeys:

| Key | Action |
|-----|--------|
| `1`..`9` | Accept the Nth ranked suggestion: *Scrutin* rewrites the file on disk and triggers a rerun so the warning disappears. |
| `0` | Whitelist the word: runs `skyspell add` with whatever `[skyspell].add_args` specifies. The default `--project` writes to a committable `skyspell-ignore.toml` at the project root, so your teammates inherit the whitelist. |
| `j`/`k` / `↑↓` | Move between misspellings. |

## Configuration

Tuning lives in `.scrutin/config.toml` under `[skyspell]`:

```toml
[skyspell]
# Args spliced between `skyspell` and every subcommand. Must include
# --lang (skyspell requires it). Default:
extra_args = ["--lang", "en_US"]

# Args appended to `skyspell add` (from the TUI's 0 key), before the
# word. Default --project scopes the whitelist to this project so
# skyspell-ignore.toml lives at the project root and can be committed.
# Set to [] to hit ~/.local/share/skyspell/global.toml instead.
add_args = ["--project"]
```

The minimal opt-in is `[[suite]] tool = "skyspell"`. Override the targets on the same block to scope the sweep:

```toml
[[suite]]
tool = "skyspell"
# default `run` covers markdown, plain text, and RMarkdown-ish files;
# override to sweep source-code comments or specific docs:
# run = ["README.md", "docs/**/*.md", "**/*.py"]
```
