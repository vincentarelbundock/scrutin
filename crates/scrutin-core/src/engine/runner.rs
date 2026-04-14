use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

use crate::engine::protocol::Message;
use crate::logbuf::LogBuffer;
use crate::project::package::{Package, TestSuite};

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

        // Materialize runner script under .scrutin/ so subprocess can load it.
        // If the suite has a user-provided runner override, read that file
        // instead of the embedded default.
        let scrutin_dir = pkg.root.join(".scrutin");
        std::fs::create_dir_all(&scrutin_dir)?;

        // R plugins source() a shared runner_r.R; write it alongside the
        // per-plugin script so the relative path resolves.
        if plugin.language() == "r" {
            std::fs::write(
                scrutin_dir.join("runner_r.R"),
                crate::r::R_RUNNER_SHARED,
            )?;
        }

        let runner_path = scrutin_dir.join(plugin.runner_basename());
        if let Some(override_path) = &suite.runner_override {
            let abs = if override_path.is_relative() {
                pkg.root.join(override_path)
            } else {
                override_path.clone()
            };
            let contents = std::fs::read_to_string(&abs).with_context(|| {
                format!(
                    "failed to read custom runner script for {}: {}",
                    plugin.name(),
                    abs.display()
                )
            })?;
            std::fs::write(&runner_path, contents)?;
        } else {
            std::fs::write(&runner_path, plugin.runner_script())?;
        }

        let mut argv = plugin.subprocess_cmd(&pkg.root);
        if argv.is_empty() {
            anyhow::bail!("tool {} returned empty subprocess command", plugin.name());
        }
        // For Python plugins, replace the auto-detected interpreter with
        // the user's [python].interpreter or [python].venv override.
        if plugin.language() == "python" && !pkg.python_interpreter.is_empty() {
            // argv is [interpreter, "-u", ".scrutin/runner.py", ...]
            // Replace just the first element with the override tokens.
            let tail: Vec<String> = argv.drain(1..).collect();
            argv = pkg.python_interpreter.clone();
            argv.extend(tail);
        }
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        for (k, v) in plugin.env_vars(&pkg.root) {
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
            .current_dir(&pkg.root)
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

        match timeout(self.timeout, read_fut).await {
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
