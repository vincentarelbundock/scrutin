//! Command-line interface, argument parsing, and top-level orchestration.
//!
//! `main.rs` is a 5-line shell that delegates to [`run`]. Reporter
//! implementations live in the [`reporter`] submodule; this file owns CLI
//! parsing, config layering, subcommand dispatch, and the `init`/`stats`
//! verbs.

mod reporter;
mod style;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use scrutin_core::analysis::hashing;
use scrutin_core::engine::pool::ProcessPool;
use scrutin_core::filter::apply_include_exclude;
use scrutin_core::project::hooks::{self, ProcessHooks};
use scrutin_core::logbuf;
use scrutin_core::metadata::{self, RunMetadata};
use scrutin_core::project::config::Config;
use scrutin_core::project::package::Package;
use scrutin_core::storage::sqlite;
use scrutin_tui as tui;

/// Top-level CLI. Most invocations are `scrutin [path]`, which is shorthand
/// for `scrutin run [path]`. Verbs that don't run tests (`init`, `stats`)
/// live as explicit subcommands so each gets its own `--help`.
#[derive(Parser)]
#[command(
    name = "scrutin",
    version,
    about = "Fast watch-mode test runner",
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Default: run tests. Flags here are forwarded to the implicit `run`
    /// subcommand when no explicit subcommand is given.
    #[command(flatten)]
    pub run_args: RunArgs,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run tests (default).
    Run(RunArgs),
    /// Initialize scaffolding. Default: `.scrutin/config.toml` and runner
    /// scripts in the current package. `init skill` installs the Agent
    /// Skill for Claude Code / Codex instead.
    Init(InitArgs),
    /// Show flaky tests and slowness statistics from the local history DB.
    Stats {
        /// Path to the project (default: current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Generate documentation artifacts (CLI reference, man pages, shell completions).
    #[cfg(feature = "generate-docs")]
    #[command(hide = true)]
    GenerateDocs {
        /// Output directory for generated files.
        #[arg(default_value = "target/docs")]
        out_dir: PathBuf,
    },
}

/// Args for `scrutin init`. Either a nested subcommand (`init skill`) or a
/// positional project path (`init [PATH]`), never both.
#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct InitArgs {
    #[command(subcommand)]
    pub kind: Option<InitKind>,

    /// Path to the project (default: current directory). Used when no
    /// subcommand is given.
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

#[derive(Subcommand)]
pub enum InitKind {
    /// Install the scrutin Agent Skill for Claude Code, Codex, or any
    /// other agent that loads `~/.claude/skills/<name>/SKILL.md`.
    ///
    /// Default destination: `~/.claude/skills/scrutin/`. Pass a directory
    /// to override, or `-` to write the skill to stdout instead of a file.
    Skill {
        /// Destination directory, or `-` for stdout.
        path: Option<String>,

        /// Overwrite an existing `SKILL.md` at the destination.
        #[arg(long)]
        force: bool,
    },
}

#[derive(clap::Args, Default)]
pub struct RunArgs {
    /// Path(s) to the project or to individual files. A single directory is
    /// the project root (default: `.`). One or more file paths activates
    /// file-mode: scrutin runs the tool named by `--tool` on just those
    /// files, with no project context. Mixing files and directories is an
    /// error.
    pub paths: Vec<PathBuf>,

    /// Tool to run in file-mode. Sugar for `--set run.tool=<name>`.
    /// Required when `paths` contains files instead of a directory. Must
    /// name a command-mode plugin (skyspell, jarl, ruff); worker-mode
    /// plugins (pytest, testthat, ...) need a project root.
    #[arg(short = 't', long = "tool", value_name = "NAME")]
    pub tool: Option<String>,

    /// Output reporter. Values: `tui`, `plain`, `github`, `web[:ADDR]`,
    /// `list`, `junit:PATH`. Defaults to `tui` when stderr is a tty, else
    /// `plain`. File-mode defaults to `plain` regardless.
    #[arg(short = 'r', long = "reporter", value_name = "NAME[:ARG]")]
    pub reporter: Vec<ReporterSpec>,

    /// Override a .scrutin/config.toml field. Repeatable. Dotted keys walk into
    /// nested tables (e.g. `run.workers=8`, `filter.include=["test_math*"]`,
    /// `watch.enabled=true`, `filter.group=fast`). RHS is parsed as a TOML
    /// expression, falling back to a bare string for unquoted values.
    #[arg(short = 's', long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

/// One output reporter. Stream reporters write to stderr / own the terminal;
/// file reporters write to a path; `list` is a special one-shot reporter.
/// Parsed from clap via `FromStr`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReporterSpec {
    Tui,
    Plain,
    Github,
    Web(Option<String>),
    List,
    Junit(PathBuf),
}

impl std::str::FromStr for ReporterSpec {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, arg) = match s.split_once(':') {
            Some((n, a)) => (n, Some(a)),
            None => (s, None),
        };
        match (name, arg) {
            ("tui", None) => Ok(ReporterSpec::Tui),
            ("plain", None) => Ok(ReporterSpec::Plain),
            ("github", None) => Ok(ReporterSpec::Github),
            ("web", None) => Ok(ReporterSpec::Web(None)),
            ("web", Some(a)) if !a.is_empty() => Ok(ReporterSpec::Web(Some(a.to_string()))),
            ("web", Some(_)) => Err("reporter 'web' address must be non-empty: web:ADDR".into()),
            ("list", None) => Ok(ReporterSpec::List),
            ("junit", Some(p)) if !p.is_empty() => Ok(ReporterSpec::Junit(PathBuf::from(p))),
            ("junit", _) => Err("reporter 'junit' requires a path: junit:PATH".into()),
            ("tui" | "plain" | "github" | "list", Some(_)) => {
                Err(format!("reporter '{name}' does not take an argument"))
            }
            _ => Err(format!(
                "unknown target '{name}' (expected: tui, plain, github, web[:ADDR], list, junit:PATH)"
            )),
        }
    }
}

/// The resolved reporter after defaults have been applied.
#[derive(Debug)]
pub enum Reporter {
    Tui,
    Plain,
    Github,
    Web {
        addr: std::net::SocketAddr,
    },
    List,
    Junit(PathBuf),
}

impl Reporter {
    pub fn is_plain(&self) -> bool {
        matches!(self, Reporter::Plain)
    }
}

/// Resolve the user-supplied reporter (or pick a default). Centralizes all
/// the defaulting rules so the run loop only sees a fully-resolved enum.
///
pub fn resolve_reporter(spec: Option<&ReporterSpec>) -> Result<Reporter> {
    let spec = match spec {
        Some(s) => s.clone(),
        None => {
            if std::io::stderr().is_terminal() {
                ReporterSpec::Tui
            } else {
                ReporterSpec::Plain
            }
        }
    };
    match spec {
        ReporterSpec::Tui => Ok(Reporter::Tui),
        ReporterSpec::Plain => Ok(Reporter::Plain),
        ReporterSpec::Github => Ok(Reporter::Github),
        ReporterSpec::List => Ok(Reporter::List),
        ReporterSpec::Junit(p) => Ok(Reporter::Junit(p)),
        ReporterSpec::Web(addr_opt) => {
            let addr_str = addr_opt.as_deref().unwrap_or("127.0.0.1:7878");
            let addr: std::net::SocketAddr = addr_str.parse().map_err(|e| {
                anyhow::anyhow!("invalid web address {:?}: {}", addr_str, e)
            })?;
            Ok(Reporter::Web { addr })
        }
    }
}

/// RAII guard ensuring process-teardown hooks fire even on early error
/// returns (`?` propagation, panics, etc.).
struct TeardownGuard {
    hooks: Option<ProcessHooks>,
}

impl TeardownGuard {
    fn new(hooks: ProcessHooks) -> Self {
        Self { hooks: Some(hooks) }
    }
    fn disarm(&mut self) -> Option<ProcessHooks> {
        self.hooks.take()
    }
}

impl Drop for TeardownGuard {
    fn drop(&mut self) {
        if let Some(h) = self.hooks.take() {
            h.run_teardown();
        }
    }
}

pub async fn run() -> Result<()> {
    install_signal_handler();

    let cli = Cli::parse();
    // The default subcommand is `run` with the top-level `run_args`.
    let command = cli.command.unwrap_or(Command::Run(cli.run_args));

    match command {
        Command::Run(args) => run_subcommand(args).await,
        Command::Init(InitArgs {
            kind: Some(InitKind::Skill { path, force }),
            ..
        }) => run_init_skill(path.as_deref(), force),
        Command::Init(InitArgs { kind: None, path }) => {
            let root = std::fs::canonicalize(&path)
                .with_context(|| format!("path does not exist: {}", path.display()))?;
            let pkg = discover_for_verb(&root)?;
            run_init(&pkg)
        }
        Command::Stats { path } => {
            let root = std::fs::canonicalize(&path)
                .with_context(|| format!("path does not exist: {}", path.display()))?;
            run_stats(&root)
        }
        #[cfg(feature = "generate-docs")]
        Command::GenerateDocs { out_dir } => generate_docs(&out_dir),
    }
}

/// Discover a `Package` for the `init` verb. Uses auto-detection only
/// (ignores `[[suite]]` config since init is about scaffolding).
fn discover_for_verb(root: &Path) -> Result<Package> {
    Package::new(
        root.to_path_buf(),
        &[],
        "auto",
        &[],
        &[],
        &[],
        Vec::new(),
        |_| Ok(scrutin_core::project::package::WorkerHookPaths::default()),
        Default::default(),
    )
}


async fn run_subcommand(mut args: RunArgs) -> Result<()> {
    // Default to "." when no positional path is given.
    if args.paths.is_empty() {
        args.paths.push(PathBuf::from("."));
    }
    // Canonicalize up-front: R's pkgload::load_all resolves relative paths
    // against its own cwd, so `demo` would blow up inside R.
    let canonical_paths: Vec<PathBuf> = args
        .paths
        .iter()
        .map(|p| {
            std::fs::canonicalize(p)
                .with_context(|| format!("path does not exist: {}", p.display()))
        })
        .collect::<Result<_>>()?;

    // Classify: exactly one directory, or one-or-more files. Anything else
    // is an error (mixing modes, or a non-file non-directory entry).
    let all_files = canonical_paths.iter().all(|p| p.is_file());
    let one_dir = canonical_paths.len() == 1 && canonical_paths[0].is_dir();
    if !all_files && !one_dir {
        anyhow::bail!(
            "mix of files and directories in positional args; pass either one directory or one-or-more files"
        );
    }
    let file_mode = all_files;

    // Fold `--tool X` into the `--set` overrides so the regular config
    // layering picks it up.
    let mut set_overrides = args.set.clone();
    if let Some(tool) = args.tool.as_deref() {
        set_overrides.push(format!("run.tool={}", tool));
    }

    // In file-mode the CLI argument is a file path; `.scrutin/` state lives
    // in a throwaway tempdir so we don't pollute the user's CWD or the
    // file's parent. In dir-mode the sole positional path is the project
    // root. `config_root` is where `Config::load` looks for `.scrutin/
    // config.toml`; in file-mode that's the tempdir, so project-local
    // config is skipped. The user-level fallback (`dirs::config_dir()`)
    // still applies, so global preferences carry over. `--tool` is still
    // required in file-mode because there are no project markers to
    // auto-detect from.
    let tempdir_guard: Option<tempfile::TempDir> = if file_mode {
        Some(tempfile::tempdir().context("creating file-mode tempdir")?)
    } else {
        None
    };
    let config_root: PathBuf = if file_mode {
        tempdir_guard.as_ref().unwrap().path().to_path_buf()
    } else {
        canonical_paths[0].clone()
    };

    // Config layering: defaults -> .scrutin/config.toml -> --set -> CLI flags.
    // scrutin intentionally has no config env vars; .scrutin/config.toml is
    // the only persistent source of truth.
    let mut cfg = Config::load(&config_root)?;
    cfg.apply_set_overrides(&set_overrides)?;

    if file_mode {
        // Watch mode makes no sense for a one-shot lint/spell-check.
        cfg.watch.enabled = false;
    }

    if !cfg.run.color {
        style::disable_color();
    }

    // Resolve reporter before anything else so we know whether to talk to
    // a tty or stay quiet about color, headers, etc. File-mode defaults to
    // plain even on a tty; explicit `-r tui`/`-r web` is still respected.
    if args.reporter.len() > 1 {
        anyhow::bail!("only one --reporter (-r) is allowed");
    }
    let explicit_reporter = args.reporter.first();
    let reporter = if file_mode && explicit_reporter.is_none() {
        Reporter::Plain
    } else {
        resolve_reporter(explicit_reporter)?
    };

    // Startup hook runs before *anything* touches plugins or spawns
    // subprocesses. In file-mode `config_root` is the scratch tempdir, so
    // user hooks are silently skipped: nothing to run.
    let process_hooks = ProcessHooks::from_config(&cfg, &config_root);
    process_hooks.run_startup()?;
    let mut teardown_guard = TeardownGuard::new(process_hooks);

    // Build the package. File-mode skips auto-detection + [[suite]] and
    // goes straight to `from_files` with the user-named tool.
    let root = config_root.clone();
    let cfg_for_hooks = cfg.clone();
    let root_for_hooks = root.clone();
    let python_interpreter = cfg.python.resolve_interpreter(&root);

    let pkg = if file_mode {
        if cfg.run.tool == "auto" {
            anyhow::bail!(
                "file-mode requires a tool: pass --tool <name> or --set run.tool=<name>"
            );
        }
        Package::from_files(
            root,
            &canonical_paths,
            &cfg.run.tool,
            &cfg.pytest.extra_args,
            &cfg.skyspell.extra_args,
            &cfg.skyspell.add_args,
            python_interpreter,
            cfg.env.clone(),
        )?
    } else {
        // When [[suite]] entries exist, use them (optionally filtered by
        // --set run.tool). Otherwise auto-detect from marker files.
        let suites: Vec<_> = if cfg.suites.is_empty() {
            Vec::new()
        } else if cfg.run.tool == "auto" {
            cfg.suites.clone()
        } else {
            cfg.suites
                .iter()
                .filter(|s| s.tool == cfg.run.tool)
                .cloned()
                .collect()
        };
        if !cfg.suites.is_empty() && suites.is_empty() {
            anyhow::bail!(
                "No [[suite]] entries match --set run.tool={:?}",
                cfg.run.tool
            );
        }
        Package::new(
            root,
            &suites,
            &cfg.run.tool,
            &cfg.pytest.extra_args,
            &cfg.skyspell.extra_args,
            &cfg.skyspell.add_args,
            python_interpreter,
            |plugin| {
                let wh = hooks::resolve_worker_hooks(&cfg_for_hooks, plugin, &root_for_hooks)?;
                Ok(scrutin_core::project::package::WorkerHookPaths {
                    startup: wh.startup,
                    teardown: wh.teardown,
                })
            },
            cfg.env.clone(),
        )?
    };
    let n_workers = cfg.run.workers.unwrap_or_else(ProcessPool::default_workers);

    // Startup pre-flight: verify suite roots exist, run globs match
    // files, command-mode tools are on PATH, Python project module
    // imports cleanly, R `pkgload` is installed. Each check fails fast
    // with an actionable error instead of producing per-file noise
    // mid-run. Disable with `[preflight] enabled = false`.
    scrutin_core::preflight::run_all(&pkg, &cfg.preflight)?;

    print_header(&pkg, n_workers, reporter.is_plain());

    let mut test_files = if file_mode {
        canonical_paths.clone()
    } else {
        pkg.test_files()?
    };
    let filter = resolve_filter_args(&cfg)?;
    // Tool filter: restrict to files owned by the named tools.
    if !filter.tools.is_empty() {
        test_files.retain(|f| {
            pkg.suite_for(f)
                .is_some_and(|s| filter.tools.iter().any(|t| t == s.plugin.name()))
        });
    }
    apply_include_exclude(&mut test_files, &filter.includes, &filter.excludes);
    if test_files.is_empty() {
        eprintln!("No test files found.");
        return Ok(());
    }
    // Shuffle (if `[run] shuffle` is set or `[run] seed` is set).
    if cfg.run.shuffle || cfg.run.seed.is_some() {
        let seed = cfg.run.seed.unwrap_or_else(fresh_seed);
        eprintln!("{}", style::dim(format!("shuffle seed: {}", seed)));
        shuffle_files(&mut test_files, seed);
    }

    // List reporter is a one-shot: print matching files and exit before
    // touching dep maps, watch, or any subprocess.
    if matches!(reporter, Reporter::List) {
        println!(
            "{} test file{} would run",
            test_files.len(),
            if test_files.len() == 1 { "" } else { "s" }
        );
        for f in &test_files {
            let rel = f.strip_prefix(&pkg.root).unwrap_or(f);
            println!("  {}", rel.display());
        }
        return Ok(());
    }

    // Both R and pytest contribute to the same unified dep map keyed by
    // path-relative-to-root.
    let depmap_stale = hashing::is_dep_map_stale(&pkg).unwrap_or(true);
    let dep_map = sqlite::with_open(&pkg.root, |c| Ok(sqlite::load_dep_map(c)))
        .ok()
        .filter(|m| !m.is_empty());

    let is_full_suite = filter.tools.is_empty()
        && filter.includes.is_empty()
        && filter.excludes.is_empty();

    let run_metadata = build_run_metadata(&cfg, &pkg, n_workers);

    // Watch mode: TUI and web use the config default (true); plain and
    // JUnit default to one-shot. Override with `-s watch.enabled=true/false`.
    let watch = match reporter {
        Reporter::Tui | Reporter::Web { .. } => cfg.watch.enabled,
        Reporter::Plain | Reporter::Github | Reporter::Junit(_) | Reporter::List => false,
    };

    let exit_code = match reporter {
        Reporter::List => unreachable!("handled above"),
        Reporter::Web { addr } => {
            let mut groups: Vec<scrutin_web::WireFilterGroup> = cfg
                .filter
                .groups
                .iter()
                .map(|(name, g)| scrutin_web::WireFilterGroup {
                    name: name.clone(),
                    include: g.include.clone(),
                    exclude: g.exclude.clone(),
                    tools: g.tools.clone(),
                })
                .collect();
            groups.sort_by(|a, b| a.name.cmp(&b.name));
            let active_group: Option<String> = cfg
                .filter
                .group
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from);
            scrutin_web::run_web(
                addr,
                pkg.clone(),
                n_workers,
                test_files.clone(),
                watch,
                cfg.run.timeout_file_ms,
                cfg.run.timeout_run_ms,
                cfg.run.fork_workers,
                cfg.web.editor.clone(),
                cfg.agent.clone(),
                groups,
                active_group,
            )
            .await?;
            0
        }
        Reporter::Tui => {
            run_tui_mode(
                &pkg,
                &test_files,
                &filter.includes,
                &filter.excludes,
                n_workers,
                watch,
                dep_map,
                &cfg,
            )
            .await?
        }
        Reporter::Plain => {
            reporter::plain::run(
                &pkg,
                &test_files,
                &filter.includes,
                &filter.excludes,
                n_workers,
                watch,
                dep_map,
                &cfg,
                None,
                depmap_stale,
                is_full_suite,
                &run_metadata,
            )
            .await?
        }
        Reporter::Github => {
            reporter::github::run(
                &pkg,
                &test_files,
                n_workers,
                &cfg,
                depmap_stale,
                is_full_suite,
                &run_metadata,
            )
            .await?
        }
        Reporter::Junit(ref junit_path) => {
            reporter::plain::run(
                &pkg,
                &test_files,
                &filter.includes,
                &filter.excludes,
                n_workers,
                watch,
                dep_map,
                &cfg,
                Some(junit_path),
                depmap_stale,
                is_full_suite,
                &run_metadata,
            )
            .await?
        }
    };

    if let Some(h) = teardown_guard.disarm() {
        h.run_teardown();
    }
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

fn install_signal_handler() {
    tokio::spawn(async {
        if tokio::signal::ctrl_c().await.is_ok() {
            // Restore the terminal in case the TUI was active (raw mode,
            // alternate screen). ratatui::restore() is a no-op when the
            // terminal was never initialized.
            ratatui::restore();
            eprintln!("\nscrutin interrupted.");
            std::process::exit(130);
        }
    });
}


/// Assemble the per-run [`RunMetadata`] from config + automatic
/// provenance.
fn build_run_metadata(cfg: &Config, pkg: &Package, n_workers: usize) -> RunMetadata {
    let mut provenance = metadata::capture_provenance(&pkg.root, cfg.metadata.enabled);
    if cfg.metadata.enabled {
        provenance.tool = Some(pkg.tool_names());
        provenance.workers = Some(n_workers);
    }
    RunMetadata {
        provenance,
        labels: cfg.extras.clone(),
    }
}

/// Fallback seed when system time is unavailable or `seed = 0` is configured.
const FALLBACK_SEED: u64 = 0xdead_beef_cafe_babe;

/// Draw a fresh seed from system time.
fn fresh_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(FALLBACK_SEED)
}

/// Fisher-Yates shuffle driven by a seeded xorshift64 PRNG.
fn shuffle_files(files: &mut [PathBuf], seed: u64) {
    let mut state = if seed == 0 { FALLBACK_SEED } else { seed };
    let mut next = || {
        // xorshift64 (Marsaglia)
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..files.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        files.swap(i, j);
    }
}

fn run_stats(root: &Path) -> Result<()> {
    let db_path = root.join(".scrutin").join("state.db");
    if !db_path.exists() {
        eprintln!("No database found. Run tests first to build history.");
        return Ok(());
    }
    print_stats(root);
    Ok(())
}


#[allow(clippy::too_many_arguments)]
async fn run_tui_mode(
    pkg: &Package,
    test_files: &[PathBuf],
    includes: &[String],
    excludes: &[String],
    n_workers: usize,
    watch: bool,
    dep_map: Option<std::collections::HashMap<String, Vec<String>>>,
    cfg: &Config,
) -> Result<i32> {
    let log = logbuf::LogBuffer::new();
    let mut run_groups: Vec<tui::RunGroup> = cfg
        .filter
        .groups
        .iter()
        .map(|(name, g)| tui::RunGroup {
            name: name.clone(),
            include: g.include.clone(),
            exclude: g.exclude.clone(),
            tools: g.tools.clone(),
        })
        .collect();
    run_groups.sort_by(|a, b| a.name.cmp(&b.name));
    let active_group: Option<String> = cfg
        .filter
        .group
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    tui::run_tui(
        pkg,
        test_files,
        includes,
        excludes,
        n_workers,
        watch,
        dep_map,
        log,
        run_groups,
        active_group,
        cfg.run.reruns,
        cfg.run.reruns_delay,
        cfg.watch.debounce_ms,
        cfg.run.timeout_file_ms,
        cfg.run.timeout_run_ms,
        cfg.run.fork_workers,
        &cfg.keymap,
        cfg.agent.clone(),
    )
    .await?;
    Ok(0)
}

// --- Stats ---

fn print_stats(root: &Path) {
    let conn = match sqlite::open(root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", style::dim(format!("could not open history DB: {e}")));
            return;
        }
    };

    match sqlite::flaky_tests(&conn) {
        Ok(flaky) if !flaky.is_empty() => {
            eprintln!("{}", style::yellow_bold("Flaky tests (alternating pass/fail):"));
            for t in &flaky {
                eprintln!(
                    "  {} {} > {}  ({}/{} failed, {:.0}% flake rate)",
                    style::yellow("⚠"),
                    style::dim(t.file.as_str()),
                    t.test,
                    t.failures,
                    t.total,
                    t.flake_rate * 100.0
                );
            }
            eprintln!();
        }
        _ => {
            eprintln!("{}", style::dim("No flaky tests detected."));
            eprintln!();
        }
    }

    match sqlite::slow_tests(&conn) {
        Ok(slow) if !slow.is_empty() => {
            eprintln!("{}", style::cyan_bold("Slowest tests:"));
            for t in &slow {
                eprintln!(
                    "  {} {} > {}  (avg {}ms, max {}ms, {} runs)",
                    style::cyan("◑"),
                    style::dim(t.file.as_str()),
                    t.test,
                    t.avg_ms as u64,
                    t.max_ms as u64,
                    t.runs
                );
            }
        }
        _ => {
            eprintln!("{}", style::dim("No slow test data yet."));
        }
    }
}

// --- Init (skill) ---

/// The canonical Agent Skill, shipped in-repo at `skills/scrutin/SKILL.md`.
/// Embedded into the binary so `scrutin init skill` can install it without
/// a separate download. `build.rs` stages the file into `OUT_DIR` so the
/// `include_str!` path stays inside the crate root (required for publishing).
const SKILL_MD: &str = include_str!(concat!(env!("OUT_DIR"), "/SKILL.md"));

fn run_init_skill(dest: Option<&str>, force: bool) -> Result<()> {
    if dest == Some("-") {
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        out.write_all(SKILL_MD.as_bytes())?;
        return Ok(());
    }

    let dest_dir = match dest {
        Some(d) => PathBuf::from(d),
        None => dirs::home_dir()
            .context("could not locate home directory for default install path")?
            .join(".claude")
            .join("skills")
            .join("scrutin"),
    };

    std::fs::create_dir_all(&dest_dir)
        .with_context(|| format!("creating {}", dest_dir.display()))?;

    let skill_path = dest_dir.join("SKILL.md");
    if skill_path.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite",
            skill_path.display()
        );
    }
    std::fs::write(&skill_path, SKILL_MD)
        .with_context(|| format!("writing {}", skill_path.display()))?;

    eprintln!("Installed scrutin skill to {}", skill_path.display());
    Ok(())
}

// --- Init ---

/// Render the annotated `.scrutin/config.toml` template: substitute `{{DETECTED}}`
/// and append a `[keymap.<mode>]` subtable per mode with every default
/// binding written as a commented `# "key" = "action"` line. Commented
/// defaults mean the file is a no-op as-shipped (defaults apply); users
/// uncomment the bindings they want to override (replace semantics).
/// Shared between `scrutin init` and the docs-generation path so they
/// can't drift.
pub(crate) fn render_config_template(detected: &str) -> String {
    let mut content = include_str!("init_template.toml")
        .replace("{{DETECTED}}", detected);
    for (mode_name, entries) in scrutin_tui::default_keymap_for_init() {
        content.push_str(&format!("\n# [keymap.{}]\n", mode_name));
        let max_key_len = entries.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        for (key, action) in entries {
            content.push_str(&format!(
                "# {:<width$} = \"{}\"\n",
                format!("\"{}\"", key),
                action,
                width = max_key_len + 2
            ));
        }
    }
    content
}

fn run_init(pkg: &Package) -> Result<()> {
    let detected = pkg.tool_names();

    let scrutin_dir = pkg.root.join(".scrutin");
    std::fs::create_dir_all(&scrutin_dir)?;
    eprintln!("Created .scrutin/");

    let toml_path = scrutin_dir.join("config.toml");
    if toml_path.exists() {
        eprintln!(".scrutin/config.toml already exists, skipping.");
    } else {
        // Always write `tool = "auto"` -- the loader only accepts a
        // single plugin name or "auto", and a multi-suite project produces
        // a `+`-joined display name that the loader would reject.
        let content = render_config_template(&detected);
        std::fs::write(&toml_path, content)?;
        eprintln!("Created .scrutin/config.toml");
    }

    // Scaffold editable runner scripts; the engine prefers these over
    // the embedded defaults whenever present.
    let runners_dir = scrutin_dir.join("runners");
    std::fs::create_dir_all(&runners_dir)?;
    for suite in &pkg.test_suites {
        let plugin = &suite.plugin;
        // Command-mode plugins (ruff, jarl, skyspell) have no runner script.
        if plugin.runner_script().is_empty() {
            continue;
        }
        let runner_path = runners_dir.join(plugin.runner_filename());
        let display_path = runner_path
            .strip_prefix(&pkg.root)
            .unwrap_or(&runner_path)
            .display();
        if runner_path.exists() {
            eprintln!("{} already exists, skipping.", display_path);
        } else {
            std::fs::write(&runner_path, plugin.runner_script())?;
            eprintln!("Wrote default runner to {}", display_path);
        }
    }

    // Only the SQLite database (run history + dep map + hash cache) is
    // per-machine throwaway state. Runner scripts under `.scrutin/runners/`
    // are user-editable and belong in version control so the whole team
    // picks up any customization.
    let gitignore_path = pkg.root.join(".gitignore");
    let needs_entry = if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path)?;
        !content.lines().any(|l| {
            let t = l.trim();
            t == ".scrutin/state.db*"
                || t == ".scrutin/"
                || t == ".scrutin"
        })
    } else {
        true
    };

    if needs_entry {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&gitignore_path)?;
        writeln!(f, "\n# scrutin history + caches (keep runner scripts tracked)")?;
        writeln!(f, ".scrutin/state.db*")?;
        eprintln!("Added .scrutin/state.db* to .gitignore");
    }

    eprintln!();
    eprintln!("Initialized scrutin for {} ({})", pkg.name, detected);
    eprintln!("Run `scrutin` to start testing.");

    Ok(())
}

// --- Helpers ---

fn print_header(pkg: &Package, n_workers: usize, ci: bool) {
    if !ci {
        eprintln!(
            "{} {} ({}, {} worker{})",
            style::bold("scrutin"),
            style::cyan(pkg.name.as_str()),
            pkg.tool_names(),
            n_workers,
            if n_workers == 1 { "" } else { "s" }
        );
        eprintln!();
    }
}

/// Resolved filter state: tool names to restrict to (empty = all),
/// include globs, exclude globs.
#[cfg_attr(test, derive(Debug))]
struct ResolvedFilter {
    tools: Vec<String>,
    includes: Vec<String>,
    excludes: Vec<String>,
}

fn resolve_filter_args(cfg: &Config) -> Result<ResolvedFilter> {
    // When `filter.group` is set, its include/exclude/tools *replace* the
    // top-level `[filter]`. With no group, fall through to top-level.
    let name = cfg.filter.group.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(name) = name {
        let g = cfg.filter.groups.get(name).ok_or_else(|| {
            let mut known: Vec<&str> = cfg.filter.groups.keys().map(String::as_str).collect();
            known.sort_unstable();
            let suffix = if known.is_empty() {
                " (no [filter.groups.*] defined in .scrutin/config.toml)".to_string()
            } else {
                format!(" (known: {})", known.join(", "))
            };
            anyhow::anyhow!("unknown filter group '{}'{}", name, suffix)
        })?;
        Ok(ResolvedFilter {
            tools: g.tools.clone(),
            includes: g.include.clone(),
            excludes: g.exclude.clone(),
        })
    } else {
        Ok(ResolvedFilter {
            tools: Vec::new(),
            includes: cfg.filter.include.clone(),
            excludes: cfg.filter.exclude.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Documentation generation (behind `generate-docs` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "generate-docs")]
fn generate_docs(out_dir: &Path) -> Result<()> {
    use clap::CommandFactory;

    std::fs::create_dir_all(out_dir)?;

    // CLI reference markdown
    let md = clap_markdown::help_markdown::<Cli>();
    let cli_md_path = out_dir.join("cli-reference.md");
    std::fs::write(&cli_md_path, md)?;
    eprintln!("  wrote {}", cli_md_path.display());

    // Rendered .scrutin/config.toml template (annotated template + commented
    // keymap defaults) for the configuration docs page.
    let cfg_toml = render_config_template("<auto-detected from the project root at init time>");
    let cfg_path = out_dir.join("configuration-template.toml");
    std::fs::write(&cfg_path, cfg_toml)?;
    eprintln!("  wrote {}", cfg_path.display());

    // Man page
    let man_dir = out_dir.join("man");
    std::fs::create_dir_all(&man_dir)?;
    let cmd = Cli::command();
    let man = clap_mangen::Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf)?;
    let man_path = man_dir.join("scrutin.1");
    std::fs::write(&man_path, buf)?;
    eprintln!("  wrote {}", man_path.display());

    // Shell completions
    let comp_dir = out_dir.join("completions");
    std::fs::create_dir_all(&comp_dir)?;
    let shells: &[(clap_complete::Shell, &str)] = &[
        (clap_complete::Shell::Bash, "scrutin.bash"),
        (clap_complete::Shell::Zsh, "_scrutin"),
        (clap_complete::Shell::Fish, "scrutin.fish"),
    ];
    for (shell, filename) in shells {
        let mut cmd = Cli::command();
        let mut file = std::fs::File::create(comp_dir.join(filename))?;
        clap_complete::generate(*shell, &mut cmd, "scrutin", &mut file);
        eprintln!("  wrote {}", comp_dir.join(filename).display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- resolve_reporter -----

    #[test]
    fn resolve_reporter_defaults_to_plain_when_no_tty() {
        let r = resolve_reporter(None).unwrap();
        assert!(matches!(r, Reporter::Plain));
    }

    #[test]
    fn resolve_reporter_web_parses_addr() {
        let r = resolve_reporter(Some(&ReporterSpec::Web(Some("127.0.0.1:9999".into())))).unwrap();
        if let Reporter::Web { addr, .. } = r {
            assert_eq!(addr.port(), 9999);
        } else {
            panic!("expected Web");
        }
    }

    #[test]
    fn resolve_reporter_web_default_addr() {
        let r = resolve_reporter(Some(&ReporterSpec::Web(None))).unwrap();
        if let Reporter::Web { addr, .. } = r {
            assert_eq!(addr.port(), 7878);
        } else {
            panic!("expected Web");
        }
    }

    #[test]
    fn resolve_reporter_web_invalid_addr() {
        let err = resolve_reporter(Some(&ReporterSpec::Web(Some("not-an-addr".into())))).unwrap_err();
        assert!(err.to_string().contains("invalid web address"));
    }

    #[test]
    fn resolve_reporter_list() {
        let r = resolve_reporter(Some(&ReporterSpec::List)).unwrap();
        assert!(matches!(r, Reporter::List));
    }

    #[test]
    fn resolve_reporter_junit() {
        let r = resolve_reporter(Some(&ReporterSpec::Junit("r.xml".into()))).unwrap();
        assert!(matches!(r, Reporter::Junit(_)));
    }

    #[test]
    fn resolve_reporter_github() {
        let r = resolve_reporter(Some(&ReporterSpec::Github)).unwrap();
        assert!(matches!(r, Reporter::Github));
    }

    // ----- ReporterSpec::FromStr -----

    #[test]
    fn reporter_spec_from_str_junit_requires_path() {
        assert!("junit".parse::<ReporterSpec>().is_err());
        assert!("junit:".parse::<ReporterSpec>().is_err());
        assert_eq!(
            "junit:r.xml".parse::<ReporterSpec>().unwrap(),
            ReporterSpec::Junit("r.xml".into())
        );
    }

    #[test]
    fn reporter_spec_from_str_web_variants() {
        assert_eq!("web".parse::<ReporterSpec>().unwrap(), ReporterSpec::Web(None));
        assert_eq!(
            "web:0.0.0.0:3000".parse::<ReporterSpec>().unwrap(),
            ReporterSpec::Web(Some("0.0.0.0:3000".into()))
        );
    }

    #[test]
    fn reporter_spec_from_str_list() {
        assert_eq!("list".parse::<ReporterSpec>().unwrap(), ReporterSpec::List);
        assert!("list:foo".parse::<ReporterSpec>().is_err());
    }

    #[test]
    fn reporter_spec_from_str_github() {
        assert_eq!("github".parse::<ReporterSpec>().unwrap(), ReporterSpec::Github);
        assert!("github:foo".parse::<ReporterSpec>().is_err());
    }

    #[test]
    fn reporter_spec_from_str_unknown() {
        assert!("xml".parse::<ReporterSpec>().is_err());
        assert!("plain:foo".parse::<ReporterSpec>().is_err());
    }

    // ----- resolve_filter_args + filter.group -----

    fn cfg_with_groups() -> Config {
        use scrutin_core::project::config::FilterGroup;
        let mut cfg = Config::default();
        cfg.filter.include = vec!["test-base*".into()];
        cfg.filter.groups.insert(
            "fast".into(),
            FilterGroup {
                tools: vec![],
                include: vec!["test-unit*".into()],
                exclude: vec!["test-slow*".into()],
            },
        );
        cfg.filter.groups.insert(
            "py_integration".into(),
            FilterGroup {
                tools: vec!["pytest".into()],
                include: vec!["test_integration_*".into()],
                exclude: vec![],
            },
        );
        cfg.filter.groups.insert(
            "r_only".into(),
            FilterGroup {
                tools: vec!["testthat".into(), "tinytest".into()],
                include: vec![],
                exclude: vec![],
            },
        );
        cfg
    }

    #[test]
    fn no_group_inherits_top_level_filter() {
        let cfg = cfg_with_groups();
        let f = resolve_filter_args(&cfg).unwrap();
        assert_eq!(f.includes, vec!["test-base*".to_string()]);
        assert!(f.excludes.is_empty());
        assert!(f.tools.is_empty());
    }

    #[test]
    fn group_replaces_top_level_filter() {
        let mut cfg = cfg_with_groups();
        cfg.filter.group = Some("fast".into());
        let f = resolve_filter_args(&cfg).unwrap();
        assert_eq!(
            f.includes,
            vec!["test-unit*".to_string()],
            "top-level test-base* is dropped when a group is selected"
        );
        assert_eq!(f.excludes, vec!["test-slow*".to_string()]);
        assert!(f.tools.is_empty(), "fast has no tools restriction");
    }

    #[test]
    fn group_applies_tool_restriction() {
        let mut cfg = cfg_with_groups();
        cfg.filter.group = Some("r_only".into());
        let f = resolve_filter_args(&cfg).unwrap();
        assert_eq!(
            f.tools,
            vec!["testthat".to_string(), "tinytest".into()]
        );
    }

    #[test]
    fn unknown_group_errors_with_known_list() {
        let mut cfg = cfg_with_groups();
        cfg.filter.group = Some("bogus".into());
        let err = resolve_filter_args(&cfg).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown filter group 'bogus'"));
        assert!(msg.contains("fast"));
    }

    #[test]
    fn empty_group_string_inherits_top_level() {
        let mut cfg = cfg_with_groups();
        cfg.filter.group = Some(String::new());
        let f = resolve_filter_args(&cfg).unwrap();
        assert_eq!(f.includes, vec!["test-base*".to_string()]);
    }
}
