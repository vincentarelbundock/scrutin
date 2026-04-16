# Frontends

Frontends are the interactive ways you watch a test run unfold: a terminal UI, a browser dashboard, and the editor integrations that embed that dashboard inside VS Code, Positron, and RStudio. All of them consume the same event stream from the engine and stream results in live, so you can browse and drill into failures while tests are still running.

Pick one with `-r` / `--reporter`:

```bash
scrutin                          # TUI (default when the terminal is a tty)
scrutin -r web                   # browser dashboard
scrutin -r web:0.0.0.0:3000      # web on a custom address
```

When no reporter is given, *Scrutin* defaults to `tui` on a tty and `plain` otherwise. The VS Code, Positron, and RStudio integrations are thin wrappers: each one spawns `scrutin -r web` in the background and embeds the resulting page inside the editor.

For non-interactive outputs (plain, JUnit, GitHub Actions, list), see [Reporters](../reporters.md).

## Available frontends

- [Terminal UI](terminal-ui.md): ratatui-based two-pane interface (default on a tty)
- [Web Dashboard](web.md): browser-based live dashboard served from the binary
- [VS Code](vscode.md): extension that embeds the web dashboard in an editor panel
- [Positron](positron.md): same extension as VS Code, installed via Open VSX
- [RStudio](rstudio.md): add-in that shows the dashboard in the Viewer pane
