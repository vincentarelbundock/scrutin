use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct Config {
    pub run: RunConfig,
    pub watch: WatchConfig,
    pub filter: FilterConfig,
    /// Explicit suite declarations. When non-empty, auto-detection is
    /// skipped entirely: the user controls exactly which tools run
    /// and where their files live. When empty, scrutin falls back to
    /// auto-detection against the project root.
    #[serde(default, rename = "suite")]
    pub suites: Vec<SuiteConfig>,
    pub python: PythonConfig,
    pub testthat: TestthatConfig,
    pub tinytest: TinytestConfig,
    pub pytest: PytestConfig,
    pub web: WebConfig,
    pub hooks: HooksConfig,
    pub metadata: MetadataConfig,
    pub preflight: PreflightConfig,
    /// User-supplied key/value labels attached to every run. Populated from
    /// `[extras]` in `.scrutin/config.toml` and `--set extras.KEY=VALUE` on the CLI.
    /// Values may be any TOML scalar (string, int, float, bool) and are
    /// coerced to strings at the storage/reporter boundary.
    #[serde(default, deserialize_with = "deserialize_extra_map")]
    pub extras: BTreeMap<String, String>,
    /// Environment variables injected into every subprocess that runs user
    /// code (test workers + `--build-depmap` Rscript). Keys are case-sensitive
    /// in storage but validated case-insensitively at parse time so configs
    /// stay portable across Linux/macOS and Windows (where `Path` and `PATH`
    /// are the same key). Empty values are valid (set the var to empty);
    /// no interpolation, no unset semantics. `[env]` always wins over
    /// inherited parent env on conflict.
    pub env: BTreeMap<String, String>,
    /// Per-mode keymap overrides. Each subtable (e.g. `[keymap.normal]`)
    /// fully replaces the default bindings for that mode (replace
    /// semantics, not overlay). Modes absent from this table keep their
    /// built-in defaults. Mode names: `normal`, `detail`, `failure`,
    /// `help`, `log`. Action names live in `scrutin-tui::keymap::Action`.
    pub keymap: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

/// Process- and worker-level hook scripts. Process hooks
/// (`startup` / `teardown`) run once per scrutin invocation from the Rust
/// binary. Worker hooks live under a nested `[hooks.<language>.<tool>]`
/// table (or a language-level fallback `[hooks.<language>]`) and are
/// sourced by each worker subprocess on boot / shutdown.
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct HooksConfig {
    /// Path to an executable script run once before the first worker spawns.
    /// Relative to project root. Must be executable with a shebang.
    /// Failure aborts the run with the script's exit code.
    pub startup: Option<PathBuf>,
    /// Path to an executable script run once after the last result drains.
    /// Failure logs a warning but does not mask the test exit code.
    pub teardown: Option<PathBuf>,
    /// Per-language worker hooks (e.g. `[hooks.python]`, `[hooks.r]`).
    /// Each may contain language-level fields and/or per-tool
    /// nested tables like `[hooks.python.pytest]`.
    #[serde(flatten)]
    pub by_language: std::collections::HashMap<String, LanguageHooks>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct LanguageHooks {
    /// Language-level worker hooks (apply to any tool in this language
    /// unless overridden by a more specific tool-level entry).
    pub worker_startup: Option<PathBuf>,
    pub worker_teardown: Option<PathBuf>,
    /// Per-tool override map, e.g. `[hooks.python.pytest]`.
    #[serde(flatten)]
    pub by_tool: std::collections::HashMap<String, ToolHooks>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct ToolHooks {
    pub worker_startup: Option<PathBuf>,
    pub worker_teardown: Option<PathBuf>,
}

/// Python-level config (applies to all Python tools: pytest,
/// great_expectations, etc.).
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PythonConfig {
    /// Override the Python interpreter command. When set, this replaces
    /// the auto-detected interpreter entirely. Split on whitespace so
    /// wrappers like `uv run python` work. When unset, the `venv` field
    /// and then the standard auto-detection chain apply.
    pub interpreter: Option<String>,
    /// Path to a virtualenv directory (relative to project root or
    /// absolute). When set, scrutin uses `<venv>/bin/python` (or
    /// `<venv>/Scripts/python.exe` on Windows) instead of auto-detecting.
    /// Ignored when `interpreter` is set.
    pub venv: Option<PathBuf>,
}

/// Startup pre-flight checks. These run *before* any test worker
/// spawns and surface common setup mistakes (missing package install,
/// missing CLI tool, typo in `[[suite]] root`, empty `run` glob list,
/// missing R `pkgload`) as one clean actionable error instead of
/// per-file noise mid-run.
///
/// Disable with `[preflight] enabled = false` if a check is wrong for
/// your project.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PreflightConfig {
    /// Master switch. When false, no pre-flight checks run.
    pub enabled: bool,
    /// Verify each suite's `root` resolves to an existing directory.
    pub suite_roots: bool,
    /// Verify each suite's `run` globs match at least one file.
    pub run_globs: bool,
    /// Verify command-mode plugins (jarl, ruff) have their CLI tool on PATH.
    pub command_tools: bool,
    /// Verify Python suites' project module imports cleanly via the
    /// resolved interpreter (catches missing `pip install -e .`).
    pub python_imports: bool,
    /// Verify R suites have `pkgload` installed (the default runner
    /// requires it for `load_all`).
    pub r_pkgload: bool,
}

impl Default for PreflightConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            suite_roots: true,
            run_globs: true,
            command_tools: true,
            python_imports: true,
            r_pkgload: true,
        }
    }
}

impl PythonConfig {
    /// Resolve the interpreter override to an argv prefix. Returns an empty
    /// vec when no override is configured (auto-detection applies).
    /// `root` is needed to resolve a relative `venv` path.
    pub fn resolve_interpreter(&self, root: &Path) -> Vec<String> {
        if let Some(ref interp) = self.interpreter {
            return interp.split_whitespace().map(String::from).collect();
        }
        if let Some(ref venv) = self.venv {
            let found = crate::python::py_find_python(root, Some(venv.as_path()));
            return vec![found];
        }
        Vec::new()
    }
}

/// Testthat-specific config.
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct TestthatConfig {
    /// Path to a custom runner script. When set, scrutin uses this file
    /// instead of the built-in runner. Relative to the project root.
    pub runner: Option<PathBuf>,
}

/// Tinytest-specific config.
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct TinytestConfig {
    /// Path to a custom runner script. See `[testthat].runner`.
    pub runner: Option<PathBuf>,
}

/// Pytest-specific escape hatches. `extra_args` is appended verbatim to
/// every `pytest.main()` invocation in the runner subprocess, letting users
/// reach for obscure pytest flags without scrutin growing a CLI option for
/// each one.
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PytestConfig {
    /// Path to a custom runner script. See `[testthat].runner`.
    pub runner: Option<PathBuf>,
    pub extra_args: Vec<String>,
}

/// Web-frontend-specific config. Only consulted by `scrutin-web`; the
/// TUI/plain reporters don't use it.
#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct WebConfig {
    /// Explicit editor command for the "open in editor" action from the
    /// browser dashboard. When unset, scrutin falls back to `$VISUAL` /
    /// `$EDITOR` (skipping known terminal editors like vim/nano, which
    /// cannot be launched detached from an HTTP handler) and finally the
    /// OS-native `open` / `xdg-open` / `start`. Split on whitespace so
    /// wrappers like `"code --wait"` work.
    pub editor: Option<String>,
}

/// Explicit declaration of a single test suite. Used in `[[suite]]` array
/// of tables in .scrutin/config.toml. When at least one suite is declared,
/// auto-detection is skipped.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteConfig {
    /// Tool name: "testthat", "tinytest", "pointblank", "jarl",
    /// "pytest", "great_expectations", "ruff", or "validate".
    pub tool: String,
    /// Suite root: the directory this suite's tool runs from. Drives the
    /// subprocess CWD and the `SCRUTIN_PKG_DIR` env var. Relative paths
    /// are joined with the project root; absolute paths are taken verbatim.
    /// Default: `.` (the project root).
    #[serde(default = "default_suite_root")]
    pub root: PathBuf,
    /// Glob patterns for files the tool operates on (tests to execute,
    /// files to lint). Relative to `root` unless absolute. Empty = plugin
    /// defaults.
    #[serde(default)]
    pub run: Vec<String>,
    /// Glob patterns for files watched to trigger reruns. Relative to
    /// `root`. Empty = plugin default (which for linters equals `run`).
    #[serde(default)]
    pub watch: Vec<String>,
    /// Custom runner script, relative to the project root. When set,
    /// the engine reads this file instead of the embedded default.
    #[serde(default)]
    pub runner: Option<PathBuf>,
}

fn default_suite_root() -> PathBuf {
    PathBuf::from(".")
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct RunConfig {
    /// Restrict auto-detection to a single tool (e.g. "testthat",
    /// "pytest"). Ignored when explicit `[[suite]]` entries are configured.
    /// Default "auto" detects all matching tools.
    pub tool: String,
    pub workers: Option<usize>,
    /// Fork-based isolation (Linux/macOS only, default false). When true,
    /// each worker loads the project once then fork()s per test file. Each
    /// child gets a COW copy of the warm interpreter state, providing both
    /// fast startup and full process isolation. When false (the default),
    /// workers are killed and respawned after every file: slower but safe.
    /// Automatically disabled on Windows where fork() is unavailable.
    ///
    /// Dangerous: if a test (or any package it loads) itself spawns forked
    /// workers (e.g. R's `parallel::mclapply`, Python's `multiprocessing`
    /// with the `fork` start method), forking an already-multithreaded
    /// parent can deadlock or crash the child. Only enable when you are
    /// confident no code under test forks on its own.
    pub fork_workers: bool,
    /// Stop after this many failing files (failed + errored). 0 = unlimited.
    pub max_fail: u32,
    /// Colored output in plain mode. Default true.
    pub color: bool,
    /// Randomize test file execution order to surface inter-file state leaks.
    pub shuffle: bool,
    /// Seed for shuffle. If `None` and shuffle is enabled, a fresh seed is
    /// drawn from system time and *always printed* so the run is reproducible.
    pub seed: Option<u64>,
    /// Per-file timeout in milliseconds. Applies to every test worker
    /// subprocess; a worker that doesn't return in this many ms is killed
    /// and the file marked errored. 0 (default) disables the per-file
    /// timeout.
    pub timeout_file_ms: u64,
    /// Whole-run timeout in milliseconds. If the entire run (across all
    /// suites) hasn't finished within this budget, all in-flight workers are
    /// cancelled. 0 (default) disables the run timeout.
    pub timeout_run_ms: u64,
    /// Re-execute failing files up to this many extra times before
    /// reporting failure. 0 (default) disables reruns. A file that passes
    /// on attempt > 1 is marked flaky in the run database, in JUnit, and
    /// in the plain-mode summary.
    pub reruns: u32,
    /// Milliseconds to wait between rerun attempts. Default 0 (immediate).
    /// Useful for flakes caused by external rate limits or eventual
    /// consistency in fixtures.
    pub reruns_delay: u64,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            tool: "auto".to_string(),
            workers: None,
            fork_workers: false,
            max_fail: 0,
            color: true,
            shuffle: false,
            seed: None,
            timeout_file_ms: 0,
            timeout_run_ms: 0,
            reruns: 0,
            reruns_delay: 0,
        }
    }
}


/// `[metadata]` : controls run-provenance capture.
/// Provenance (git commit, hostname, etc.) is captured automatically when
/// `enabled = true` (the default); set `enabled = false` to opt out for
/// privacy or air-gapped builds. User labels live in the top-level
/// `[extras]` section (not here).
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct MetadataConfig {
    pub enabled: bool,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}


#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct WatchConfig {
    /// Whether watch mode is enabled by default in the TUI and web.
    /// Override via `--set watch.enabled=true|false`. Plain / GitHub / JUnit
    /// reporters are always one-shot regardless. Default `true`.
    pub enabled: bool,
    pub debounce_ms: u64,
    pub ignore: Vec<String>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            debounce_ms: 50,
            ignore: vec![".git".to_string(), "*.Rhistory".to_string()],
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct FilterConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    #[serde(default)]
    pub groups: std::collections::HashMap<String, FilterGroup>,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct FilterGroup {
    /// Restrict this group to specific tools (by name, e.g. "testthat",
    /// "pytest"). When empty, the group applies to all active suites.
    pub tools: Vec<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Config {
    /// Look up the custom runner script path for a given tool name.
    /// Returns `None` when the user has not set one (use the built-in default).
    pub fn runner_override(&self, tool: &str) -> Option<&Path> {
        match tool {
            "testthat" => self.testthat.runner.as_deref(),
            "tinytest" => self.tinytest.runner.as_deref(),
            "pytest" => self.pytest.runner.as_deref(),
            _ => None,
        }
    }

    /// Load config from `root/.scrutin/config.toml`, falling back to user config.
    pub fn load(root: &Path) -> Result<Self> {
        let candidate = root.join(".scrutin").join("config.toml");
        if candidate.is_file() {
            let contents = std::fs::read_to_string(&candidate)?;
            let config: Config = toml::from_str(&contents)?;
            config.validate_env()?;
            return Ok(config);
        }

        // Fallback to user-level config
        if let Some(config_dir) = dirs::config_dir() {
            let user_config = config_dir.join("scrutin").join("config.toml");
            if user_config.is_file() {
                let contents = std::fs::read_to_string(&user_config)?;
                let config: Config = toml::from_str(&contents)?;
                config.validate_env()?;
                return Ok(config);
            }
        }

        Ok(Config::default())
    }

    /// Validate `[env]` keys: reject invalid characters (which would break
    /// `Command::env` at spawn time) and case-insensitive duplicates (which
    /// would silently collapse on Windows). Values are not validated — empty
    /// strings are legal, and any byte sequence is in principle a valid env
    /// value on Unix. Called from `load`.
    fn validate_env(&self) -> Result<()> {
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for key in self.env.keys() {
            if key.is_empty() {
                bail!("invalid [env] key: empty string");
            }
            if key.contains('=') || key.contains('\0') {
                bail!("invalid [env] key {:?}: contains '=' or NUL", key);
            }
            if key.chars().next().is_some_and(char::is_whitespace)
                || key.chars().last().is_some_and(char::is_whitespace)
            {
                bail!("invalid [env] key {:?}: leading or trailing whitespace", key);
            }
            let lower = key.to_ascii_lowercase();
            if let Some(existing) = seen.get(&lower) {
                bail!(
                    "duplicate [env] key (case-insensitive, breaks on Windows): {:?} vs {:?}",
                    existing,
                    key
                );
            }
            seen.insert(lower, key.clone());
        }
        Ok(())
    }

    /// Apply a list of `key.path=value` overrides (from `-s/--set`).
    /// Each entry's right-hand side is parsed as a TOML expression, so
    /// booleans, integers, strings, and arrays all work without per-flag
    /// type plumbing. Dotted keys walk into nested tables, creating them
    /// as needed.
    ///
    /// Precedence: `--set` overlays the file-loaded config and loses only
    /// to surviving CLI flags applied by the caller. `.scrutin/config.toml` is the
    /// only persistent source of truth — there are intentionally no
    /// `SCRUTIN_*` config env vars to layer in between.
    pub fn apply_set_overrides(&mut self, entries: &[String]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        // Round-trip through toml::Value: serialize current state, patch in
        // place, deserialize back. This bypasses the need for per-field
        // setters and lets every dotted key path work uniformly.
        let mut value = toml::Value::try_from(&*self)
            .map_err(|e| anyhow::anyhow!("internal: failed to serialize config: {e}"))?;
        for entry in entries {
            apply_one_override(&mut value, entry)?;
        }
        let new_cfg: Config = value
            .try_into()
            .map_err(|e| anyhow::anyhow!("--set produced invalid config: {e}"))?;
        new_cfg.validate_env()?;
        *self = new_cfg;
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Config> {
        let cfg: Config = toml::from_str(s)?;
        cfg.validate_env()?;
        Ok(cfg)
    }

    #[test]
    fn empty_env_section_is_default() {
        let cfg = parse("").unwrap();
        assert!(cfg.env.is_empty());
    }

    #[test]
    fn env_section_round_trips() {
        let cfg = parse(
            r#"
[env]
RUST_LOG = "debug"
DATABASE_URL = "postgres://localhost/test"
EMPTY = ""
"#,
        )
        .unwrap();
        assert_eq!(cfg.env.get("RUST_LOG").map(String::as_str), Some("debug"));
        assert_eq!(
            cfg.env.get("DATABASE_URL").map(String::as_str),
            Some("postgres://localhost/test")
        );
        assert_eq!(cfg.env.get("EMPTY").map(String::as_str), Some(""));
    }

    #[test]
    fn env_rejects_case_insensitive_duplicates() {
        // Path vs PATH would silently collapse on Windows.
        let err = parse(
            r#"
[env]
Path = "a"
PATH = "b"
"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("case-insensitive"),
            "got: {err}"
        );
    }

    #[test]
    fn set_override_typed_values() {
        let mut cfg = Config::default();
        cfg.apply_set_overrides(&[
            "run.workers=8".into(),
            "run.shuffle=true".into(),
            "run.max_fail=3".into(),
            "run.tool=pytest".into(),
        ])
        .unwrap();
        assert_eq!(cfg.run.workers, Some(8));
        assert!(cfg.run.shuffle);
        assert_eq!(cfg.run.max_fail, 3);
        assert_eq!(cfg.run.tool, "pytest");
    }

    #[test]
    fn set_override_creates_nested_table() {
        let mut cfg = Config::default();
        cfg.apply_set_overrides(&["env.DATABASE_URL=postgres://x".into()])
            .unwrap();
        assert_eq!(
            cfg.env.get("DATABASE_URL").map(String::as_str),
            Some("postgres://x")
        );
    }

    #[test]
    fn set_override_array_replaces() {
        let mut cfg = Config::default();
        cfg.apply_set_overrides(&[r#"pytest.extra_args=["--tb=short","-vv"]"#.into()])
            .unwrap();
        assert_eq!(cfg.pytest.extra_args, vec!["--tb=short", "-vv"]);
    }

    #[test]
    fn set_override_validates_env_keys() {
        let mut cfg = Config::default();
        let err = cfg
            .apply_set_overrides(&["env.PATH=/a".into(), "env.path=/b".into()])
            .unwrap_err();
        assert!(err.to_string().contains("case-insensitive"), "got: {err}");
    }

    #[test]
    fn set_override_rejects_missing_eq() {
        let mut cfg = Config::default();
        assert!(cfg.apply_set_overrides(&["run.workers".into()]).is_err());
    }

    #[test]
    fn env_rejects_invalid_key_chars() {
        assert!(parse("[env]\n\"FOO=BAR\" = \"x\"\n").is_err());
        // Leading whitespace.
        assert!(parse("[env]\n\" FOO\" = \"x\"\n").is_err());
        // Empty key.
        assert!(parse("[env]\n\"\" = \"x\"\n").is_err());
    }
}

/// Custom deserializer for `[extras]` that accepts any TOML scalar
/// (string, integer, float, bool) and stringifies it. Lets users write
/// `--set extras.build=4521` (where the RHS is parsed as an int) without
/// forcing them to quote everything. Tables and arrays are rejected :
/// labels are key/value strings, not nested structures.
fn deserialize_extra_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as DeError;
    let raw: BTreeMap<String, toml::Value> = BTreeMap::deserialize(deserializer)?;
    let mut out = BTreeMap::new();
    for (k, v) in raw {
        let s = match v {
            toml::Value::String(s) => s,
            toml::Value::Integer(i) => i.to_string(),
            toml::Value::Float(f) => f.to_string(),
            toml::Value::Boolean(b) => b.to_string(),
            toml::Value::Datetime(d) => d.to_string(),
            toml::Value::Array(_) | toml::Value::Table(_) => {
                return Err(DeError::custom(format!(
                    "extras.{k}: expected scalar (string, int, float, bool), got array/table"
                )));
            }
        };
        out.insert(k, s);
    }
    Ok(out)
}

/// Apply a single `key.path=value` override to a `toml::Value` tree, in
/// place. Splits the key on `.`, walks into nested tables (creating them
/// where missing), and parses the right-hand side as a TOML expression so
/// types come from TOML's grammar (`true`, `42`, `"foo"`, `["a", "b"]`).
/// Anything that doesn't parse as a TOML expression is treated as a bare
/// string, so `--set run.tool=pytest` works without quoting.
fn apply_one_override(root: &mut toml::Value, entry: &str) -> Result<()> {
    let (key, raw_value) = entry
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("invalid --set {entry:?}: expected key=value"))?;
    let key = key.trim();
    if key.is_empty() {
        bail!("invalid --set {entry:?}: empty key");
    }

    // Parse the RHS as a TOML expression by wrapping it in `_ = <rhs>` and
    // pulling the value back out. Falls back to a bare string on parse
    // failure so unquoted identifiers (paths, tool names) Just Work.
    // Parse `_ = <rhs>` as a TOML doc. Reject docs with extra entries
    // (e.g. a value containing a newline + `[section]`) so a stray newline
    // can't silently inject unrelated config.
    let parsed_value = match format!("_ = {}", raw_value).parse::<toml::Value>() {
        Ok(toml::Value::Table(mut t)) if t.len() == 1 => t
            .remove("_")
            .unwrap_or_else(|| toml::Value::String(raw_value.to_string())),
        _ => toml::Value::String(raw_value.to_string()),
    };

    // Walk the dotted path, creating intermediate tables as needed.
    let parts: Vec<&str> = key.split('.').collect();
    let mut cursor = root;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            bail!("invalid --set {entry:?}: empty path segment");
        }
        let is_last = i == parts.len() - 1;
        // Ensure the cursor is a table before indexing.
        if !cursor.is_table() {
            bail!(
                "invalid --set {entry:?}: cannot index into non-table at {:?}",
                parts[..i].join(".")
            );
        }
        let table = cursor.as_table_mut().expect("just checked is_table");
        if is_last {
            table.insert(part.to_string(), parsed_value);
            return Ok(());
        }
        // Descend, creating an empty table if missing.
        cursor = table
            .entry(part.to_string())
            .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    }
    Ok(())
}

