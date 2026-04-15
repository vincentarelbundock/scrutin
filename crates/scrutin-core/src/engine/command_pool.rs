//! Lightweight command-mode pool for plugins that run a one-shot external
//! command per file (linters like jarl, ruff). No long-lived subprocess,
//! no stdin/stdout protocol, no respawn/poison logic.
//!
//! Shares [`CancelHandle`] and [`BusyCounter`] with [`super::pool::ProcessPool`]
//! so multi-suite runs get unified cancellation and busy counting.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

use crate::engine::pool::{BusyCounter, CancelHandle};
use crate::engine::protocol::{Event, Message};
use crate::engine::run_events::FileResult;
use crate::logbuf::LogBuffer;
use crate::project::package::{Package, TestSuite};

pub struct CommandPool {
    pkg: Arc<Package>,
    suite: Arc<TestSuite>,
    timeout: Duration,
    concurrency: Arc<Semaphore>,
    busy: BusyCounter,
    cancel: CancelHandle,
    log: Option<LogBuffer>,
}

impl CommandPool {
    pub fn new(
        pkg: &Package,
        suite: &TestSuite,
        n_workers: usize,
        timeout: Duration,
        log: Option<LogBuffer>,
        cancel: CancelHandle,
        busy: BusyCounter,
    ) -> Self {
        CommandPool {
            pkg: Arc::new(pkg.clone()),
            suite: Arc::new(suite.clone()),
            timeout,
            concurrency: Arc::new(Semaphore::new(n_workers)),
            busy,
            cancel,
            log,
        }
    }

    pub async fn run_tests(&self, test_files: &[PathBuf], tx: mpsc::UnboundedSender<FileResult>) {
        let mut set: JoinSet<()> = JoinSet::new();

        for test_file in test_files {
            let concurrency = self.concurrency.clone();
            let busy = self.busy.clone();
            let cancel = self.cancel.clone();
            let pkg = self.pkg.clone();
            let suite = self.suite.clone();
            let timeout = self.timeout;
            let log = self.log.clone();
            let tx = tx.clone();
            let file = test_file.clone();

            set.spawn(async move {
                let permit = match concurrency.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return,
                };

                if cancel.is_file_cancelled(&file) {
                    drop(permit);
                    let _ = tx.send(cancelled_result(&file));
                    return;
                }

                busy.inc();
                let messages = run_command(&pkg, &suite, &file, timeout, log.as_ref()).await;
                busy.dec();
                drop(permit);

                let _ = tx.send(FileResult {
                    file,
                    messages,
                    cancelled: false,
                });
            });
        }

        while let Some(joined) = set.join_next().await {
            if let Err(e) = joined
                && e.is_panic()
                && let Some(ref lb) = self.log
            {
                lb.push(
                    self.suite.plugin.name(),
                    &format!("command task panicked: {e}\n"),
                );
            }
        }
    }
}

async fn run_command(
    pkg: &Package,
    suite: &TestSuite,
    test_file: &PathBuf,
    timeout: Duration,
    log: Option<&LogBuffer>,
) -> Vec<Message> {
    let plugin = &suite.plugin;
    let spec = match plugin.command_spec(&suite.root, pkg) {
        Some(s) => s,
        None => {
            return vec![Message::Event(Event::engine_error(
                file_basename(test_file),
                "internal: command_spec() returned None for command-mode plugin",
            ))];
        }
    };

    let mut cmd = tokio::process::Command::new(&spec.argv[0]);
    cmd.args(&spec.argv[1..]);
    cmd.arg(test_file);
    cmd.current_dir(&suite.root);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    for (k, v) in plugin.env_vars(&suite.root) {
        cmd.env(k, v);
    }
    // User [env] from .scrutin/config.toml, applied last so user vars win.
    cmd.envs(pkg.env.iter());

    let t0 = std::time::Instant::now();
    let result = if timeout.is_zero() {
        Ok(cmd.output().await)
    } else {
        tokio::time::timeout(timeout, cmd.output()).await
    };
    let duration_ms = t0.elapsed().as_millis() as u64;

    let file_name = file_basename(test_file);

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Mirror non-empty stderr to the log buffer.
            if let Some(lb) = log {
                let name = plugin.name();
                for line in stderr.lines() {
                    if !line.is_empty() && !plugin.is_noise_line(line) {
                        lb.push(name, &format!("{line}\n"));
                    }
                }
            }

            plugin.parse_command_output(
                &file_name,
                &stdout,
                &stderr,
                output.status.code(),
                duration_ms,
            )
        }
        Ok(Err(e)) => {
            vec![
                Message::Event(Event::engine_error(
                    &file_name,
                    format!(
                        "failed to spawn {} ({}). Is it installed and on PATH?",
                        spec.argv[0], e
                    ),
                )),
                Message::Done,
            ]
        }
        Err(_) => {
            vec![
                Message::Event(Event::engine_error(
                    &file_name,
                    format!("timeout after {}s", timeout.as_secs()),
                )),
                Message::Done,
            ]
        }
    }
}

fn file_basename(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn cancelled_result(file: &std::path::Path) -> FileResult {
    FileResult {
        file: file.to_path_buf(),
        messages: vec![Message::Done],
        cancelled: true,
    }
}
