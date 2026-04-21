use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;
use xxhash_rust::xxh64::xxh64;

use crate::engine::protocol::Message;
use crate::logbuf::LogBuffer;
use crate::project::hooks::absolute_under;
use crate::project::package::{Package, TestSuite};
use crate::project::plugin::Plugin;

/// Cap on the in-memory stderr ring buffer per worker. Older bytes are
/// truncated when a worker spews more than this — enough to surface a
/// failing test's traceback without unbounded growth.
const STDERR_BUF_CAP: usize = 8192;

/// An async R/Python worker subprocess. One worker = one long-lived child;
/// `run_test` writes a path on stdin and reads NDJSON until `Done`.
pub struct RProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr_buf: Arc<Mutex<String>>,
    timeout: Duration,
    // Stderr drain task; aborted on Drop.
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl RProcess {
    pub async fn spawn_with_timeout_and_log(
        pkg: &Package,
        suite: &TestSuite,
        timeout: Duration,
        _fork_workers: bool,
        log: Option<LogBuffer>,
    ) -> Result<Self> {
        // Non-fork-pool spawn: the old SCRUTIN_FORK_WORKERS path (for
        // Windows fallback where each worker forks per file internally).
        // Fork pool uses spawn_fork_parent() instead.
        Self::spawn_inner(pkg, suite, timeout, None, log).await
    }

    async fn spawn_inner(
        pkg: &Package,
        suite: &TestSuite,
        timeout: Duration,
        tcp_port: Option<u16>,
        log: Option<LogBuffer>,
    ) -> Result<Self> {
        let plugin = &suite.plugin;

        let contents = resolve_runner_contents(pkg, suite)?;
        let runner_path = materialise_runner(&pkg.root, plugin.as_ref(), &contents)?;

        let runner_path_str = runner_path.to_string_lossy().into_owned();
        let mut argv = plugin.subprocess_cmd(&suite.root, &runner_path_str);
        if argv.is_empty() {
            anyhow::bail!("tool {} returned empty subprocess command", plugin.name());
        }
        // For Python plugins, replace the auto-detected interpreter with
        // the user's [python].interpreter or [python].venv override.
        if plugin.language() == "python" && !pkg.python_interpreter.is_empty() {
            // argv is [interpreter, "-u", "<runner path>", ...]
            // Replace just the first element with the override tokens.
            let tail: Vec<String> = argv.drain(1..).collect();
            argv = pkg.python_interpreter.clone();
            argv.extend(tail);
        }
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        for (k, v) in plugin.env_vars(&suite.root) {
            cmd.env(k, v);
        }
        // Load strategy: tells the R runner how to make the package under
        // test available (`load_all` / `library` / `none`). Non-R suites
        // ignore it. Only emit when not the default so we don't spray a
        // meaningless env var into pytest/ruff/skyspell subprocesses.
        if plugin.language() == "r"
            && suite.r_load != crate::r::LoadStrategy::default()
        {
            cmd.env("SCRUTIN_LOAD_STRATEGY", suite.r_load.worker_env_value());
        }
        // Per-suite extra env populated by the engine (e.g. R_LIBS_USER
        // pointing at the temp library written by a pre-pool R CMD INSTALL).
        for (k, v) in &suite.extra_env {
            cmd.env(k, v);
        }
        if let Some(p) = &suite.worker_hooks.startup {
            cmd.env("SCRUTIN_WORKER_STARTUP", p);
        }
        if let Some(p) = &suite.worker_hooks.teardown {
            cmd.env("SCRUTIN_WORKER_TEARDOWN", p);
        }
        if !pkg.pytest_extra_args.is_empty() {
            // JSON-encoded so args containing spaces/quotes survive the
            // round trip into pytest_runner.py without a custom delimiter.
            let json = serde_json::to_string(&pkg.pytest_extra_args)
                .unwrap_or_else(|_| "[]".to_string());
            cmd.env("SCRUTIN_PYTEST_EXTRA_ARGS", json);
        }
        // TCP fork mode: the runner reads this port and forks children that
        // connect back to Rust via TCP to deliver NDJSON results.
        if let Some(port) = tcp_port {
            cmd.env("SCRUTIN_TCP_PORT", port.to_string());
        }
        // [env] from .scrutin/config.toml. Applied LAST so user vars win over both
        // inherited parent env (tokio's default) and scrutin's own injections
        // above. An empty value is intentional -- sets the var to empty.
        cmd.envs(pkg.env.iter());
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&suite.root)
            .kill_on_drop(true)
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to spawn {} subprocess ({}). Is it installed and on PATH?",
                    plugin.name(),
                    argv[0]
                )
            })?;

        let stdout = BufReader::new(child.stdout.take().unwrap());
        let stderr: ChildStderr = child.stderr.take().unwrap();
        let stdin = child.stdin.take().unwrap();

        // Drain stderr into a bounded buffer (for crash diagnostics) and mirror
        // each line to the shared LogBuffer if present. Unlike the old
        // thread-based version, this is a tokio task tied to the worker.
        let stderr_buf = Arc::new(Mutex::new(String::new()));
        let log_source = plugin.name().to_string();
        // Snapshot the suite (cheap Arc-based clone) so the spawned task can
        // call `plugin.is_noise_line` without borrowing across .await.
        let suite_for_filter = suite.clone();
        let stderr_task = {
            let stderr_buf = Arc::clone(&stderr_buf);
            let log = log.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if !suite_for_filter.plugin.is_noise_line(&line)
                        && let Some(ref lb) = log {
                            lb.push(&log_source, &format!("{}\n", line));
                        }
                    if let Ok(mut s) = stderr_buf.lock() {
                        s.push_str(&line);
                        s.push('\n');
                        if s.len() > STDERR_BUF_CAP {
                            let mut start = s.len() - STDERR_BUF_CAP;
                            // Advance to a char boundary so we don't
                            // slice inside a multibyte UTF-8 sequence.
                            while !s.is_char_boundary(start) {
                                start += 1;
                            }
                            *s = s[start..].to_string();
                        }
                    }
                }
            })
        };

        Ok(RProcess {
            child,
            stdin: Some(stdin),
            stdout,
            stderr_buf,
            timeout,
            stderr_task: Some(stderr_task),
        })
    }

    pub async fn run_test(&mut self, test_path: &Path) -> Result<Vec<Message>> {
        let path_str = test_path.to_string_lossy().into_owned();
        let cmd = format!("{}\n", path_str);
        if let Some(stdin) = self.stdin.as_mut() {
            // A worker that already died on startup (e.g. bad worker_startup
            // hook) will make these fail. Ignore here and let the read path
            // surface whatever the worker managed to emit on stdout before
            // exiting — we specifically want to catch `<worker_startup>`
            // error messages.
            let _ = stdin.write_all(cmd.as_bytes()).await;
            let _ = stdin.flush().await;
        }

        // Whole-test timeout (not per-line). Reads NDJSON lines until `Done`.
        let read_fut = async {
            let mut messages = Vec::new();
            let mut line = String::new();
            loop {
                line.clear();
                let n = self.stdout.read_line(&mut line).await?;
                if n == 0 {
                    return Err(anyhow::anyhow!("EOF"));
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Message>(trimmed) {
                    Ok(Message::Done) => {
                        messages.push(Message::Done);
                        return Ok(messages);
                    }
                    Ok(Message::Event(e)) => {
                        // Worker-startup hook failures use a sentinel file
                        // name. Surface them as a hard error so the pool
                        // can poison itself; never push them as regular
                        // events.
                        if e.file == "<worker_startup>" {
                            let msg = e.message.clone().unwrap_or_default();
                            return Err(anyhow::anyhow!("WORKER_STARTUP_FAILED: {msg}"));
                        }
                        messages.push(Message::Event(e));
                    }
                    Ok(msg) => messages.push(msg),
                    Err(_) => continue,
                }
            }
        };

        let result = if self.timeout.is_zero() {
            Ok(read_fut.await)
        } else {
            timeout(self.timeout, read_fut).await
        };
        match result {
            Ok(Ok(msgs)) => Ok(msgs),
            Ok(Err(e)) => {
                // Propagate worker_startup sentinel verbatim so the pool
                // can detect it and poison itself.
                let msg = e.to_string();
                if msg.starts_with("WORKER_STARTUP_FAILED:") {
                    return Err(e);
                }
                let stderr_tail = self
                    .stderr_buf
                    .lock()
                    .ok()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                if stderr_tail.is_empty() {
                    bail!(
                        "Subprocess exited unexpectedly while running {} ({})",
                        path_str,
                        e
                    );
                } else {
                    bail!(
                        "Subprocess exited unexpectedly while running {}\n--- stderr ---\n{}",
                        path_str,
                        stderr_tail
                    );
                }
            }
            Err(_) => {
                self.kill().await;
                bail!(
                    "Timeout after {}s running {}",
                    self.timeout.as_secs(),
                    path_str
                );
            }
        }
    }

    /// Mutable access to stdin for writing file paths.
    pub fn stdin_mut(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }

    /// Spawn a fork-mode parent process. Sets `SCRUTIN_TCP_PORT` so the
    /// runner forks children that connect back via TCP.
    pub async fn spawn_fork_parent(
        pkg: &Package,
        suite: &TestSuite,
        timeout: Duration,
        tcp_port: u16,
        log: Option<LogBuffer>,
    ) -> Result<Self> {
        Self::spawn_inner(pkg, suite, timeout, Some(tcp_port), log).await
    }

    pub async fn kill(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        if let Some(t) = self.stderr_task.take() {
            t.abort();
        }
    }
}

impl Drop for RProcess {
    fn drop(&mut self) {
        // kill_on_drop handles the child; just abort the stderr task.
        if let Some(t) = self.stderr_task.take() {
            t.abort();
        }
    }
}

/// Resolve the runner-script contents for a suite.
///
/// Precedence (first hit wins):
///   1. Explicit `[[suite]].runner = "path"` in config (captured on the
///      suite as `runner_override`). Read verbatim.
///   2. Project override at `<root>/.scrutin/runners/<tool>.<ext>`
///      (where `scrutin init` writes editable defaults). Read verbatim.
///   3. The embedded default baked into the binary.
fn resolve_runner_contents<'a>(
    pkg: &Package,
    suite: &'a TestSuite,
) -> Result<std::borrow::Cow<'a, str>> {
    let plugin = &suite.plugin;
    if let Some(override_path) = &suite.runner_override {
        let abs = absolute_under(&pkg.root, override_path);
        let contents = std::fs::read_to_string(&abs).with_context(|| {
            format!(
                "failed to read custom runner script for {}: {}",
                plugin.name(),
                abs.display()
            )
        })?;
        return Ok(std::borrow::Cow::Owned(contents));
    }
    let project_override = pkg
        .root
        .join(".scrutin")
        .join("runners")
        .join(plugin.runner_filename());
    match std::fs::read_to_string(&project_override) {
        Ok(s) => return Ok(std::borrow::Cow::Owned(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e).with_context(|| {
                format!(
                    "failed to read project runner override for {}: {}",
                    plugin.name(),
                    project_override.display()
                )
            });
        }
    }
    Ok(std::borrow::Cow::Borrowed(plugin.runner_script()))
}

/// Write the selected runner contents to a per-project cache dir and
/// return its absolute path. The cache lives under the OS cache dir
/// (falling back to the system temp dir), keyed on a hash of the
/// project root so concurrent scrutin invocations on different
/// projects never clobber each other.
fn materialise_runner(project_root: &Path, plugin: &dyn Plugin, contents: &str) -> Result<PathBuf> {
    let base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    let project_key = format!(
        "{:016x}",
        xxh64(project_root.to_string_lossy().as_bytes(), 0)
    );
    let dir = base.join("scrutin").join("runners").join(project_key);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating runner cache directory {}", dir.display()))?;
    let path = dir.join(plugin.runner_filename());
    std::fs::write(&path, contents)
        .with_context(|| format!("writing runner cache file {}", path.display()))?;
    Ok(path)
}
