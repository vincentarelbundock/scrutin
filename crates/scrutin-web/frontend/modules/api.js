// All HTTP + editor-bridge I/O. Keeps the network surface in one file so
// swapping transports (e.g. adding a retry layer) is localized.

import { state, BASE, IS_VSCODE, vscode, setOutcomeRanks } from "./state.js";
import { toast } from "./util.js";
import { setStatus } from "./render.js";

export async function postJSON(path, body) {
  try {
    const res = await fetch(`${BASE}${path}`, {
      method: "POST",
      headers: body ? { "Content-Type": "application/json" } : {},
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      const txt = await res.text();
      toast(`${path} \u2192 ${res.status}: ${txt}`, true);
      return null;
    }
    return await res.json().catch(() => ({}));
  } catch (e) {
    toast(`${path} failed: ${e}`, true);
    return null;
  }
}

/// Hydrate state from /api/snapshot. Called once at boot.
export async function fetchSnapshot() {
  try {
    const res = await fetch(`${BASE}/api/snapshot`);
    if (!res.ok) throw new Error(`snapshot http ${res.status}`);
    const snap = await res.json();
    state.pkg = snap.pkg;
    state.files = new Map();
    state.fileOrder = [];
    for (const f of snap.files) {
      state.files.set(f.id, f);
      state.fileOrder.push(f.id);
    }
    state.currentRun = snap.current_run;
    state.watching = snap.watching;
    state.nWorkers = snap.n_workers;
    state.totals = snap.current_run?.totals ?? state.totals;
    state.keymap = snap.keymap ?? [];
    // Replace the local outcome rank from the server's authoritative
    // order (core::Outcome::rank()).
    if (Array.isArray(snap.outcome_order)) {
      const m = {};
      snap.outcome_order.forEach((o, i) => { m[o] = i; });
      setOutcomeRanks(m);
    }
    if (!state.selected && state.fileOrder.length > 0) {
      state.selected = state.fileOrder[0];
    }
  } catch (e) {
    toast(`snapshot failed: ${e}`, true);
  }
}

/// Fetch a window of source around `line`. With line=null the endpoint
/// returns the top of the file, which is what we want for file-level
/// errors with no expectation location.
export async function fetchSource(fileId, line) {
  const hasLine = line != null;
  const key = hasLine ? `${fileId}:${line}` : `${fileId}:top`;
  if (state.sourceCache.has(key)) return state.sourceCache.get(key);
  try {
    const url = hasLine
      ? `${BASE}/api/file/${fileId}/source?line=${line}&context=8`
      : `${BASE}/api/file/${fileId}/source`;
    const res = await fetch(url);
    if (!res.ok) return null;
    const data = await res.json();
    state.sourceCache.set(key, data);
    return data;
  } catch (_) {
    return null;
  }
}

/// Fetch the dep-mapped production source for a test file.
export async function fetchSourceFor(fileId) {
  const key = `source-for:${fileId}`;
  if (state.sourceCache.has(key)) return state.sourceCache.get(key);
  try {
    const res = await fetch(`${BASE}/api/file/${fileId}/source-for`);
    if (!res.ok) return null;
    const data = await res.json();
    state.sourceCache.set(key, data);
    return data;
  } catch (_) {
    return null;
  }
}

// ── Run-control verbs ──────────────────────────────────────────────────────

export async function runAll() {
  setStatus("starting run\u2026");
  await postJSON("/api/run");
}

export async function runVisible() {
  if (state.filtered.length === 0) { toast("nothing visible to run", true); return; }
  if (state.currentRun?.in_progress) return;
  setStatus(`running ${state.filtered.length} visible file${state.filtered.length === 1 ? "" : "s"}\u2026`);
  await postJSON("/api/rerun", { files: state.filtered.map(String) });
}

export async function cancelRun() {
  if (!state.currentRun?.in_progress) return;
  setStatus("cancelling\u2026");
  await postJSON("/api/cancel");
}

export async function rerunFailing() {
  if (!(state.currentRun?.bad_files?.length)) return;
  setStatus("rerunning failing\u2026");
  await postJSON("/api/rerun-failing");
}

export async function runMultiSelected() {
  if (state.multiSelected.size === 0) return;
  const ids = [...state.multiSelected];
  setStatus(`running ${ids.length} selected file${ids.length === 1 ? "" : "s"}\u2026`);
  await postJSON("/api/rerun", { files: ids.map(String) });
}

export async function rerunSelected() {
  if (state.multiSelected.size > 0) { runMultiSelected(); return; }
  if (!state.selected) return;
  setStatus("rerunning selected\u2026");
  await postJSON("/api/rerun", { files: [String(state.selected)] });
}

export async function toggleWatch() {
  state.watching = !state.watching;
  // Caller re-renders header after awaiting.
  await postJSON("/api/watch", { enabled: state.watching });
}

export async function runPluginAction(actionName, fileId) {
  const id = fileId ?? state.selected;
  if (!id) return;
  const res = await postJSON("/api/suite-action", { file_id: id, action: actionName });
  if (res !== null) {
    const label = actionName.replace(/_/g, " ");
    toast(res.rerun ? `${label}: done, re-running` : `${label}: done`);
  }
}

/// Apply (or whitelist) a spell-check correction. Pass `replacement` to
/// accept a suggestion; omit it to whitelist the word via `skyspell add`.
export async function applyCorrection(fileId, correction, replacement) {
  const body = {
    file_id: String(fileId),
    word: correction.word,
    line: correction.line,
    col_start: correction.col_start,
    col_end: correction.col_end,
  };
  if (replacement != null) body.replacement = replacement;
  const res = await postJSON("/api/correction", body);
  if (res !== null) {
    toast(res.message ?? "correction applied");
  }
}

// ── Editor integrations (standalone POST vs VSCode webview postMessage) ──

export async function openInEditor(fileId, line) {
  const id = fileId ?? state.selected;
  if (!id) return;
  if (IS_VSCODE) {
    const f = state.files.get(id);
    if (!f) return;
    const root = state.pkg?.root ?? "";
    const absPath = f.path.startsWith("/") ? f.path : `${root}/${f.path}`;
    vscode.postMessage({ command: "openFile", path: absPath, line });
  } else {
    const body = { file_id: id };
    if (line != null) body.line = line;
    const res = await postJSON("/api/open-editor", body);
    if (res !== null) notifyOpened(res);
  }
}

export async function openSourceInEditor() {
  if (!state.selected) return;
  const src = await fetchSourceFor(state.selected);
  if (!src || !src.path) { toast("no source mapping found", true); return; }
  if (IS_VSCODE) {
    const root = state.pkg?.root ?? "";
    vscode.postMessage({ command: "openFile", path: `${root}/${src.path}` });
  } else {
    const root = state.pkg?.root ?? "";
    const body = { path: `${root}/${src.path}` };
    const res = await postJSON("/api/open-editor", body);
    if (res !== null) notifyOpened(res);
  }
}

function notifyOpened(res) {
  const opened = res?.opened ? `opened in ${res.opened}` : "opened";
  if (res?.hint) toast(`${opened} \u2014 ${res.hint}`, true);
  else toast(opened);
}
