// Drill-down navigation (Files \u2192 Detail \u2192 Failure) and the global
// failure carousel. State-mutation functions; each ends by calling the
// render refresh.

import { state } from "./state.js";
import { isBadOutcome } from "./util.js";
import { updateTestFiltered } from "./sort.js";
import {
  renderLeftPane, renderRightPane, renderHints, renderControls,
} from "./render.js";

// ── Full re-render of the 3 "level-dependent" panes. ────────────────────
function refreshUi() {
  renderLeftPane();
  renderRightPane();
  renderHints();
}

// ── File selection (Files level cursor) ────────────────────────────────
export function selectFile(id) {
  state.selected = id;
  state.testCursor = 0;
  updateTestFiltered();
  document.querySelectorAll(".file-row.selected").forEach((el) => el.classList.remove("selected"));
  const li = document.querySelector(`.file-row[data-id="${id}"]`);
  if (li) li.classList.add("selected");
  renderRightPane();
}

export function moveFileSelection(delta) {
  if (state.filtered.length === 0) return;
  const idx = state.filtered.indexOf(state.selected);
  let next = idx + delta;
  if (!Number.isFinite(delta)) next = delta > 0 ? state.filtered.length - 1 : 0;
  next = Math.max(0, Math.min(state.filtered.length - 1, next));
  selectFile(state.filtered[next]);
  const li = document.querySelector(`.file-row[data-id="${state.selected}"]`);
  if (li) li.scrollIntoView({ block: "nearest" });
}

// ── Test cursor (Detail level) ─────────────────────────────────────────
export function moveTestSelection(delta) {
  if (state.testFiltered.length === 0) return;
  // Spill across files on unit steps (j/k/arrows) so the user can scroll
  // through every test in the run without exiting Detail view. Down past
  // the last test advances to the first test of the next visible file;
  // up past the first test wraps to the last test of the previous file.
  // Page/top/bottom jumps (±Infinity or larger deltas) stay file-scoped.
  if (delta === 1 && state.testCursor === state.testFiltered.length - 1) {
    const idx = state.filtered.indexOf(state.selected);
    if (idx >= 0 && idx + 1 < state.filtered.length) {
      selectFile(state.filtered[idx + 1]);
      renderLeftPane();
      const row = document.querySelector(`.test-row[data-idx="0"]`);
      if (row) row.scrollIntoView({ block: "nearest" });
      return;
    }
  }
  if (delta === -1 && state.testCursor === 0) {
    const idx = state.filtered.indexOf(state.selected);
    if (idx > 0) {
      selectFile(state.filtered[idx - 1]);
      state.testCursor = Math.max(0, state.testFiltered.length - 1);
      renderLeftPane();
      renderRightPane();
      const row = document.querySelector(`.test-row[data-idx="${state.testCursor}"]`);
      if (row) row.scrollIntoView({ block: "nearest" });
      return;
    }
  }
  let next = state.testCursor + delta;
  if (!Number.isFinite(delta)) next = delta > 0 ? state.testFiltered.length - 1 : 0;
  next = Math.max(0, Math.min(state.testFiltered.length - 1, next));
  state.testCursor = next;
  renderLeftPane();
  renderRightPane();
  const row = document.querySelector(`.test-row[data-idx="${next}"]`);
  if (row) row.scrollIntoView({ block: "nearest" });
}

/// Jump the test cursor to the next (or previous) bad-outcome test in the
/// currently selected file. Used by the `nav-next-failing` hotkey.
export function moveToNextFailing(delta) {
  if (state.testFiltered.length === 0) return;
  const dir = delta > 0 ? 1 : -1;
  let idx = state.testCursor;
  for (let i = 0; i < state.testFiltered.length; i++) {
    idx = (idx + dir + state.testFiltered.length) % state.testFiltered.length;
    if (isBadOutcome(state.testFiltered[idx])) {
      state.testCursor = idx;
      renderLeftPane();
      renderRightPane();
      const row = document.querySelector(`.test-row[data-idx="${idx}"]`);
      if (row) row.scrollIntoView({ block: "nearest" });
      return;
    }
  }
}

// ── Drill in/out: Files \u2192 Detail ─────────────────────────────────
export function enterDetail() {
  if (!state.selected) return;
  const f = state.files.get(state.selected);
  if (!f || !f.messages || f.messages.length === 0) return;
  state.level = "detail";
  updateTestFiltered();
  refreshUi();
}

export function exitDetail() {
  if (state.level !== "detail") return;
  state.level = "files";
  state.testCursor = 0;
  refreshUi();
}

// ── Failure view (global carousel across all bad events) ──────────────
//
// `state.failures` is a flat fail/error list rebuilt on entry, not kept
// in sync with run events. Simpler than a derived store; rebuild cost is
// trivial.

export function buildFailures() {
  const out = [];
  for (const id of state.fileOrder) {
    const f = state.files.get(id);
    if (!f || !f.messages) continue;
    for (const m of f.messages) {
      if (isBadOutcome(m)) {
        out.push({
          fileId: f.id,
          file: f.name,
          filePath: f.path,
          test: m.test_name ?? "<anon>",
          message: m.message ?? "",
          line: m.location?.line ?? null,
          outcome: m.outcome,
        });
      }
    }
  }
  state.failures = out;
}

export function enterFailure() {
  buildFailures();
  if (state.failures.length === 0) return;

  // If drilling in from Detail on a bad test, land on the matching failure
  // so the user's position survives the level change.
  let cursor = 0;
  if (state.level === "detail") {
    const m = state.testFiltered[state.testCursor];
    if (m && isBadOutcome(m)) {
      const name = m.test_name ?? "<anon>";
      const idx = state.failures.findIndex(
        (ff) => ff.fileId === state.selected && ff.test === name,
      );
      if (idx >= 0) cursor = idx;
    }
  }
  state.failureCursor = cursor;
  state.level = "failure";
  refreshUi();
}

export function exitFailure() {
  if (state.level !== "failure") return;
  // One level up means Detail for the current failure's owning file.
  const ff = state.failures[state.failureCursor];
  if (ff && state.files.get(ff.fileId)) {
    state.selected = ff.fileId;
    updateTestFiltered();
    const idx = state.testFiltered.findIndex(
      (m) => (m.test_name ?? "<anon>") === ff.test,
    );
    state.testCursor = idx >= 0 ? idx : 0;
    state.level = "detail";
  } else {
    state.level = "files";
  }
  refreshUi();
}

export function moveFailureSelection(delta) {
  if (state.failures.length === 0) return;
  let next = state.failureCursor + delta;
  if (!Number.isFinite(delta)) next = delta > 0 ? state.failures.length - 1 : 0;
  next = Math.max(0, Math.min(state.failures.length - 1, next));
  if (next === state.failureCursor) return;
  state.failureCursor = next;
  // Sync `selected` if the carousel walked across files.
  const ff = state.failures[state.failureCursor];
  if (ff && state.selected !== ff.fileId) state.selected = ff.fileId;
  renderLeftPane();
  renderRightPane();
}

/// Jump to a specific drill level, collapsing deeper levels one at a time
/// (so each exit function runs its cleanup). Used by clickable breadcrumb
/// segments.
export function jumpToLevel(targetLevel) {
  while (state.level !== targetLevel) {
    if (state.level === "failure") exitFailure();
    else if (state.level === "detail") exitDetail();
    else break;
  }
}

// ── Multi-selection (files level only) ─────────────────────────────────

export function toggleMultiSelect(id) {
  if (state.multiSelected.has(id)) state.multiSelected.delete(id);
  else state.multiSelected.add(id);
  state.lastClicked = id;
  const li = document.querySelector(`.file-row[data-id="${id}"]`);
  if (li) li.classList.toggle("multi-selected", state.multiSelected.has(id));
  renderControls();
}

export function rangeSelect(id) {
  const anchor = state.lastClicked;
  if (!anchor) { toggleMultiSelect(id); return; }
  const list = state.filtered;
  const a = list.indexOf(anchor);
  const b = list.indexOf(id);
  if (a === -1 || b === -1) { toggleMultiSelect(id); return; }
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  state.multiSelected.clear();
  for (let i = lo; i <= hi; i++) state.multiSelected.add(list[i]);
  document.querySelectorAll(".file-row.multi-selected").forEach((el) => el.classList.remove("multi-selected"));
  for (const sid of state.multiSelected) {
    const li = document.querySelector(`.file-row[data-id="${sid}"]`);
    if (li) li.classList.add("multi-selected");
  }
  selectFile(id);
  renderControls();
}

export function clearMultiSelect() {
  if (state.multiSelected.size === 0) return;
  state.multiSelected.clear();
  document.querySelectorAll(".file-row.multi-selected").forEach((el) => el.classList.remove("multi-selected"));
  renderControls();
}
