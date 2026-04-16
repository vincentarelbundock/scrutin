//! Read-only GETs: the full snapshot for a fresh client, plus the
//! lighter per-resource lookups (`/api/files`, `/api/file/{id}`,
//! `/api/suites`, `/api/config`).

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use serde::Deserialize;

use scrutin_core::analysis::deps::build_reverse_dep_map;

use crate::state::{AppState, sorted_files};
use crate::wire::{
    FileId, WireFile, WirePackage, WireSnapshot, WireSuite, WireSuiteAction,
};
// WireFilterGroup is re-exported via crate root; not re-imported here.

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/snapshot", get(snapshot))
        .route("/api/files", get(files))
        .route("/api/file/{id}", get(file_by_id))
        .route("/api/file/{id}/source", get(file_source))
        .route("/api/file/{id}/source-for", get(source_for_test))
        .route("/api/suites", get(suites))
        .route("/api/keymap", get(keymap))
        .route("/api/config", get(config))
        .route("/syntect.css", get(syntect_css))
}

async fn syntect_css() -> axum::response::Response {
    use axum::http::{HeaderValue, StatusCode, header};
    use axum::response::IntoResponse;
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8")),
            (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600")),
        ],
        crate::highlight::theme_css(),
    )
        .into_response()
}

async fn snapshot(State(state): State<AppState>) -> Json<WireSnapshot> {
    let pkg = package_of(&state);
    let files = {
        let fmap = state.files.read().await;
        sorted_files(&fmap)
    };
    let summary = state.summary.read().await.clone();
    let watching = *state.watch.read().await;
    Json(WireSnapshot {
        pkg,
        files,
        current_run: summary,
        watching,
        n_workers: state.n_workers,
        keymap: serde_json::from_str(&scrutin_core::keymap::keymap_json()).unwrap_or_default(),
        outcome_order: scrutin_core::engine::protocol::Outcome::all_by_rank()
            .into_iter()
            .map(Into::into)
            .collect(),
        groups: (*state.groups).clone(),
        active_group: state.active_group.clone(),
    })
}

async fn files(State(state): State<AppState>) -> Json<Vec<WireFile>> {
    let fmap = state.files.read().await;
    Json(sorted_files(&fmap))
}

async fn file_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WireFile>, StatusCode> {
    let id: FileId = id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let fmap = state.files.read().await;
    fmap.get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn suites(State(state): State<AppState>) -> Json<Vec<WireSuite>> {
    Json(package_of(&state).suites)
}

#[derive(Deserialize)]
struct SourceQuery {
    #[serde(default)]
    line: Option<u32>,
    #[serde(default = "default_context")]
    context: u32,
}
fn default_context() -> u32 {
    20
}

/// Serve a snippet of a test file's source around `line`, or the whole
/// file (capped at 500 lines) when `line` is absent. Returns
/// `{ start_line, lines: [...] }` — the client renders it with a gutter.
async fn file_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SourceQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id: FileId = id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let path = {
        let fmap = state.files.read().await;
        let f = fmap.get(&id).ok_or(StatusCode::NOT_FOUND)?;
        state.pkg.root.join(&f.path)
    };
    // `file_id` is a trusted key into the server-side file map, so no
    // containment check against pkg.root is needed. In file-mode pkg.root
    // is a scratch tempdir and the real file lives elsewhere, which made
    // the old starts_with guard reject every legitimate request.
    let canon = std::fs::canonicalize(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let root = std::fs::canonicalize(&state.pkg.root).ok();
    let content = std::fs::read_to_string(&canon).map_err(|_| StatusCode::NOT_FOUND)?;
    let total = content.lines().count();
    let (start, end) = match q.line {
        Some(l) if l > 0 => {
            let target = (l as usize).saturating_sub(1);
            let ctx = q.context as usize;
            let target = target.min(total.saturating_sub(1));
            let s = target.saturating_sub(ctx);
            let e = (target + ctx + 1).min(total);
            (s, e)
        }
        _ => (0, total.min(500)),
    };
    let ext = canon.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lines = crate::highlight::highlight_slice(ext, &content, start, end);
    let display = match &root {
        Some(r) => canon.strip_prefix(r).unwrap_or(&canon).to_string_lossy(),
        None => canon.to_string_lossy(),
    };
    Ok(Json(serde_json::json!({
        "start_line": start + 1,
        "lines": lines,
        "highlight_line": q.line,
        "path": display,
    })))
}

/// Resolve the source file(s) that a test file depends on via the dep map.
/// Returns `{ source_path, lines, start_line, highlight_line }` for the
/// best-matching source file, or 404 if no mapping exists.
async fn source_for_test(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SourceQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id: FileId = id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let test_name = {
        let fmap = state.files.read().await;
        let f = fmap.get(&id).ok_or(StatusCode::NOT_FOUND)?;
        f.name.clone()
    };

    // Build the reverse dep map and find the best source match.
    let dep_map = state.dep_map.read().await;
    let reverse = build_reverse_dep_map(&dep_map);
    let sources = reverse.get(&test_name);
    let best = sources.and_then(|srcs| {
        let test_stem = test_name
            .strip_prefix("test-")
            .or_else(|| test_name.strip_prefix("test_"))
            .and_then(|s| {
                s.strip_suffix(".R")
                    .or_else(|| s.strip_suffix(".r"))
                    .or_else(|| s.strip_suffix(".py"))
            })
            .unwrap_or("");
        srcs.iter()
            .find(|s| {
                let src_stem = std::path::Path::new(s)
                    .file_stem()
                    .and_then(|f| f.to_str())
                    .unwrap_or("");
                src_stem.eq_ignore_ascii_case(test_stem)
            })
            .or_else(|| srcs.first())
    });

    let source_rel = best.ok_or(StatusCode::NOT_FOUND)?;
    let abs_path = state.pkg.root.join(source_rel);
    let canon = std::fs::canonicalize(&abs_path).map_err(|_| StatusCode::NOT_FOUND)?;
    let root = std::fs::canonicalize(&state.pkg.root).map_err(|_| StatusCode::NOT_FOUND)?;
    if !canon.starts_with(&root) {
        return Err(StatusCode::FORBIDDEN);
    }
    let content = std::fs::read_to_string(&canon).map_err(|_| StatusCode::NOT_FOUND)?;
    let total = content.lines().count();
    let (start, end) = match q.line {
        Some(l) if l > 0 => {
            let target = (l as usize).saturating_sub(1);
            let ctx = q.context as usize;
            let target = target.min(total.saturating_sub(1));
            let s = target.saturating_sub(ctx);
            let e = (target + ctx + 1).min(total);
            (s, e)
        }
        _ => (0, total.min(500)),
    };
    let ext = canon.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lines = crate::highlight::highlight_slice(ext, &content, start, end);
    Ok(Json(serde_json::json!({
        "start_line": start + 1,
        "lines": lines,
        "highlight_line": q.line,
        "path": canon.strip_prefix(&root).unwrap_or(&canon).to_string_lossy(),
    })))
}

async fn keymap() -> Json<serde_json::Value> {
    let raw = scrutin_core::keymap::keymap_json();
    let val: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
    Json(val)
}

async fn config(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Minimal config echo for v1: enough to show in a "config" pane
    // without exposing anything secret (scrutin has no secrets, but
    // stay conservative).
    Json(serde_json::json!({
        "pkg_name": state.pkg.name,
        "root": state.pkg.root.to_string_lossy(),
        "tool": state.pkg.tool_names(),
        "n_workers": state.n_workers,
    }))
}

/// Translate the core `Package` into a `WirePackage`. Pure function;
/// called from both snapshot and suites handlers.
fn package_of(state: &AppState) -> WirePackage {
    let suites: Vec<WireSuite> = state
        .pkg
        .test_suites
        .iter()
        .map(|s| {
            let actions: Vec<WireSuiteAction> = s
                .plugin
                .actions()
                .iter()
                .map(|a| {
                    use scrutin_core::project::plugin::ActionScope;
                    WireSuiteAction {
                        name: a.name.to_string(),
                        key: a.key.to_string(),
                        label: a.label.to_string(),
                        scope: match a.scope {
                            ActionScope::File => "file".into(),
                            ActionScope::All => "all".into(),
                        },
                    }
                })
                .collect();
            WireSuite {
                name: s.plugin.name().to_string(),
                language: s.plugin.language().to_string(),
                test_dirs: s
                    .run_search_dirs()
                    .iter()
                    .map(|td| {
                        td.strip_prefix(&state.pkg.root)
                            .unwrap_or(td)
                            .to_string_lossy()
                            .to_string()
                    })
                    .collect(),
                source_dir: s
                    .watch_search_dirs()
                    .first()
                    .map(|d| {
                        d.strip_prefix(&state.pkg.root)
                            .unwrap_or(d)
                            .to_string_lossy()
                            .to_string()
                    }),
                file_count: 0,
                actions,
            }
        })
        .collect();
    WirePackage {
        name: state.pkg.name.clone(),
        root: state.pkg.root.to_string_lossy().to_string(),
        tool: state.pkg.tool_names(),
        suites,
    }
}
