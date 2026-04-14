// Overlay palettes: sort, run, plugin actions, test-sort.
// All four use the shared `togglePalette` helper; the four concrete
// toggles are thin wrappers that provide the rows + click callback.

import { state } from "./state.js";
import { $, escapeHtml } from "./util.js";
import { SORT_OPTIONS, updateTestFiltered } from "./sort.js";
import {
  runAll, runVisible, rerunFailing, runMultiSelected, runPluginAction,
} from "./api.js";
import {
  renderFilterList, renderLeftPane, renderRightPane,
} from "./render.js";

/// Shared palette: floating overlay menu with a title, rows, footer,
/// click + keyboard navigation. Each row: `{ id, label, desc?, enabled?, active? }`.
export function togglePalette(id, title, rows, footer, onClick) {
  const existing = $(id);
  if (existing) { closePalette(id); return; }
  const pal = document.createElement("div");
  pal.id = id;
  pal.className = "overlay-palette";
  const items = rows.map((r, i) => {
    const cls = [
      "pal-row",
      r.enabled === false ? "disabled" : "",
      r.active ? "active" : "",
      i === 0 ? "cursor" : "",
    ].filter(Boolean).join(" ");
    return `<div class="${cls}" data-id="${escapeHtml(r.id)}" data-idx="${i}">
      <span class="pal-label">${escapeHtml(r.label)}</span>
      <span class="pal-desc">${escapeHtml(r.desc ?? "")}</span>
    </div>`;
  }).join("");
  pal.innerHTML = `<div class="pal-card">
    <div class="pal-title">${escapeHtml(title)}</div>
    ${items}
    <div class="pal-footer">${escapeHtml(footer)}</div>
  </div>`;
  document.body.appendChild(pal);

  let cursor = 0;
  const palRows = pal.querySelectorAll(".pal-row");
  const n = palRows.length;

  const updateCursor = () => {
    palRows.forEach((r, i) => r.classList.toggle("cursor", i === cursor));
  };

  const close = () => closePalette(id);
  const selectCurrent = () => {
    const row = palRows[cursor];
    if (!row || row.classList.contains("disabled")) return;
    close();
    onClick(row.dataset.id);
  };

  const onKey = (e) => {
    const k = e.key;
    if (k === "j" || k === "ArrowDown") {
      cursor = (cursor + 1) % n; updateCursor();
      e.preventDefault(); e.stopPropagation();
    } else if (k === "k" || k === "ArrowUp") {
      cursor = (cursor - 1 + n) % n; updateCursor();
      e.preventDefault(); e.stopPropagation();
    } else if (k === "Enter") {
      selectCurrent();
      e.preventDefault(); e.stopPropagation();
    }
  };
  window.addEventListener("keydown", onKey, true);
  pal._onKey = onKey;

  pal.addEventListener("click", (e) => {
    const row = e.target.closest(".pal-row");
    if (!row || row.classList.contains("disabled")) { close(); return; }
    close();
    onClick(row.dataset.id);
  });
}

export function closePalette(id) {
  const pal = $(id);
  if (!pal) return;
  if (pal._onKey) window.removeEventListener("keydown", pal._onKey, true);
  pal.remove();
}

// ── Concrete palettes ──────────────────────────────────────────────────

export function toggleSortPalette() {
  togglePalette("sort-palette", "Sort",
    SORT_OPTIONS.map((o) => ({
      id: o.id,
      label: o.label,
      desc: o.desc + (o.id === state.sortMode ? (state.sortReversed ? " \u2193" : " \u2191") : ""),
      active: o.id === state.sortMode,
    })),
    "click to select \u00b7 click active to reverse \u00b7 s or Esc to close",
    (picked) => {
      if (picked === state.sortMode) {
        state.sortReversed = !state.sortReversed;
      } else {
        state.sortMode = picked;
        state.sortReversed = false;
      }
      renderFilterList();
      renderLeftPane();
    },
  );
}

export const closeSortPalette = () => closePalette("sort-palette");

export function toggleActionPalette() {
  const f = state.selected ? state.files.get(state.selected) : null;
  const suite = f ? (state.pkg?.suites ?? []).find((s) => s.name === f.suite) : null;
  const actions = suite?.actions ?? [];
  if (actions.length === 0) return;
  togglePalette("action-palette", "Actions",
    actions.map((a) => ({
      id: a.name,
      label: a.label,
      desc: a.scope === "all" ? "all files" : "this file",
    })),
    "click to run \u00b7 a or Esc to close",
    (picked) => { runPluginAction(picked); },
  );
}

export const closeActionPalette = () => closePalette("action-palette");

export function toggleRunPalette() {
  if (state.currentRun?.in_progress) return;
  const nAll = state.fileOrder.length;
  const nVisible = state.filtered.length;
  const nSelected = state.multiSelected.size;
  const nFailed = state.currentRun?.bad_files?.length ?? 0;
  togglePalette("run-palette", "Run",
    [
      { id: "all",      label: "all",      desc: `${nAll} file${nAll === 1 ? "" : "s"}`,           enabled: nAll > 0 },
      { id: "visible",  label: "visible",  desc: `${nVisible} file${nVisible === 1 ? "" : "s"}`,   enabled: nVisible > 0 },
      { id: "selected", label: "selected", desc: `${nSelected} file${nSelected === 1 ? "" : "s"}`, enabled: nSelected > 0 },
      { id: "failed",   label: "failed",   desc: `${nFailed} file${nFailed === 1 ? "" : "s"}`,     enabled: nFailed > 0 },
    ],
    "click to run \u00b7 r or Esc to close",
    (picked) => {
      if (picked === "all") runAll();
      else if (picked === "visible") runVisible();
      else if (picked === "selected") runMultiSelected();
      else if (picked === "failed") rerunFailing();
    },
  );
}

export const closeRunPalette = () => closePalette("run-palette");

/// Test-sort palette: same mechanism, but filters out "suite" (tests
/// don't carry a suite independent from their owning file).
export function toggleTestSortPalette() {
  const opts = SORT_OPTIONS.filter((o) => o.id !== "suite");
  togglePalette("sort-palette", "Sort tests",
    opts.map((o) => ({
      id: o.id,
      label: o.label,
      desc: o.desc + (o.id === state.testSortMode ? (state.testSortReversed ? " \u2193" : " \u2191") : ""),
      active: o.id === state.testSortMode,
    })),
    "click to select \u00b7 click active to reverse \u00b7 s or Esc to close",
    (picked) => {
      if (picked === state.testSortMode) {
        state.testSortReversed = !state.testSortReversed;
      } else {
        state.testSortMode = picked;
        state.testSortReversed = false;
      }
      updateTestFiltered();
      renderRightPane();
      if (state.level === "detail") renderLeftPane();
    },
  );
}
