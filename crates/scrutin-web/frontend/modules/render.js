// All render* functions live here. Level-dependent rendering (left pane,
// right pane, breadcrumb) dispatches through LEVELS so each level's
// behavior is defined in levels.js, not as a switch on `state.level`.

import { state, STATUS_CYCLE } from "./state.js";
import {
  $, escapeHtml, displayStatus, formatMs, formatMetrics, isBadOutcome,
} from "./util.js";
import { sortMessages, fileStatusRank } from "./sort.js";
import { currentLevel, LEVELS } from "./levels.js";
import {
  renderSourceRows, sourcePlaceholder,
  renderTestSourceInto, renderFnSourceInto, wireEditButtons,
} from "./sources.js";
import {
  selectFile, moveFailureSelection, jumpToLevel,
  enterDetail, enterFailure,
  rangeSelect, toggleMultiSelect, clearMultiSelect,
} from "./navigation.js";
import { toggleActionPalette, toggleTestSortPalette } from "./palettes.js";

// ── Top-level dispatch ────────────────────────────────────────────────

export function renderAll() {
  renderHeader();
  populatePluginDropdown();
  renderColHeaders();
  renderFilterList();
  renderLeftPane();
  renderRightPane();
  renderCounts();
  renderControls();
  renderHints();
}

// ── Topbar metadata (right side) + workers + watch toggle ─────────────

export function renderHeader() {
  const pkgEl = $("pkg-name");
  if (pkgEl) pkgEl.textContent = state.pkg?.name ?? "\u2014";
  const workersEl = $("workers");
  if (workersEl) {
    const busy = state.currentRun?.in_progress ? state.busy : 0;
    workersEl.textContent = `${busy}/${state.nWorkers} workers`;
  }
  const watch = $("toggle-watch");
  if (watch) watch.checked = state.watching;
}

// ── Topbar breadcrumb (left side). Segments come from the level. ──────

export function renderBreadcrumb() {
  const pill = $("level-pill");
  const crumbs = $("crumbs");
  const backBtn = $("btn-back");
  if (!pill || !crumbs) return;

  const L = currentLevel();
  pill.textContent = L.id.toUpperCase();
  pill.className = `level-pill level-${L.id}`;
  if (backBtn) backBtn.classList.toggle("hidden", L.id === "files");

  const segs = L.segments();
  crumbs.innerHTML = "";
  segs.forEach((seg, i) => {
    if (i > 0) {
      const sep = document.createElement("span");
      sep.className = "sep";
      sep.textContent = "\u203a";
      crumbs.appendChild(sep);
    }
    const isCurrent = seg.level === L.id;
    const btn = document.createElement("button");
    btn.className = "crumb" + (isCurrent ? " current" : "");
    btn.textContent = seg.label;
    btn.title = seg.label;
    if (!isCurrent) btn.addEventListener("click", () => jumpToLevel(seg.level));
    else btn.disabled = true;
    crumbs.appendChild(btn);
  });

  const counter = L.counter();
  if (counter) {
    const span = document.createElement("span");
    span.className = "counter";
    span.textContent = counter;
    crumbs.appendChild(span);
  }
}

// ── Bottom hints bar (auto-generated from the shared keymap) ──────────

export function renderHints() {
  const el = $("keyboard-hint");
  if (!el) return;
  const running = state.currentRun && state.currentRun.in_progress;
  const level = state.level;

  const seen = new Set();
  const parts = [];
  for (const b of state.keymap) {
    if (!b.bar || seen.has(b.action)) continue;
    if (!b.levels.includes(level)) continue;
    if (b.when === "when_idle" && running) continue;
    if (b.when === "when_running" && !running) continue;
    seen.add(b.action);
    const sp = b.bar.indexOf(" ");
    const key = sp > 0 ? b.bar.slice(0, sp) : b.bar;
    const label = sp > 0 ? b.bar.slice(sp + 1) : "";
    parts.push(`<kbd>${escapeHtml(key)}</kbd> ${escapeHtml(label)}`);
  }
  const hasActions = (state.pkg?.suites ?? []).some((s) => (s.actions ?? []).length > 0);
  if (hasActions && (level === "files" || level === "detail")) {
    parts.push("<kbd>a</kbd> actions");
  }
  el.innerHTML = parts.join(" <span class='sep'>\u00b7</span> ");
}

// ── File-list chrome: plugin dropdown, col headers, filter list ───────

export function populatePluginDropdown() {
  const sel = $("plugin-select");
  if (!sel) return;
  const suitesInUse = new Set();
  for (const [, f] of state.files) if (f.suite) suitesInUse.add(f.suite);
  sel.style.display = suitesInUse.size > 1 ? "" : "none";
  const existing = new Set(Array.from(sel.options).map((o) => o.value));
  const want = new Set(["", ...(state.pkg?.suites ?? []).map((s) => s.name)]);
  if (existing.size !== want.size || [...existing].some((v) => !want.has(v))) {
    sel.innerHTML = "";
    const allOpt = document.createElement("option");
    allOpt.value = ""; allOpt.textContent = "all";
    sel.appendChild(allOpt);
    for (const s of state.pkg?.suites ?? []) {
      const o = document.createElement("option");
      o.value = s.name; o.textContent = s.name;
      sel.appendChild(o);
    }
  }
  sel.value = state.pluginFilter;
}

export function renderColHeaders() {
  const hdr = $("col-headers");
  if (!hdr) return;
  const suitesInUse = new Set();
  for (const [, f] of state.files) if (f.suite) suitesInUse.add(f.suite);
  const multiSuite = suitesInUse.size > 1;
  hdr.className = multiSuite ? "file-row col-header" : "file-row col-header no-suite";

  const btn = (label, mode, compact) => {
    const active = state.sortMode === mode;
    const arrow = active && !compact ? (state.sortReversed ? " \u2193" : " \u2191") : "";
    return `<button class="col-btn ${active ? "active" : ""}" data-sort="${mode}">${label}<span class="col-arrow">${arrow}</span></button>`;
  };

  hdr.innerHTML = `
    ${btn("\u25cf", "status", true)}
    ${btn("name", "name")}
    ${multiSuite ? btn("suite", "suite") : ""}
    ${btn("time", "time")}
  `;
  hdr.querySelectorAll(".col-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const mode = b.dataset.sort;
      if (mode === state.sortMode) state.sortReversed = !state.sortReversed;
      else { state.sortMode = mode; state.sortReversed = false; }
      renderColHeaders();
      renderFilterList();
      renderLeftPane();
    });
  });
}

export function renderFilterList() {
  const q = state.filterText.trim().toLowerCase();
  const plugin = state.pluginFilter;
  const filtered = state.fileOrder.filter((id) => {
    const f = state.files.get(id);
    if (!f) return false;
    if (plugin && f.suite !== plugin) return false;
    if (state.statusFilter) {
      if (state.statusFilter === "warned") {
        if (f.status !== "passed" || (f.counts?.warn ?? 0) === 0) return false;
      } else if (f.status !== state.statusFilter) {
        return false;
      }
    }
    if (q) return f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q);
    return true;
  });
  const origIdx = new Map(state.fileOrder.map((id, i) => [id, i]));
  const tiebreak = (a, b) => (origIdx.get(a) ?? 0) - (origIdx.get(b) ?? 0);
  filtered.sort((a, b) => {
    const fa = state.files.get(a);
    const fb = state.files.get(b);
    let cmp = 0;
    switch (state.sortMode) {
      case "name":   cmp = (fa?.name  ?? "").localeCompare(fb?.name  ?? ""); break;
      case "suite":  cmp = (fa?.suite ?? "").localeCompare(fb?.suite ?? ""); break;
      case "status": cmp = fileStatusRank(fa) - fileStatusRank(fb); break;
      case "time":   cmp = (fb?.last_duration_ms ?? -1) - (fa?.last_duration_ms ?? -1); break;
      default: cmp = 0; break;
    }
    return cmp !== 0 ? cmp : tiebreak(a, b);
  });
  if (state.sortReversed) filtered.reverse();
  state.filtered = filtered;
}

export function cyclePlugin(delta) {
  const names = ["", ...(state.pkg?.suites ?? []).map((s) => s.name)];
  if (names.length <= 1) return;
  const i = names.indexOf(state.pluginFilter);
  const next = (i + delta + names.length) % names.length;
  state.pluginFilter = names[next];
  const sel = $("plugin-select");
  if (sel) sel.value = state.pluginFilter;
  renderFilterList();
  renderLeftPane();
  renderControls();
}

export function cycleStatus(delta) {
  const i = STATUS_CYCLE.indexOf(state.statusFilter);
  const idx = i < 0 ? 0 : i;
  const next = (idx + delta + STATUS_CYCLE.length) % STATUS_CYCLE.length;
  state.statusFilter = STATUS_CYCLE[next];
  const sel = $("status-select");
  if (sel) sel.value = state.statusFilter;
  renderFilterList();
  renderLeftPane();
  renderControls();
}

// ── Left pane dispatch ────────────────────────────────────────────────

export function renderLeftPane() {
  // Toggle the failure-view CSS hook so the sidebar collapses in Failure.
  const layout = $("layout");
  if (layout) layout.classList.toggle("failure-view", LEVELS.failure === currentLevel());

  currentLevel().renderLeft({
    renderFileList,
    renderTestListLeft,
  });

  // Sub-bar + col headers only show on the Files level.
  const isFiles = state.level === "files";
  const subbar = $("left-subbar");
  const colHeaders = $("col-headers");
  if (subbar) subbar.style.display = isFiles ? "" : "none";
  if (colHeaders) colHeaders.style.display = isFiles ? "" : "none";
  renderBreadcrumb();
}

function renderFileList() {
  const ul = $("left-list");
  if (!ul) return;
  let maxMs = 0;
  for (const id of state.filtered) {
    const f = state.files.get(id);
    if (f?.last_duration_ms != null && f.last_duration_ms > maxMs) maxMs = f.last_duration_ms;
  }
  const suitesInUse = new Set();
  for (const [, f] of state.files) if (f.suite) suitesInUse.add(f.suite);
  const multiSuite = suitesInUse.size > 1;
  ul.innerHTML = "";
  ul.className = "file-list";
  for (const id of state.filtered) {
    const f = state.files.get(id);
    if (!f) continue;
    const li = document.createElement("li");
    li.className = multiSuite ? "file-row" : "file-row no-suite";
    li.dataset.id = id;
    if (id === state.selected) li.classList.add("selected");
    if (state.multiSelected.has(id)) li.classList.add("multi-selected");
    li.innerHTML = `
      <span class="status-dot ${displayStatus(f)}"></span>
      <span class="fname">${escapeHtml(f.name)}</span>
      ${multiSuite ? `<span class="suite">${escapeHtml(f.suite)}</span>` : ""}
      ${renderDurationCell(f, maxMs)}
    `;
    li.addEventListener("click", (e) => {
      if (e.shiftKey) rangeSelect(id);
      else if (e.metaKey || e.ctrlKey) toggleMultiSelect(id);
      else { clearMultiSelect(); selectFile(id); state.lastClicked = id; }
    });
    li.addEventListener("dblclick", () => enterDetail());
    ul.appendChild(li);
  }
}

function renderTestListLeft() {
  const ul = $("left-list");
  if (!ul) return;
  ul.innerHTML = "";
  ul.className = "test-list";
  const tests = state.testFiltered;
  for (let i = 0; i < tests.length; i++) {
    const m = tests[i];
    const li = document.createElement("li");
    li.className = "test-row";
    if (i === state.testCursor) li.classList.add("selected");
    li.dataset.idx = i;
    const name = m.test_name ?? "<anon>";
    const dur = m.duration_ms != null ? formatMs(m.duration_ms) : "";
    li.innerHTML = `
      <span class="outcome-dot ${m.outcome}"></span>
      <span class="test-name">${escapeHtml(name)}</span>
      <span class="test-duration">${dur}</span>
    `;
    li.addEventListener("click", () => {
      state.testCursor = i;
      renderLeftPane();
      renderRightPane();
    });
    ul.appendChild(li);
  }
}

function renderDurationCell(f, maxMs) {
  const ms = f.last_duration_ms;
  if (ms == null || maxMs === 0) return `<span class="duration-wrap"></span>`;
  const pct = Math.max(4, Math.round((ms / maxMs) * 100));
  let cls = "";
  if (f.status === "failed") cls = "failed";
  else if (f.status === "errored") cls = "errored";
  else if (f.status === "running") cls = "running";
  else if ((f.counts?.warn ?? 0) > 0) cls = "warned";
  return `
    <span class="duration-wrap">
      <span class="duration-bar ${cls}" style="width: ${pct}%"></span>
      <span class="duration-ms">${formatMs(ms)}</span>
    </span>
  `;
}

// ── Right pane dispatch ───────────────────────────────────────────────

export function renderRightPane() {
  // `.failure-body` strips padding and scroll so the 3-pane Failure grid
  // can fill the viewport.
  const body = $("right-body");
  if (body) body.classList.toggle("failure-body", state.level === "failure");

  currentLevel().renderRight({
    renderTestListRight,
    renderTestDetail,
    renderFailureDetail,
  });
}

function selectedSuiteActions() {
  if (!state.selected) return [];
  const f = state.files.get(state.selected);
  if (!f) return [];
  const suite = (state.pkg?.suites ?? []).find((s) => s.name === f.suite);
  return suite?.actions ?? [];
}

function renderTestListRight() {
  const body = $("right-body");
  if (!body) return;

  if (!state.selected) {
    body.innerHTML = '<div class="detail-empty">select a file from the list</div>';
    return;
  }
  const f = state.files.get(state.selected);
  if (!f) { body.innerHTML = ""; return; }

  const actions = selectedSuiteActions();
  const actionBtn = actions.length > 0
    ? `<button class="edit-btn action-btn" id="open-actions-btn" title="Plugin actions (f)">actions</button>`
    : "";

  const header = `<div class="detail-file-header">
    <h2>${escapeHtml(f.name)}</h2>
    <span class="status-pill ${displayStatus(f)}">${displayStatus(f)}</span>
    <button class="edit-btn" data-edit="test" title="Edit test file (e)">edit test</button>
    <button class="edit-btn" data-edit="source" title="Edit source file (E)">edit source</button>
    ${actionBtn}
  </div>`;

  if (!f.messages || f.messages.length === 0) {
    body.innerHTML = header + '<div class="detail-empty">no test messages yet \u2014 run this file to see results.</div>';
    wireEditButtons(body, f.id);
    wireActionsButton(body);
    return;
  }

  const sorted = sortMessages(f.messages);
  const rows = sorted.map((m, idx) => {
    const name = m.test_name ?? "<anon>";
    const dur = m.duration_ms != null ? formatMs(m.duration_ms) : "";
    const showMsg = m.message && (isBadOutcome(m) || m.outcome === "warn");
    const msgPreview = showMsg
      ? `<div class="test-row-message ${m.outcome}">${escapeHtml(m.message.split("\n")[0].slice(0, 120))}</div>`
      : "";
    return `<li class="test-row${showMsg ? " has-message" : ""}" data-idx="${idx}">
      <span class="outcome-dot ${m.outcome}"></span>
      <span class="test-name">${escapeHtml(name)}</span>
      <span class="test-duration">${dur}</span>
      ${msgPreview}
    </li>`;
  }).join("");

  const summary = `<div class="detail-summary" style="padding: 6px 12px;">
    sort: <a href="#" class="test-sort-link">${state.testSortMode}${state.testSortReversed ? " \u2193" : " \u2191"}</a>
  </div>`;

  body.innerHTML = header + summary + `<ul class="test-list">${rows}</ul>`;

  wireEditButtons(body, f.id);
  wireActionsButton(body);

  const sortLink = body.querySelector(".test-sort-link");
  if (sortLink) sortLink.addEventListener("click", (e) => { e.preventDefault(); toggleTestSortPalette(); });

  body.querySelectorAll(".test-row").forEach((el) => {
    el.addEventListener("click", () => {
      state.testCursor = parseInt(el.dataset.idx, 10);
      enterDetail();
    });
  });
}

function wireActionsButton(body) {
  const actBtn = body.querySelector("#open-actions-btn");
  if (actBtn) actBtn.addEventListener("click", () => toggleActionPalette());
}

function renderTestDetail() {
  const body = $("right-body");
  if (!body) return;

  const f = state.files.get(state.selected);
  if (!f) return;

  const tests = state.testFiltered;
  const m = tests[state.testCursor];
  if (!m) { body.innerHTML = '<div class="detail-empty">no test selected</div>'; return; }

  const name = m.test_name ?? "<anon>";
  const bad = isBadOutcome(m);

  let html = "";
  html += `<div class="detail-section">
    <div class="detail-meta">
      <span class="outcome-tag ${m.outcome}">${m.outcome}</span>
      <span>${escapeHtml(name)}</span>
      ${m.duration_ms != null ? `<span>${formatMs(m.duration_ms)}</span>` : ""}
      ${m.location ? `<span>${escapeHtml(m.location.file)}${m.location.line != null ? `:${m.location.line}` : ""}</span>` : ""}
      ${bad ? `<button class="edit-btn" id="focus-failure-btn" title="Focus failure (Enter)">focus \u2192</button>` : ""}
    </div>
  </div>`;

  if (m.message) {
    const msgHeader = m.outcome === "warn" ? "Warning" : "Error";
    html += `<div class="detail-section">
      <div class="detail-section-header">${msgHeader}</div>
      <div class="detail-error ${bad ? "" : "warn-msg"}">${escapeHtml(m.message)}</div>
    </div>`;
  }

  if (m.metrics) {
    html += `<div class="detail-section">
      <div class="detail-section-header">Metrics</div>
      <div style="font-size: 12px; color: var(--fg-dim);">${formatMetrics(m.metrics)}</div>
    </div>`;
  }

  html += `<div class="detail-section">
    <div class="detail-section-header">
      Test source
      <button class="edit-btn" data-edit="test" title="Edit test file (e)">edit</button>
    </div>
    <div class="source-snippet" id="detail-test-source">${sourcePlaceholder()}</div>
  </div>`;

  html += `<div class="detail-section">
    <div class="detail-section-header">
      Source
      <button class="edit-btn" data-edit="source" title="Edit source file (E)">edit</button>
    </div>
    <div class="source-snippet" id="detail-source-fn">${sourcePlaceholder()}</div>
  </div>`;

  body.innerHTML = html;
  wireEditButtons(body, f.id, m.location?.line);

  $("focus-failure-btn")?.addEventListener("click", () => enterFailure());

  renderTestSourceInto("detail-test-source", f.id, m.location?.line);
  renderFnSourceInto("detail-source-fn", f.id, (path) => {
    // Update the section header with the source file name.
    const el = $("detail-source-fn");
    const header = el?.closest(".detail-section")?.querySelector(".detail-section-header");
    if (header) {
      const editBtn = header.querySelector(".edit-btn");
      header.textContent = `Source \u2014 ${path}`;
      header.style.textTransform = "none";
      if (editBtn) header.appendChild(editBtn);
    }
  });
}

function renderFailureDetail() {
  const body = $("right-body");
  if (!body) return;

  const ff = state.failures[state.failureCursor];
  if (!ff) {
    body.innerHTML = '<div class="detail-empty">no failures in this run</div>';
    return;
  }

  const total = state.failures.length;
  const nav = `<div class="failure-carousel">
    <button class="edit-btn" id="failure-prev" ${state.failureCursor === 0 ? "disabled" : ""} title="Previous failure (k)">\u2190 prev</button>
    <span class="failure-pos">${state.failureCursor + 1} / ${total}</span>
    <button class="edit-btn" id="failure-next" ${state.failureCursor === total - 1 ? "disabled" : ""} title="Next failure (j)">next \u2192</button>
    <span class="failure-spacer"></span>
    <button class="edit-btn" data-edit="test" title="Edit test file (e)">edit test</button>
    <button class="edit-btn" data-edit="source" title="Edit source file (E)">edit source</button>
  </div>`;

  const hasLine = ff.line != null;
  const testHeader = hasLine ? `Test \u2014 line ${ff.line}` : "Test";
  const panes = `<div class="failure-panes">
    <div class="failure-top">
      <div class="failure-pane">
        <div class="failure-pane-header">${testHeader}</div>
        <div class="source-snippet" id="failure-test-source">${sourcePlaceholder()}</div>
      </div>
      <div class="failure-pane">
        <div class="failure-pane-header" id="failure-source-title">Source</div>
        <div class="source-snippet" id="failure-source-fn">${sourcePlaceholder()}</div>
      </div>
    </div>
    <div class="failure-pane failure-bottom">
      <div class="failure-pane-header">Error</div>
      <div class="detail-error">${escapeHtml(ff.message)}</div>
    </div>
  </div>`;

  body.innerHTML = nav + panes;

  $("failure-prev")?.addEventListener("click", () => moveFailureSelection(-1));
  $("failure-next")?.addEventListener("click", () => moveFailureSelection(+1));
  wireEditButtons(body, ff.fileId, ff.line);

  renderTestSourceInto("failure-test-source", ff.fileId, ff.line);
  renderFnSourceInto("failure-source-fn", ff.fileId, (path) => {
    const hdr = $("failure-source-title");
    if (hdr) hdr.textContent = `Source \u2014 ${path}`;
  });
}

// ── Counts footer + run controls ──────────────────────────────────────

const COUNT_ICONS = { pass: "\u25cf", fail: "\u2717", error: "\u26a0", warn: "\u26a1", skip: "\u2298", xfail: "\u2299" };

export function renderCounts() {
  const t = state.totals;
  setCount("pass",  t.pass);
  setCount("fail",  t.fail);
  setCount("error", t.error);
  setCount("warn",  t.warn);
  setCount("skip",  t.skip);
  setCount("xfail", t.xfail);
}

export function setCount(name, n) {
  const el = document.querySelector(`#countsbar .count.${name}`);
  if (el) el.textContent = `${COUNT_ICONS[name] || ""}${n}`;
}

export function setStatus(s) {
  const el = $("status-text");
  if (el) el.textContent = s;
}

export function renderControls() {
  const running = state.currentRun?.in_progress === true;
  const bad = (state.currentRun?.bad_files ?? []).length > 0;
  const cancelBtn = $("btn-cancel");
  if (cancelBtn) cancelBtn.classList.toggle("hidden", !running);
  const rerunBtn = $("btn-rerun-failing");
  if (rerunBtn) rerunBtn.classList.toggle("hidden", !bad || running);
  const runBtn = $("btn-run-all");
  if (runBtn) runBtn.disabled = running;
  const visBtn = $("btn-run-visible");
  if (visBtn) {
    const n = state.filtered.length;
    visBtn.textContent = `\u25b6 run visible (${n})`;
    visBtn.disabled = running || n === 0;
  }
  const selBtn = $("btn-run-selected");
  if (selBtn) {
    const n = state.multiSelected.size;
    selBtn.textContent = n > 0 ? `\u25b6 run selected (${n})` : `\u25b6 run selected`;
    selBtn.disabled = running || !state.selected;
  }
}
