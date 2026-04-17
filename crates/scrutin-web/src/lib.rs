//! scrutin-web: browser-based live dashboard that shares the `run_events`
//! seam with the TUI.
//!
//! One tokio runtime hosts the axum HTTP server, the scrutin engine, and
//! (optionally) the file watcher. Localhost-only by design:
//! [`run_web`] binds to `127.0.0.1:<port>` and the loopback middleware in
//! [`server::require_loopback`] rejects anything else as defence in depth.
//!
//! See `docs/web-spec.md` for the full design rationale.

mod assets;
mod routes;
mod server;
mod state;
mod wire;
#[cfg(test)]
mod wire_tests;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use scrutin_core::project::package::Package;

pub use state::AppState;
pub use wire::WireFilterGroup;

/// Entry point: spin up the axum server, serve the embedded frontend, and
/// block until Ctrl-C. Called from `scrutin-bin/src/cli.rs` when the user
/// passes `--target web`.
#[allow(clippy::too_many_arguments)]
pub async fn run_web(
    addr: SocketAddr,
    pkg: Package,
    n_workers: usize,
    test_files: Vec<std::path::PathBuf>,
    watch: bool,
    timeout_file_ms: u64,
    timeout_run_ms: u64,
    fork_workers: bool,
    editor: Option<String>,
    groups: Vec<WireFilterGroup>,
    active_group: Option<String>,
) -> Result<()> {
    if !addr.ip().is_loopback() {
        anyhow::bail!(
            "scrutin refuses to bind to a non-loopback address ({addr}). \
             Bind to 127.0.0.1 or ::1."
        );
    }

    let state = AppState::new(Arc::new(pkg), test_files, n_workers, watch, timeout_file_ms, timeout_run_ms, fork_workers, editor, groups, active_group);

    // Build dep map eagerly so the watcher can resolve source→test edges.
    state.start_dep_map_build();

    // Start file watcher if watch mode is enabled.
    let _watcher_guard = if watch {
        Some(state.start_watcher(50)?)
    } else {
        None
    };

    let router = server::build_router(state.clone());

    // Try the requested port, then scan upward if occupied.
    let listener = {
        let max_attempts: u16 = 10;
        let mut port = addr.port();
        let mut last_err = None;
        let mut bound = None;
        for _ in 0..max_attempts {
            let try_addr = SocketAddr::new(addr.ip(), port);
            match tokio::net::TcpListener::bind(try_addr).await {
                Ok(l) => { bound = Some(l); break; }
                Err(e) => { last_err = Some(e); port += 1; }
            }
        }
        match bound {
            Some(l) => l,
            None => {
                return Err(last_err.unwrap())
                    .with_context(|| format!("failed to bind to any port in {}..{}", addr.port(), addr.port() + max_attempts));
            }
        }
    };
    let actual = listener.local_addr()?;
    eprintln!("scrutin-web: listening on http://{actual}");

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        eprintln!("scrutin-web: shutting down");
    };

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .context("axum::serve failed")?;

    // Cancel any in-flight run cleanly on shutdown.
    state.cancel_all().await;

    Ok(())
}