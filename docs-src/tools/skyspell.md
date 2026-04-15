# skyspell

A source-code-aware spell checker scrutin drives as a plugin. Like the linters (jarl, ruff) it maps diagnostics to `warn` events, so misspellings appear alongside test results in the TUI and web dashboard. Scrutin auto-detects skyspell when a `skyspell.toml` file is present at the project root.

## Installing skyspell

skyspell is not shipped with scrutin. Install it separately:

- **Linux**: `cargo install skyspell`. Needs the Enchant C library with headers plus one backend (aspell, hunspell, or nuspell) and a matching dictionary.
- **macOS**: `brew install enchant hunspell` to pull the native dependencies, then `cargo install skyspell`.
- **Windows**: prebuilt installer in the GitHub Releases section of the upstream repo.

Upstream lives at [codeberg.org/your-tools/skyspell](https://codeberg.org/your-tools/skyspell) (the GitHub mirror was archived in early 2026).

Verify the install with `skyspell --lang en_US suggest helllo`; it should print candidates including `hello`.

## Directory structure

```
myproject/
├── .scrutin/
│   └── config.toml        # optional [skyspell] tuning
├── skyspell.toml          # opt-in marker (contents ignored)
├── skyspell-ignore.toml   # auto-managed project whitelist
├── README.md
└── docs/
    └── guide.md
```

## Minimal example

**skyspell.toml**

An empty file is enough to opt in. Put configuration in `.scrutin/config.toml` under `[skyspell]` instead.

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

skyspell runs concurrently alongside any test tools in the same project. Default `run` patterns cover `**/*.md`, `**/*.markdown`, `**/*.txt`, `**/*.rst`, `**/*.qmd`, and `**/*.Rmd`.

## Fix flow (TUI)

In Detail view (Enter on a file with misspellings), each misspelling shows its suggestions inline below the message. Two classes of hotkey:

| Key | Action |
|-----|--------|
| `1`..`9` | Accept the Nth ranked suggestion: scrutin rewrites the file on disk and triggers a rerun so the warning disappears. |
| `0` | Whitelist the word: runs `skyspell add` with whatever `[skyspell].add_args` specifies. The default `--project` writes to `skyspell-ignore.toml` next to your marker (commitable), so your teammates inherit the whitelist. |
| `j`/`k` / `↑↓` | Move between misspellings. |

## Configuration

All tuning lives in `.scrutin/config.toml` under `[skyspell]`, mirroring `[pytest]`:

```toml
[skyspell]
# Args spliced between `skyspell` and every subcommand. Must include
# --lang (skyspell requires it). Default:
extra_args = ["--lang", "en_US"]

# Args appended to `skyspell add` (from the TUI's 0 key), before the
# word. Default --project scopes the whitelist to this project so
# skyspell-ignore.toml lives next to the marker and can be committed.
# Set to [] to hit ~/.local/share/skyspell/global.toml instead.
add_args = ["--project"]
```

To change the lint targets, declare a suite:

```toml
[[suite]]
tool = "skyspell"
# default `run` covers markdown, plain text, and RMarkdown-ish files;
# override to sweep source-code comments:
# run = ["README.md", "docs/**/*.md", "**/*.py"]
```
