//! "Send to LLM agent" handoff.
//!
//! Builds a markdown diagnosis prompt for a single failing test message
//! (outcome + error + windowed test source + windowed dep-mapped
//! production source), drops it on disk as a wrapper script, and spawns
//! that script inside a fresh terminal window so the user lands inside
//! an interactive `claude` (or `codex`, `aider`, ...) session pre-seeded
//! with everything needed to diagnose the failure.
//!
//! The two halves (prompt assembly + terminal launch) live together
//! here so both the TUI (direct in-process spawn) and the web server
//! (axum endpoint that returns 200 / error JSON) share one
//! implementation. Adding a new frontend = call [`diagnose`] with the
//! relevant context; nothing else.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use crate::engine::protocol::Outcome;
use crate::project::config::AgentConfig;

/// Everything an agent handoff needs to know about a single failing test
/// event. Borrowed because callers always have these values in hand
/// (file path, message, dep-map lookup) and we never mutate them.
pub struct DiagnoseRequest<'a> {
    /// Project root (used as `cwd` for the spawned agent so relative
    /// paths in the prompt resolve as the user expects).
    pub pkg_root: &'a Path,
    /// Test file path relative to `pkg_root`, for the prompt header.
    pub test_file_rel: &'a str,
    /// Absolute path used to read the source slice.
    pub test_file_abs: &'a Path,
    /// Optional dep-mapped production source file (absolute). When
    /// present, a windowed slice is included in the prompt so the agent
    /// sees the function under test, not just the assertion.
    pub source_file_abs: Option<&'a Path>,
    /// Line in `test_file_abs` where the failure was reported. Drives
    /// the source window. None → top of file.
    pub failing_line: Option<u32>,
    pub outcome: Outcome,
    pub test_name: Option<&'a str>,
    pub error_message: Option<&'a str>,
    pub config: &'a AgentConfig,
}

/// Telemetry returned from a successful spawn. Frontends echo this back
/// to the user (toast in web, status line in TUI) so they know which
/// terminal opened and where the prompt landed on disk.
pub struct LaunchInfo {
    pub script_path: PathBuf,
    pub prompt_path: PathBuf,
    /// First argv element of the resolved launch command (e.g.
    /// `"tmux"`, `"open"`, `"ghostty"`). Useful for "opened tmux
    /// window" messages without exposing the full command line.
    pub terminal: String,
}

/// Paths produced by [`prepare_handoff`]: everything a frontend needs
/// to run the agent in its own embedded terminal, without scrutin
/// spawning an OS window.
pub struct Handoff {
    pub script_path: PathBuf,
    pub prompt_path: PathBuf,
    pub cwd: PathBuf,
}

/// Assemble the prompt + wrapper script on disk, but don't spawn
/// anything. Used by editor extensions (VSCode, Positron) that run the
/// script in their own integrated terminal.
pub fn prepare_handoff(req: DiagnoseRequest) -> Result<Handoff> {
    let prompt = build_prompt(&req)?;
    let stamp = unique_stamp();

    let tmpdir = std::env::temp_dir();
    let prompt_path = tmpdir.join(format!("scrutin-diagnose-{stamp}.md"));
    let script_path = tmpdir.join(format!("scrutin-diagnose-{stamp}.sh"));

    std::fs::write(&prompt_path, &prompt)
        .with_context(|| format!("writing prompt to {}", prompt_path.display()))?;

    let script = build_wrapper_script(req.pkg_root, &req.config.cli, &prompt_path);
    write_executable(&script_path, &script)?;

    Ok(Handoff {
        script_path,
        prompt_path,
        cwd: req.pkg_root.to_path_buf(),
    })
}

/// Assemble the prompt + spawn a terminal running the configured agent
/// CLI. On success, the agent is now running in a separate window with
/// the prompt as its first user message; this function returns
/// immediately without waiting for the agent to exit.
pub fn diagnose(req: DiagnoseRequest) -> Result<LaunchInfo> {
    let terminal_template = req.config.terminal.clone();
    let handoff = prepare_handoff(req)?;

    let (program, args) =
        resolve_launcher(terminal_template.as_deref(), &handoff.script_path, &handoff.cwd);

    Command::new(&program)
        .args(&args)
        .spawn()
        .with_context(|| format!("spawning terminal `{program}` for agent handoff"))?;

    Ok(LaunchInfo {
        script_path: handoff.script_path,
        prompt_path: handoff.prompt_path,
        terminal: program,
    })
}

// ── Prompt assembly ─────────────────────────────────────────────────────────

fn build_prompt(req: &DiagnoseRequest) -> Result<String> {
    let mut out = String::with_capacity(2048);

    out.push_str("# Test failure diagnosis\n\n");
    out.push_str(&format!(
        "scrutin caught a `{}` outcome in `{}`",
        outcome_label(req.outcome),
        req.test_file_rel,
    ));
    if let Some(line) = req.failing_line {
        out.push_str(&format!(" at line {line}"));
    }
    out.push_str(". Please diagnose what's going wrong and propose a fix.\n\n");

    if let Some(name) = req.test_name.filter(|s| !s.is_empty()) {
        out.push_str(&format!("**Test name:** `{name}`\n\n"));
    }

    if let Some(msg) = req.error_message.filter(|s| !s.is_empty()) {
        out.push_str("## Error message\n\n```\n");
        out.push_str(msg.trim_end());
        out.push_str("\n```\n\n");
    }

    let ctx = req.config.context_lines;
    out.push_str(&format!("## Test source — `{}`\n\n", req.test_file_rel));
    let test_lang = lang_for(req.test_file_abs);
    let test_slice = read_window(req.test_file_abs, req.failing_line, ctx)?;
    fenced(&mut out, test_lang, &test_slice);

    if let Some(src) = req.source_file_abs {
        let rel = src
            .strip_prefix(req.pkg_root)
            .unwrap_or(src)
            .display()
            .to_string();
        out.push_str(&format!("\n## Source under test — `{rel}`\n\n"));
        let src_lang = lang_for(src);
        // Without a dep-map line annotation, show the top of the file
        // (`read_window(_, None, _)` caps at start). Future improvement:
        // walk the dep-map back to the specific function definition.
        let src_slice = read_window(src, None, ctx.saturating_mul(2))?;
        fenced(&mut out, src_lang, &src_slice);
    }

    out.push_str(
        "\n## What I'd like you to do\n\n\
         1. Reproduce the failure in your head: read the test, then the source.\n\
         2. Identify the root cause (be specific: which line, which condition).\n\
         3. Propose the smallest change that makes the test pass without weakening it.\n\
         4. If the test itself looks wrong, say so explicitly and explain why.\n",
    );

    Ok(out)
}

/// Read a `±context`-line slice around `line` (1-based). When `line` is
/// `None`, returns the first `2 * context + 1` lines. Caps every read
/// at the file's natural bounds.
fn read_window(path: &Path, line: Option<u32>, context: u32) -> Result<WindowSlice> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let (start, end) = match line {
        Some(l) if l > 0 => {
            let target = (l as usize).saturating_sub(1).min(total.saturating_sub(1));
            let ctx = context as usize;
            (target.saturating_sub(ctx), (target + ctx + 1).min(total))
        }
        _ => (0, total.min((context as usize).saturating_mul(2).saturating_add(1))),
    };
    let body = lines[start..end].join("\n");
    Ok(WindowSlice {
        body,
        start_line: start + 1,
    })
}

struct WindowSlice {
    body: String,
    start_line: usize,
}

/// Emit a fenced code block with a leading `Lstart` comment so the
/// agent can map line numbers back to the original file. The comment
/// uses the language's own syntax so it doesn't break highlighting.
fn fenced(out: &mut String, lang: &str, slice: &WindowSlice) {
    out.push_str("```");
    out.push_str(lang);
    out.push('\n');
    let prefix = match lang {
        "py" | "python" | "r" | "R" | "sh" | "bash" => "#",
        _ => "//",
    };
    out.push_str(&format!("{prefix} starting at line {}\n", slice.start_line));
    out.push_str(&slice.body);
    if !slice.body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n");
}

fn lang_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "py" => "python",
        "R" | "r" => "r",
        "sh" => "bash",
        "rs" => "rust",
        "js" | "mjs" => "javascript",
        "ts" => "typescript",
        "md" => "markdown",
        "toml" => "toml",
        // Fall back to plain (no language hint) for unknown extensions.
        // The fenced block still renders; just no syntax coloring.
        _ => "",
    }
}

fn outcome_label(o: Outcome) -> &'static str {
    match o {
        Outcome::Pass => "pass",
        Outcome::Fail => "fail",
        Outcome::Error => "error",
        Outcome::Skip => "skip",
        Outcome::Xfail => "xfail",
        Outcome::Warn => "warn",
    }
}

// ── Terminal launcher ───────────────────────────────────────────────────────

/// POSIX shell wrapper that lands the user inside the agent CLI in the
/// project root. The trailing `read` keeps the window open after the
/// agent exits so error messages don't disappear with the shell.
fn build_wrapper_script(cwd: &Path, cli: &str, prompt_path: &Path) -> String {
    format!(
        "#!/bin/sh\n\
         set -e\n\
         cd {cwd}\n\
         {cli} \"$(cat {prompt})\"\n\
         status=$?\n\
         echo\n\
         echo \"[scrutin] agent exited with $status. press enter to close.\"\n\
         read _\n",
        cwd = sh_quote(cwd.to_string_lossy().as_ref()),
        cli = cli,
        prompt = sh_quote(prompt_path.to_string_lossy().as_ref()),
    )
}

fn write_executable(path: &Path, body: &str) -> Result<()> {
    let mut f = std::fs::File::create(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(body.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("chmod +x {}", path.display()))?;
    }
    Ok(())
}

/// Resolve the terminal launch command into `(program, args)`. Order:
///   1. `config.terminal` template (with `{script}` and `{cwd}` placeholders).
///   2. `$TMUX` set → `tmux new-window -c <cwd> <script>`.
///   3. `$TERM_PROGRAM` matches a known terminal → that terminal's syntax.
///   4. macOS fallback: `open -a Terminal <script>`.
///   5. Linux fallback: walk `$TERMINAL`, `x-terminal-emulator`, then a
///      list of common terminals; the first one on `$PATH` wins.
fn resolve_launcher(
    template: Option<&str>,
    script: &Path,
    cwd: &Path,
) -> (String, Vec<String>) {
    if let Some(tpl) = template {
        let expanded = tpl
            .replace("{script}", &script.display().to_string())
            .replace("{cwd}", &cwd.display().to_string());
        let mut parts = expanded.split_whitespace();
        let program = parts.next().unwrap_or("").to_string();
        let args = parts.map(String::from).collect();
        return (program, args);
    }

    // tmux: lands the new pane in cwd and runs the wrapper there.
    if std::env::var_os("TMUX").is_some() && which("tmux") {
        return (
            "tmux".to_string(),
            vec![
                "new-window".into(),
                "-c".into(),
                cwd.display().to_string(),
                script.display().to_string(),
            ],
        );
    }

    if let Ok(term) = std::env::var("TERM_PROGRAM") {
        if let Some(launch) = launcher_for_term_program(&term, script) {
            return launch;
        }
    }

    if cfg!(target_os = "macos") {
        return (
            "open".to_string(),
            vec!["-a".into(), "Terminal".into(), script.display().to_string()],
        );
    }

    // Linux best effort.
    if let Ok(t) = std::env::var("TERMINAL") {
        if which(&t) {
            return (t, vec!["-e".into(), script.display().to_string()]);
        }
    }
    let candidates: &[(&str, &[&str])] = &[
        ("x-terminal-emulator", &["-e"]),
        ("ghostty", &["-e"]),
        ("alacritty", &["-e"]),
        ("kitty", &[]),
        ("wezterm", &["start", "--"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("tilix", &["-e"]),
        ("xterm", &["-e"]),
    ];
    for (cmd, prefix) in candidates {
        if which(cmd) {
            let mut args: Vec<String> = prefix.iter().map(|s| s.to_string()).collect();
            args.push(script.display().to_string());
            return (cmd.to_string(), args);
        }
    }

    // Last resort: just run the script in the current process group.
    // The agent will attach to whatever stdio scrutin already owns,
    // which is wrong for the TUI but at least doesn't silently fail.
    (script.display().to_string(), Vec::new())
}

fn launcher_for_term_program(term: &str, script: &Path) -> Option<(String, Vec<String>)> {
    let s = script.display().to_string();
    match term {
        "ghostty" if which("ghostty") => Some(("ghostty".into(), vec!["-e".into(), s])),
        "iTerm.app" => Some(("open".into(), vec!["-a".into(), "iTerm".into(), s])),
        "Apple_Terminal" => Some(("open".into(), vec!["-a".into(), "Terminal".into(), s])),
        "WezTerm" if which("wezterm") => {
            Some(("wezterm".into(), vec!["start".into(), "--".into(), s]))
        }
        "kitty" if which("kitty") => Some(("kitty".into(), vec![s])),
        "alacritty" if which("alacritty") => {
            Some(("alacritty".into(), vec!["-e".into(), s]))
        }
        _ => None,
    }
}

fn which(cmd: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        if dir.join(cmd).is_file() {
            return true;
        }
        // Some terminals install as `.app` bundles on macOS; handled by
        // the `open -a` paths above, not this PATH walk.
    }
    false
}

fn sh_quote(s: &str) -> String {
    // Single-quote everything; replace embedded `'` with `'\''`. POSIX-safe
    // and works for any path the user can put on disk.
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn unique_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("{pid}-{nanos}")
}

// Ensure callers can fail hard on outcomes we'd never diagnose (e.g.
// pass). Callers that want to still send a passing test as a "review
// this" prompt can bypass and call `build_prompt` + the writer halves
// directly; for now we keep the public API restricted.
#[allow(dead_code)]
fn ensure_diagnosable(o: Outcome) -> Result<()> {
    match o {
        Outcome::Pass | Outcome::Skip | Outcome::Xfail => {
            Err(anyhow!("nothing to diagnose for outcome `{}`", outcome_label(o)))
        }
        _ => Ok(()),
    }
}
