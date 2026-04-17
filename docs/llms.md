# LLMs

*Scrutin* is designed to work with LLM agents (Claude Code, Codex, Aider, Cursor, Continue, and anything else that can shell out) in both directions: you can ask an agent for help directly from a failing test, and agents can drive *Scrutin* non-interactively to run and interpret suites.

## Ask an agent about a failure

From a failing test, hand the failure off to a CLI agent in one keystroke.

**How to trigger:**

- Press `a` on a failing test in the Detail or Failure view.

**What Scrutin sends:** a Markdown prompt containing the outcome, error message, a windowed slice of the test source around the failing line, and (when a dep-map entry is known) a windowed slice of the production source under test. The prompt lands on disk in `$TMPDIR` so you can re-use it. Scrutin then launches the configured agent CLI in a terminal, cwd set to the project root.

**Where the terminal opens:**

- **Standalone TUI / browser**: a fresh OS terminal window. Scrutin auto-picks one (tmux if `$TMUX` is set, then `$TERM_PROGRAM`, then the OS default); override with `terminal = "..."` under `[agent]`.
- **Embedded in VS Code / Positron**: the editor's integrated terminal, inside the same window as the dashboard. No configuration required; Scrutin detects the webview host and forwards the script to the extension automatically.

**Configure in `.scrutin/config.toml`:**

```toml
[agent]
cli           = "claude"          # or "codex", "aider", "gemini", ...
context_lines = 20                # lines of source on each side of the failing line

# Optional: override terminal selection (standalone only). Placeholders
# {script} and {cwd} are substituted at launch time.
# terminal = "ghostty -e {script}"
# terminal = "tmux new-window -c {cwd} {script}"
```

All three fields are optional; with no `[agent]` block Scrutin uses `claude`, 20 lines of context, and an auto-detected terminal. The agent CLI must be on `$PATH`.

## Plain text output

The plain reporter (`-r plain`) is the recommended mode for agent consumption. It produces deterministic, colorless output (no ANSI escapes when stderr is not a tty), one line per file, with failure blocks that include `At: <path>:<line>` pointers an agent can open directly, and a final tally. Because the format is stable across runs, it can be pasted directly into a model's context window.

```bash
scrutin -r plain                       # full run, deterministic output
scrutin -r plain --set run.max_fail=1  # stop after the first failing file
scrutin -r list                        # enumerate test files without running them
scrutin -r junit:report.xml            # run + structured sidecar for programmatic parsing
```

The process exit code is the source of truth: `0` when every file passes, non-zero when any file fails. Agents should trust the exit code and not try to parse counts from plain text. For structured counts and per-test metadata, use `-r junit:report.xml`.

## Agent Skill

An agent "skill" is a markdown that guides an agent when using certain tools, or when accomplishing certain tasks. The canonical agent skill for *Scrutin* ships inside the binary.

=== "Claude Code (default location)"
    ```bash
    scrutin init skill
    ```
    Writes `~/.claude/skills/scrutin/SKILL.md`. Claude Code picks it up on next launch.

=== "Custom directory"
    ```bash
    scrutin init skill ./my-skills
    ```
    Writes `./my-skills/SKILL.md`. Useful for project-local skills (`.claude/skills/scrutin/`) or for editors that look elsewhere.

=== "Stdout (pipe to anything)"
    ```bash
    scrutin init skill -
    ```
    Prints the raw Markdown. Pipe into `pbcopy`, redirect to an arbitrary path, or embed in a larger prompt.

Add `--force` to overwrite an existing `SKILL.md`.

### Claude Code

Once `~/.claude/skills/scrutin/SKILL.md` is in place, Claude Code activates the skill automatically whenever a user message mentions tests, linting, watch mode, or refers to a project that contains `.scrutin/` or a *Scrutin*-relevant marker file. No per-project setup is required.

For project-wide activation (e.g. a team repo where every collaborator should get the skill), commit the file at `.claude/skills/scrutin/SKILL.md` in the project root:

```bash
scrutin init skill .claude/skills/scrutin/
git add .claude/skills/scrutin/SKILL.md
git commit -m "add scrutin Agent Skill"
```

Claude Code loads both user-level and project-level skills; the project-local one travels with the repo.

### Codex, Aider, and other agents

Agents that don't natively load `SKILL.md` files can still use the same content. Two common patterns:

1. **Paste into an `AGENTS.md` file** at the project root. Codex and many other agents read this file automatically. `scrutin init skill -` prints the Markdown suitable for inclusion.
2. **Add to the system prompt**. Most CLI agents accept a system-prompt file or flag; point it at the `SKILL.md` (or concatenate it with the rest of your prompt).

Either way, the instructions boil down to: call `scrutin -r plain` (or `-r junit:report.xml` for structured output), trust the exit code, and respect the project's `.scrutin/config.toml`.

## `llms.txt`

*Scrutin* publishes an [llms.txt](https://vincentarelbundock.github.io/scrutin/llms.txt) index at the documentation root following the [llmstxt.org](https://llmstxt.org) convention. Agents that crawl documentation can fetch it to land on the right pages (reporters, configuration, command-line reference, per-tool guides) without reading the whole site.

See the [Reporters](reporters/index.md) page for each reporter's full output and the [Configuration reference](reference/configuration.md) for every tunable key.

## What makes Scrutin LLM-friendly

- **Deterministic plain reporter** (`-r plain`): compact, colorless, one line per file, failure blocks with source pointers, final tally.
- **Structured output on demand**: `-r junit:report.xml` writes a machine-parseable JUnit XML sidecar; `-r github` emits GitHub Actions annotations; `-r list` enumerates files that would run without spawning any subprocess.
- **Exit code is the source of truth**: `0` when every file passes, non-zero when any file fails.
- **No config environment variables**: every persistent setting lives in `.scrutin/config.toml`. One-off overrides go through `--set key=value` (TOML-parsed). There are no hidden env vars to set or leak.
- **Preflight checks fail fast**: missing tool binaries, empty suite roots, and import errors produce a single actionable message before any run starts, instead of hundreds of per-file errors.
- **Shipped Agent Skill**: `scrutin init skill` writes a `SKILL.md` that teaches any compatible agent exactly when and how to invoke *Scrutin*.
- **`llms.txt` index**: served at [vincentarelbundock.github.io/scrutin/llms.txt](https://vincentarelbundock.github.io/scrutin/llms.txt) for agents that crawl the documentation.
- **CLAUDE.md in the repo**: contributors using Claude Code get architectural context automatically.


