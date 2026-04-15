//! Control endpoints: start / rerun / rerun-failing / cancel / watch.
//!
//! All mutations route through `AppState::spawn_run` so there's exactly
//! one place that decides "cancel the previous run, start a new one,
//! spin up the forwarder."

use std::path::PathBuf;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::post;
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use crate::wire::{FileId, RunId};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/run", post(run_all))
        .route("/api/rerun", post(rerun))
        .route("/api/rerun-failing", post(rerun_failing))
        .route("/api/cancel", post(cancel))
        .route("/api/watch", post(watch))
        .route("/api/open-editor", post(open_editor))
        .route("/api/suite-action", post(suite_action))
        .route("/api/correction", post(correction_action))
}

#[derive(Deserialize)]
struct RunBody {
    #[serde(default)]
    files: Option<Vec<String>>, // file_id hex strings
}

#[derive(Serialize)]
struct RunResponse {
    run_id: RunId,
}

#[derive(Serialize)]
struct ErrResp {
    error: String,
}


async fn run_all(
    State(state): State<AppState>,
) -> Result<Json<RunResponse>, (StatusCode, Json<ErrResp>)> {
    // Run every file currently known to the server.
    let files: Vec<PathBuf> = {
        let fmap = state.files.read().await;
        fmap.values()
            .map(|f| state.pkg.root.join(&f.path))
            .collect()
    };
    do_spawn(state, files).await
}

async fn rerun(
    State(state): State<AppState>,
    Json(body): Json<RunBody>,
) -> Result<Json<RunResponse>, (StatusCode, Json<ErrResp>)> {
    let Some(ids) = body.files else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrResp {
                error: "missing `files`".into(),
            }),
        ));
    };
    let parsed: Vec<FileId> = ids.iter().filter_map(|s| s.parse().ok()).collect();
    let files: Vec<PathBuf> = {
        let fmap = state.files.read().await;
        parsed
            .iter()
            .filter_map(|id| fmap.get(id))
            .map(|f| state.pkg.root.join(&f.path))
            .collect()
    };
    if files.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrResp {
                error: "no matching files".into(),
            }),
        ));
    }
    do_spawn(state, files).await
}

async fn rerun_failing(
    State(state): State<AppState>,
) -> Result<Json<RunResponse>, (StatusCode, Json<ErrResp>)> {
    let bad_ids: Vec<FileId> = {
        let s = state.summary.read().await;
        s.bad_files.clone()
    };
    let files: Vec<PathBuf> = {
        let fmap = state.files.read().await;
        bad_ids
            .iter()
            .filter_map(|id| fmap.get(id))
            .map(|f| state.pkg.root.join(&f.path))
            .collect()
    };
    if files.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrResp {
                error: "no failing files".into(),
            }),
        ));
    }
    do_spawn(state, files).await
}

async fn cancel(State(state): State<AppState>) -> StatusCode {
    state.cancel_all().await;
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct WatchBody {
    enabled: bool,
}

async fn watch(
    State(state): State<AppState>,
    Json(body): Json<WatchBody>,
) -> StatusCode {
    let mut w = state.watch.write().await;
    *w = body.enabled;
    StatusCode::NO_CONTENT
}

async fn do_spawn(
    state: AppState,
    files: Vec<PathBuf>,
) -> Result<Json<RunResponse>, (StatusCode, Json<ErrResp>)> {
    match state.spawn_run(files).await {
        Ok(run_id) => Ok(Json(RunResponse { run_id })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrResp {
                error: e.to_string(),
            }),
        )),
    }
}

#[derive(Deserialize)]
struct OpenEditorBody {
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
}

#[derive(Serialize)]
struct OpenEditorResponse {
    opened: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

async fn open_editor(
    State(state): State<AppState>,
    Json(body): Json<OpenEditorBody>,
) -> Result<Json<OpenEditorResponse>, (StatusCode, Json<ErrResp>)> {
    // By `file_id`: the file is already in the trusted file map; its
    // stored path is authoritative (absolute in file-mode, relative in
    // project mode). By raw `path`: untrusted client input, must be
    // confined under the project root to prevent traversal.
    let (abs, trusted) = if let Some(ref path) = body.path {
        (state.pkg.root.join(path), false)
    } else if let Some(ref file_id) = body.file_id {
        let fid: crate::wire::FileId = file_id
            .parse()
            .map_err(|_| bad("invalid file_id"))?;
        let rel_path = {
            let fmap = state.files.read().await;
            let f = fmap.get(&fid).ok_or_else(|| bad("unknown file_id"))?;
            f.path.clone()
        };
        (state.pkg.root.join(&rel_path), true)
    } else {
        return Err(bad("file_id or path required"));
    };
    let canon = std::fs::canonicalize(&abs).map_err(|_| bad("file not found"))?;
    if !trusted {
        let root =
            std::fs::canonicalize(&state.pkg.root).map_err(|_| bad("root not found"))?;
        if !canon.starts_with(&root) {
            return Err(bad("path escapes project root"));
        }
    }

    let path_str = canon.to_string_lossy().to_string();
    let (argv, skipped_terminal_editor) = pick_editor_argv(&state, &path_str, body.line);
    let label = argv.first().cloned().unwrap_or_else(|| "system default".into());

    let mut cmd = std::process::Command::new(&argv[0]);
    for a in &argv[1..] {
        cmd.arg(a);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map_err(|e| bad(&format!("failed to launch {label}: {e}")))?;

    let hint = skipped_terminal_editor.map(|name| {
        format!(
            "$EDITOR={name} is a terminal editor and can't attach to the browser; \
             fell back to the system default. Set [web].editor in .scrutin/config.toml \
             (e.g. \"code\") to pick a GUI editor."
        )
    });

    Ok(Json(OpenEditorResponse {
        opened: label,
        hint,
    }))
}

/// Pick the argv for "open file in editor". Returns `(argv, skipped)` where
/// `skipped` names a terminal editor we had to pass over (so the caller can
/// show a hint); it is `None` when an editor was successfully selected
/// without needing to skip one.
fn pick_editor_argv(
    state: &AppState,
    path_str: &str,
    line: Option<u32>,
) -> (Vec<String>, Option<String>) {
    // 1. Explicit config override — wins over everything.
    if let Some(ref editor) = state.editor {
        let tokens: Vec<String> = editor.split_whitespace().map(String::from).collect();
        if !tokens.is_empty() {
            return (build_editor_argv(&tokens, path_str, line), None);
        }
    }

    // 2. $VISUAL then $EDITOR — skip either one if it's a terminal editor
    //    that can't attach to the browser.
    let mut skipped: Option<String> = None;
    for var in ["VISUAL", "EDITOR"] {
        let Ok(raw) = std::env::var(var) else { continue };
        let tokens: Vec<String> = raw.split_whitespace().map(String::from).collect();
        let Some(first) = tokens.first() else { continue };
        if is_terminal_editor(first) {
            skipped.get_or_insert_with(|| first.clone());
            continue;
        }
        return (build_editor_argv(&tokens, path_str, line), None);
    }

    // 3. OS-native default.
    (os_default_argv(path_str), skipped)
}

fn build_editor_argv(tokens: &[String], path_str: &str, line: Option<u32>) -> Vec<String> {
    let mut argv: Vec<String> = tokens.to_vec();
    let basename = std::path::Path::new(&tokens[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&tokens[0]);
    match basename {
        "code" | "code-insiders" | "positron" => {
            if let Some(line) = line {
                argv.push("--goto".into());
                argv.push(format!("{path_str}:{line}"));
            } else {
                argv.push(path_str.to_string());
            }
        }
        _ => {
            if let Some(line) = line {
                argv.push(format!("+{line}"));
            }
            argv.push(path_str.to_string());
        }
    }
    argv
}

fn os_default_argv(path_str: &str) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        vec!["open".into(), path_str.into()]
    }
    #[cfg(target_os = "linux")]
    {
        vec!["xdg-open".into(), path_str.into()]
    }
    #[cfg(target_os = "windows")]
    {
        // `start` is a cmd builtin, not an exe — wrap with `cmd /C`.
        // The empty "" arg is the window title placeholder that `start`
        // expects when the first real arg is quoted.
        vec!["cmd".into(), "/C".into(), "start".into(), "".into(), path_str.into()]
    }
}

/// Editors that require a tty. If `$EDITOR` is one of these, the standalone
/// web server can't launch it usefully (no tty to attach to) and we fall
/// through to the OS-native "open with default app" with a hint to set
/// `[web].editor` explicitly.
fn is_terminal_editor(editor: &str) -> bool {
    let name = std::path::Path::new(editor)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(editor);
    matches!(
        name,
        "vim" | "nvim" | "vi" | "ex" | "view"
            | "emacs" | "emacsclient"
            | "nano" | "pico"
            | "helix" | "hx"
            | "micro"
            | "kak" | "kakoune"
            | "joe" | "jmacs" | "jstar"
            | "ne" | "mg" | "ed"
    )
}

#[derive(Deserialize)]
struct PluginActionBody {
    file_id: String,
    action: String,
}

async fn suite_action(
    State(state): State<AppState>,
    Json(body): Json<PluginActionBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrResp>)> {
    let file_id: crate::wire::FileId = body
        .file_id
        .parse()
        .map_err(|_| bad("invalid file_id"))?;
    let rel_path = {
        let fmap = state.files.read().await;
        let f = fmap.get(&file_id).ok_or_else(|| bad("unknown file_id"))?;
        f.path.clone()
    };
    // `rel_path` came from the trusted file map (keyed by file_id we
    // just validated), so no traversal check is needed. In file-mode the
    // path is absolute and lives outside `pkg.root` (a tempdir); in
    // project mode it's a root-relative string.
    let abs = state.pkg.root.join(&rel_path);
    let _ = std::fs::canonicalize(&abs).map_err(|_| bad("file not found"))?;

    // `suite_for` is the single source of truth: whichever suite owns
    // the targeted file decides both the action lookup and the spawn
    // cwd. Using the file-map's cached `suite` field would let those
    // drift if the cache lagged behind a config reload.
    let owning_suite = state
        .pkg
        .suite_for(&abs)
        .ok_or_else(|| bad(&format!("no suite owns file {}", rel_path)))?;
    let suite_name = owning_suite.plugin.name().to_string();
    let plugin_action = owning_suite
        .plugin
        .actions()
        .into_iter()
        .find(|a| a.name == body.action)
        .ok_or_else(|| bad(&format!("unknown action {:?} for suite {:?}", body.action, suite_name)))?;
    let action_cwd = owning_suite.root.clone();

    use scrutin_core::project::plugin::ActionScope;

    // Collect relative paths for the target files. Canonical paths are used
    // for the command args; root-joined relative paths for spawn_run (so
    // FileId computation matches).
    let rel_paths: Vec<String> = match plugin_action.scope {
        ActionScope::File => vec![rel_path],
        ActionScope::All => {
            let fmap = state.files.read().await;
            fmap.values()
                .filter(|f| f.suite == suite_name)
                .map(|f| f.path.clone())
                .collect()
        }
    };

    let mut cmd = std::process::Command::new(&plugin_action.command[0]);
    for arg in &plugin_action.command[1..] {
        cmd.arg(arg);
    }
    for rp in &rel_paths {
        cmd.arg(state.pkg.root.join(rp));
    }
    cmd.current_dir(&action_cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let status = cmd
        .status()
        .map_err(|e| bad(&format!("failed to run {}: {e}", plugin_action.command[0])))?;

    let rerun = plugin_action.rerun && status.success();
    if rerun {
        let files: Vec<std::path::PathBuf> = rel_paths
            .iter()
            .map(|rp| state.pkg.root.join(rp))
            .collect();
        let _ = state.spawn_run(files).await;
    }

    Ok(Json(serde_json::json!({
        "success": status.success(),
        "rerun": rerun,
    })))
}

#[derive(Deserialize)]
struct CorrectionBody {
    file_id: String,
    word: String,
    line: u32,
    col_start: u32,
    col_end: u32,
    /// Non-empty replacement → accept that suggestion. When `None`, the
    /// request is a whitelist-add: `skyspell add` the word instead of
    /// editing the file.
    #[serde(default)]
    replacement: Option<String>,
}

/// Spell-check correction: either replace a misspelled word with a chosen
/// suggestion (by rewriting the file in place) or whitelist the word via
/// `skyspell add`. Triggers a rerun of the file so the refreshed findings
/// drop the handled entry.
async fn correction_action(
    State(state): State<AppState>,
    Json(body): Json<CorrectionBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrResp>)> {
    use scrutin_core::engine::protocol::Correction;
    use scrutin_core::prose::skyspell;

    let file_id: FileId = body
        .file_id
        .parse()
        .map_err(|_| bad("invalid file_id"))?;

    let (abs_path, suite_root) = {
        let fmap = state.files.read().await;
        let f = fmap.get(&file_id).ok_or_else(|| bad("unknown file_id"))?;
        let abs = state.pkg.root.join(&f.path);
        let suite_root = state
            .pkg
            .test_suites
            .iter()
            .find(|s| s.plugin.name() == f.suite)
            .map(|s| s.root.clone())
            .unwrap_or_else(|| state.pkg.root.clone());
        (abs, suite_root)
    };

    let correction = Correction {
        word: body.word.clone(),
        line: body.line,
        col_start: body.col_start,
        col_end: body.col_end,
        suggestions: Vec::new(),
    };

    let scope_label = if let Some(replacement) = body.replacement.as_deref() {
        skyspell::apply_correction_to_file(&abs_path, &correction, replacement)
            .map_err(|e| bad(&format!("apply failed: {}", e)))?;
        format!("replaced with {:?}", replacement)
    } else {
        let scope = skyspell::add_word_to_dict(
            &suite_root,
            &state.pkg.skyspell_extra_args,
            &state.pkg.skyspell_add_args,
            &body.word,
        )
        .map_err(|e| bad(&format!("add failed: {}", e)))?;
        match scope {
            skyspell::AddScope::Project(path) => format!("added to {}", path.display()),
            skyspell::AddScope::Global => "added to skyspell global dictionary".into(),
        }
    };

    // Re-run the single file so the server-side findings refresh and the
    // browser sees the warning disappear.
    let _ = state.spawn_run(vec![abs_path]).await;

    Ok(Json(serde_json::json!({
        "success": true,
        "message": scope_label,
    })))
}

fn bad(msg: &str) -> (StatusCode, Json<ErrResp>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrResp {
            error: msg.to_string(),
        }),
    )
}
