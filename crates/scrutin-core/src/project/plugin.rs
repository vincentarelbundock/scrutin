//! Plugin infrastructure: one trait, several implementations (testthat,
//! tinytest, pytest). Plugins are compiled into the binary and registered
//! in a static list. On startup, [`detect_suites`] returns *every* plugin
//! whose `detect()` matches the project root: scrutin now happily runs
//! multiple tools side-by-side in the same directory.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};

use crate::engine::protocol::{Message, Outcome};

// Plugin implementations live under the per-language trees
// (`crate::r::*`, `crate::python::*`). The registry function
// `all_plugins()` below is the only place that knows about every language.

/// Command-mode execution spec. When [`Plugin::command_spec`] returns
/// `Some(…)`, the engine bypasses the long-lived worker protocol and runs
/// this command once per file, appending the file path as the last arg.
/// The plugin's [`Plugin::parse_command_output`] converts stdout into
/// [`Message`]s.
pub struct CommandSpec {
    /// Base argv (e.g. `["ruff", "check", "--output-format", "json"]`).
    /// The target file path is appended as the last argument at runtime.
    pub argv: Vec<String>,
}

pub trait Plugin: Send + Sync {
    /// Short human-readable name (e.g., "testthat", "pytest").
    fn name(&self) -> &'static str;

    /// Language identifier used as the top-level key under `[hooks.*]` in
    /// .scrutin/config.toml (e.g. "r", "python").
    fn language(&self) -> &'static str;

    /// Does this plugin apply to the given project root?
    fn detect(&self, root: &Path) -> bool;

    /// For language plugins that import the project under test at run
    /// time (e.g. pytest's `__import__("mypkg")` warm-up), return the
    /// importable module/package name. `None` means the plugin doesn't
    /// need a pre-flight import check (linters like ruff/jarl don't
    /// import anything).
    ///
    /// The engine uses this at startup to verify the venv / package is
    /// set up correctly *before* running any tests, so users see an
    /// actionable error ("install editable: pip install -e .") instead
    /// of a per-file ModuleNotFoundError.
    fn project_module_name(&self, _root: &Path) -> Option<String> {
        None
    }

    /// Subprocess command to spawn (argv-style). The first element is
    /// the binary. Worker-mode plugins must override; command-mode
    /// plugins (those that return `Some` from [`Plugin::command_spec`])
    /// are never consulted here.
    fn subprocess_cmd(&self, _root: &Path) -> Vec<String> {
        Vec::new()
    }

    /// Runner script contents (embedded via `include_str!`). Worker-
    /// mode plugins override; command-mode plugins inherit the empty
    /// default.
    fn runner_script(&self) -> &'static str {
        ""
    }

    /// File extension for the runner script (without the dot), e.g.
    /// `"R"` or `"py"`. Worker-mode plugins override.
    fn script_extension(&self) -> &'static str {
        ""
    }

    /// Basename of the runner script written under `.scrutin/`. Two plugins
    /// in the same project must use distinct basenames so they don't clobber
    /// each other's runner scripts. Default: `runner.<ext>`.
    fn runner_basename(&self) -> String {
        format!("runner.{}", self.script_extension())
    }

    /// Human name of the project/package at `root` (reads DESCRIPTION,
    /// pyproject.toml, etc.).
    fn project_name(&self, root: &Path) -> String;

    /// Version of the project/package under test. R: `Version:` field from
    /// `DESCRIPTION`. Python: `[project].version` from `pyproject.toml`.
    /// Returns `None` when no version is declared. This is metadata; the
    /// default is `None` so plugins that don't apply (linters with no
    /// natural project concept) just inherit it.
    fn project_version(&self, _root: &Path) -> Option<String> {
        None
    }

    /// Version of the testing / linting tool itself (e.g. the installed
    /// testthat or pytest package). Queried via a short subprocess call;
    /// any failure returns `None`. Plugins should make this cheap to call
    /// (the reporter caches the result once per run).
    fn tool_version(&self, _root: &Path) -> Option<String> {
        None
    }

    /// Default glob patterns for this plugin's `run` list — files the tool
    /// operates on (tests to execute, files to lint). Relative to the
    /// suite root. Used when the user's `[[suite]].run` is empty.
    fn default_run(&self) -> Vec<String>;

    /// Default glob patterns for this plugin's `watch` list — files watched
    /// for dep-map reruns. Relative to the suite root. Returning an empty
    /// vec means "watch the same files you run" (linter default).
    fn default_watch(&self) -> Vec<String>;

    /// Is this path a test file? (used by watcher to route changes)
    fn is_test_file(&self, path: &Path) -> bool;

    /// Is this path a source file? (used by watcher)
    fn is_source_file(&self, path: &Path) -> bool;

    /// Given a source file stem (e.g., "foo"), return candidate test filenames
    /// ("test-foo.R", "test_foo.py", etc.): used by the Tier-1 filename
    /// heuristic. Return an empty vec when the plugin has no meaningful
    /// stem→test mapping (linters, data-validators).
    fn test_file_candidates(&self, _source_stem: &str) -> Vec<String> {
        Vec::new()
    }

    /// Extra env vars to set on each subprocess.
    fn env_vars(&self, _root: &Path) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Filter a single stderr line from a worker subprocess. Return `true`
    /// to drop the line (e.g. R startup chatter, Python warning preludes).
    /// Default is to keep everything. Plugin-specific so the engine doesn't
    /// have to know which language emits which kind of noise.
    fn is_noise_line(&self, _line: &str) -> bool {
        false
    }

    /// Outcomes this plugin can emit. The TUI hides status filter chips
    /// for outcomes not in this set, and `scrutin stats` skips columns
    /// that would always be zero. Defaults to the four unit-test outcomes.
    fn supported_outcomes(&self) -> &'static [Outcome] {
        &[Outcome::Pass, Outcome::Fail, Outcome::Error, Outcome::Skip]
    }

    /// Short label for this plugin's notion of "subject", used in TUI
    /// detail panes and stats headers. Defaults to "test"; data validators
    /// override to "step", "check", "expectation", etc.
    fn subject_label(&self) -> &'static str {
        "test"
    }

    /// Plugin-specific actions exposed as keyboard shortcuts in the TUI
    /// and web UI. Each action runs a shell command on the selected file
    /// (path appended as the last argument). Default: no actions.
    fn actions(&self) -> Vec<PluginAction> {
        Vec::new()
    }

    /// If this plugin runs as a one-shot command rather than a long-lived
    /// NDJSON worker, return the command specification. The engine spawns
    /// the command once per file (no persistent subprocess). Default:
    /// `None` (worker mode).
    ///
    /// `pkg` gives the plugin access to Package-level config (e.g.
    /// `pkg.skyspell_extra_args`) so command-mode plugins can splice
    /// user-configurable flags into their argv.
    fn command_spec(
        &self,
        _root: &Path,
        _pkg: &crate::project::package::Package,
    ) -> Option<CommandSpec> {
        None
    }

    /// Parse the raw stdout of a command-mode invocation into protocol
    /// [`Message`]s. Only called when [`Self::command_spec`] returns
    /// `Some`. `file` is the basename of the test file.
    fn parse_command_output(
        &self,
        _file: &str,
        _stdout: &str,
        _stderr: &str,
        _exit_code: Option<i32>,
        _duration_ms: u64,
    ) -> Vec<Message> {
        vec![Message::Event(crate::engine::protocol::Event::engine_error(
            "<plugin>",
            "plugin does not implement parse_command_output",
        ))]
    }
}

/// Whether a plugin action targets a single file or all files in the suite.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ActionScope {
    /// Run the command on the currently selected file (path appended as last arg).
    #[default]
    File,
    /// Run the command on every file in the suite (after include/exclude filters).
    /// All matching file paths are appended as trailing arguments in a single
    /// invocation.
    All,
}

/// A plugin-defined action that can be triggered from the TUI or web UI.
/// The command is run with the target file's absolute path appended as the
/// last argument.
#[derive(Clone, Debug)]
pub struct PluginAction {
    /// Stable identifier (e.g. "fix", "fix_unsafe").
    pub name: &'static str,
    /// Keyboard shortcut character (e.g. 'f', 'F').
    pub key: char,
    /// Human-readable label for hints bar / buttons (e.g. "fix", "fix unsafe").
    pub label: &'static str,
    /// Base command (argv-style). The target file path is appended.
    pub command: Vec<String>,
    /// Re-run the file through the plugin after the action completes.
    pub rerun: bool,
    /// Whether this action targets a single file or all suite files.
    pub scope: ActionScope,
}

/// Every plugin compiled into the binary, in detection-priority order
/// (only matters when two plugins claim the same root, which the shipped
/// plugins never do). Each language module owns its own list; this
/// function flattens them.
pub fn all_plugins() -> Vec<Arc<dyn Plugin>> {
    let mut out = Vec::new();
    out.extend(crate::r::plugins());
    out.extend(crate::python::plugins());
    out.extend(crate::prose::plugins());
    out
}

/// Look up a compiled-in plugin by name. Returns `None` for unknown names.
pub fn plugin_by_name(name: &str) -> Option<Arc<dyn Plugin>> {
    all_plugins().into_iter().find(|p| p.name() == name)
}

/// Detect plugins that match `root`. When `tool_filter` is set
/// (and not "auto"), only that single tool is considered. Returns
/// an error when nothing matches.
pub fn detect_plugins(root: &Path, tool_filter: &str) -> Result<Vec<Arc<dyn Plugin>>> {
    let plugins = all_plugins();

    if tool_filter != "auto" {
        let p = plugins
            .iter()
            .find(|p| p.name() == tool_filter)
            .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", tool_filter))?;
        if !p.detect(root) {
            bail!(
                "Tool {:?} requested but its marker files were not found at {}",
                tool_filter,
                root.display()
            );
        }
        return Ok(vec![p.clone()]);
    }

    let matches: Vec<_> = plugins.iter().filter(|p| p.detect(root)).cloned().collect();
    if matches.is_empty() {
        bail!(
            "No test tools detected in {}. \
             Configure suites explicitly in .scrutin/config.toml or run scrutin from the project root.",
            root.display()
        );
    }
    Ok(matches)
}
