# LLMs

*Scrutin* is designed to be driven from an LLM agent (Claude Code, Codex, Aider, Cursor, Continue, and anything else that can shell out). This page collects the features that make it agent-friendly, how to install the first-party Agent Skill, and recommended patterns for calling *Scrutin* from a non-interactive model.

## What makes it LLM-friendly

- **Deterministic plain reporter** (`-r plain`): compact, colorless (no ANSI when stderr is not a tty), one line per file, a failure block with `At: <path>:<line>` pointers, and a final tally. Suitable for direct inclusion in a model's response.
- **Structured output on demand**: `-r junit:report.xml` writes a machine-parseable JUnit XML sidecar; `-r github` emits GitHub Actions annotations; `-r list` enumerates files that would run without spawning any subprocess.
- **Exit code is the source of truth**: `0` when every file passes, non-zero when any file fails. Agents should trust the exit code and not try to parse counts from plain text.
- **No config environment variables**: every persistent setting lives in `.scrutin/config.toml`. One-off overrides go through `--set key=value` (TOML-parsed). There are no hidden env vars to set or leak.
- **Preflight checks fail fast**: missing tool binaries, empty suite roots, and import errors produce a single actionable message before any run starts, instead of hundreds of per-file errors.
- **Shipped Agent Skill**: `scrutin init skill` writes a `SKILL.md` that teaches any compatible agent exactly when and how to invoke *Scrutin*.
- **`llms.txt` index**: served at [vincentarelbundock.github.io/scrutin/llms.txt](https://vincentarelbundock.github.io/scrutin/llms.txt) for agents that crawl the documentation.
- **CLAUDE.md in the repo**: contributors using Claude Code get architectural context automatically.

## Install the Agent Skill

The canonical skill ships inside the *Scrutin* binary.

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

## Claude Code

Once `~/.claude/skills/scrutin/SKILL.md` is in place, Claude Code activates the skill automatically whenever a user message mentions tests, linting, watch mode, or refers to a project that contains `.scrutin/` or a *Scrutin*-relevant marker file. No per-project setup is required.

For project-wide activation (e.g. a team repo where every collaborator should get the skill), commit the file at `.claude/skills/scrutin/SKILL.md` in the project root:

```bash
scrutin init skill .claude/skills/scrutin/
git add .claude/skills/scrutin/SKILL.md
git commit -m "add scrutin Agent Skill"
```

Claude Code loads both user-level and project-level skills; the project-local one travels with the repo.

## Codex, Aider, and other agents

Agents that don't natively load `SKILL.md` files can still use the same content. Two common patterns:

1. **Paste into an `AGENTS.md` file** at the project root. Codex and many other agents read this file automatically. `scrutin init skill -` prints the Markdown suitable for inclusion.
2. **Add to the system prompt**. Most CLI agents accept a system-prompt file or flag; point it at the `SKILL.md` (or concatenate it with the rest of your prompt).

Either way, the instructions boil down to: call `scrutin -r plain` (or `-r junit:report.xml` for structured output), trust the exit code, and respect the project's `.scrutin/config.toml`.

## `llms.txt`

*Scrutin* publishes an [llms.txt](https://vincentarelbundock.github.io/scrutin/llms.txt) index at the documentation root following the [llmstxt.org](https://llmstxt.org) convention. Agents that crawl documentation can fetch it to land on the right pages (reporters, configuration, command-line reference, per-tool guides) without reading the whole site.

## Agent best practices

When calling *Scrutin* from a non-interactive model, prefer these patterns:

```bash
scrutin -r plain                       # full run, deterministic output
scrutin -r plain --set run.max_fail=1  # stop after first failing file
scrutin -r list                        # enumerate without running
scrutin -r junit:report.xml            # run + structured sidecar
```

And avoid:

- Launching `scrutin` with no `-r` flag: defaults to an interactive TUI that will hang without a human at the keyboard.
- Enabling watch mode from an agent: watch is only useful with a live frontend (TUI or web dashboard).
- Parsing tallies from plain text: use the exit code, or `-r junit:report.xml` for counts.
- Guessing configuration env vars: there are none. Use `--set key=value` or edit `.scrutin/config.toml`.

See the [Reporters](reporters/index.md) page for each reporter's full output and the [Configuration reference](reference/configuration.md) for every tunable key.
