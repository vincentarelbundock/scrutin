// ../../crates/scrutin-web/frontend/modules/state.js
var IS_VSCODE = typeof acquireVsCodeApi === "function";
var vscode = IS_VSCODE ? acquireVsCodeApi() : null;
var BASE = IS_VSCODE ? window.__SCRUTIN_BASE_URL__ || "http://127.0.0.1:7878" : "";
var IS_EDITOR = IS_VSCODE;
var state = {
  pkg: null,
  files: /* @__PURE__ */ new Map(),
  // FileId -> WireFile
  fileOrder: [],
  // stable display order
  filtered: [],
  // visible slice after all filters
  selected: null,
  // FileId (highlighted file)
  multiSelected: /* @__PURE__ */ new Set(),
  lastClicked: null,
  // anchor for shift-click range selection
  currentRun: null,
  // WireRunSummary
  watching: false,
  nWorkers: 1,
  busy: 0,
  filterText: "",
  pluginFilter: "",
  statusFilter: "",
  sortMode: "status",
  sortReversed: false,
  testSortMode: "status",
  testSortReversed: false,
  totals: { pass: 0, fail: 0, error: 0, skip: 0, xfail: 0, warn: 0 },
  sourceCache: /* @__PURE__ */ new Map(),
  keymap: [],
  level: "files",
  // "files" | "detail" | "failure"
  testCursor: 0,
  testFiltered: [],
  // sorted messages for the selected file
  failureCursor: 0,
  failures: []
  // global {fileId, file, test, message, line, outcome}
};
var STATUS_CYCLE = [
  "",
  "failed",
  "errored",
  "warned",
  "passed",
  "skipped",
  "running",
  "pending",
  "cancelled"
];
var OUTCOME_RANK = { fail: 0, error: 1, warn: 2, pass: 3, skip: 4, xfail: 5 };
function setOutcomeRanks(ranks) {
  for (const k of Object.keys(OUTCOME_RANK)) delete OUTCOME_RANK[k];
  Object.assign(OUTCOME_RANK, ranks);
}

// ../../crates/scrutin-web/frontend/modules/util.js
var $ = (id) => document.getElementById(id);
function escapeHtml(s) {
  if (s == null) return "";
  return String(s).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#039;");
}
var isBadOutcome = (m) => m?.outcome === "fail" || m?.outcome === "error";
function displayStatus(f) {
  if (f.status === "passed" && (f.counts?.warn ?? 0) > 0) return "warned";
  return f.status;
}
function formatMs(ms) {
  if (ms < 1e3) return `${ms}ms`;
  return `${(ms / 1e3).toFixed(1)}s`;
}
function formatMetrics(m) {
  if (m.total != null && m.failed != null) {
    const frac = m.fraction != null ? (m.fraction * 100).toFixed(2) : "0.00";
    return `${m.failed} of ${m.total} failed (${frac}%)`;
  }
  if (m.total != null) return `${m.total} checked`;
  if (m.failed != null) return `${m.failed} failed`;
  return "";
}
var toastTimer = null;
function toast(msg, isError) {
  const el = $("toast");
  if (!el) return;
  el.textContent = msg;
  el.classList.remove("hidden");
  el.classList.toggle("error", !!isError);
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), 4e3);
}

// ../../crates/scrutin-web/frontend/modules/sort.js
var SORT_OPTIONS = [
  { id: "sequential", label: "sequential", desc: "original order" },
  { id: "status", label: "status", desc: "failures first" },
  { id: "name", label: "name", desc: "alphabetical" },
  { id: "suite", label: "suite", desc: "by suite" },
  { id: "time", label: "time", desc: "slowest first" }
];
function fileStatusRank(f) {
  if (!f) return 99;
  switch (f.status) {
    case "errored":
      return 0;
    case "failed":
      return 1;
    case "passed":
      return (f.counts?.warn ?? 0) > 0 ? 2 : 6;
    case "running":
      return 3;
    case "cancelled":
      return 4;
    case "pending":
      return 5;
    case "skipped":
      return 7;
    default:
      return 8;
  }
}
function sortMessages(msgs) {
  const sorted = [...msgs];
  const mode = state.testSortMode;
  if (mode === "sequential") return sorted;
  sorted.sort((a, b) => {
    switch (mode) {
      case "status":
        return (OUTCOME_RANK[a.outcome] ?? 9) - (OUTCOME_RANK[b.outcome] ?? 9);
      case "name":
        return (a.test_name ?? "").localeCompare(b.test_name ?? "");
      case "time":
        return (b.duration_ms ?? 0) - (a.duration_ms ?? 0);
      default:
        return 0;
    }
  });
  if (state.testSortReversed) sorted.reverse();
  return sorted;
}
function updateTestFiltered() {
  const f = state.files.get(state.selected);
  if (!f || !f.messages) {
    state.testFiltered = [];
    return;
  }
  state.testFiltered = sortMessages(f.messages);
  if (state.testCursor >= state.testFiltered.length) {
    state.testCursor = Math.max(0, state.testFiltered.length - 1);
  }
}

// ../../crates/scrutin-web/frontend/modules/navigation.js
function refreshUi() {
  renderLeftPane();
  renderRightPane();
  renderHints();
}
function selectFile(id) {
  state.selected = id;
  state.testCursor = 0;
  updateTestFiltered();
  document.querySelectorAll(".file-row.selected").forEach((el) => el.classList.remove("selected"));
  const li = document.querySelector(`.file-row[data-id="${id}"]`);
  if (li) li.classList.add("selected");
  renderRightPane();
}
function moveFileSelection(delta) {
  if (state.filtered.length === 0) return;
  const idx = state.filtered.indexOf(state.selected);
  let next = idx + delta;
  if (!Number.isFinite(delta)) next = delta > 0 ? state.filtered.length - 1 : 0;
  next = Math.max(0, Math.min(state.filtered.length - 1, next));
  selectFile(state.filtered[next]);
  const li = document.querySelector(`.file-row[data-id="${state.selected}"]`);
  if (li) li.scrollIntoView({ block: "nearest" });
}
function moveTestSelection(delta) {
  if (state.testFiltered.length === 0) return;
  if (delta === 1 && state.testCursor === state.testFiltered.length - 1) {
    const idx = state.filtered.indexOf(state.selected);
    if (idx >= 0 && idx + 1 < state.filtered.length) {
      selectFile(state.filtered[idx + 1]);
      renderLeftPane();
      const row2 = document.querySelector(`.test-row[data-idx="0"]`);
      if (row2) row2.scrollIntoView({ block: "nearest" });
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
      const row2 = document.querySelector(`.test-row[data-idx="${state.testCursor}"]`);
      if (row2) row2.scrollIntoView({ block: "nearest" });
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
function enterDetail() {
  if (!state.selected) return;
  const f = state.files.get(state.selected);
  if (!f || !f.messages || f.messages.length === 0) return;
  state.level = "detail";
  updateTestFiltered();
  refreshUi();
}
function exitDetail() {
  if (state.level !== "detail") return;
  state.level = "files";
  state.testCursor = 0;
  refreshUi();
}
function buildFailures() {
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
          outcome: m.outcome
        });
      }
    }
  }
  state.failures = out;
}
function enterFailure() {
  buildFailures();
  if (state.failures.length === 0) return;
  let cursor = 0;
  if (state.level === "detail") {
    const m = state.testFiltered[state.testCursor];
    if (m && isBadOutcome(m)) {
      const name = m.test_name ?? "<anon>";
      const idx = state.failures.findIndex(
        (ff) => ff.fileId === state.selected && ff.test === name
      );
      if (idx >= 0) cursor = idx;
    }
  }
  state.failureCursor = cursor;
  state.level = "failure";
  refreshUi();
}
function exitFailure() {
  if (state.level !== "failure") return;
  const ff = state.failures[state.failureCursor];
  if (ff && state.files.get(ff.fileId)) {
    state.selected = ff.fileId;
    updateTestFiltered();
    const idx = state.testFiltered.findIndex(
      (m) => (m.test_name ?? "<anon>") === ff.test
    );
    state.testCursor = idx >= 0 ? idx : 0;
    state.level = "detail";
  } else {
    state.level = "files";
  }
  refreshUi();
}
function moveFailureSelection(delta) {
  if (state.failures.length === 0) return;
  let next = state.failureCursor + delta;
  if (!Number.isFinite(delta)) next = delta > 0 ? state.failures.length - 1 : 0;
  next = Math.max(0, Math.min(state.failures.length - 1, next));
  if (next === state.failureCursor) return;
  state.failureCursor = next;
  const ff = state.failures[state.failureCursor];
  if (ff && state.selected !== ff.fileId) state.selected = ff.fileId;
  renderLeftPane();
  renderRightPane();
}
function jumpToLevel(targetLevel) {
  while (state.level !== targetLevel) {
    if (state.level === "failure") exitFailure();
    else if (state.level === "detail") exitDetail();
    else break;
  }
}
function toggleMultiSelect(id) {
  if (state.multiSelected.has(id)) state.multiSelected.delete(id);
  else state.multiSelected.add(id);
  state.lastClicked = id;
  const li = document.querySelector(`.file-row[data-id="${id}"]`);
  if (li) li.classList.toggle("multi-selected", state.multiSelected.has(id));
  renderControls();
}
function rangeSelect(id) {
  const anchor = state.lastClicked;
  if (!anchor) {
    toggleMultiSelect(id);
    return;
  }
  const list = state.filtered;
  const a = list.indexOf(anchor);
  const b = list.indexOf(id);
  if (a === -1 || b === -1) {
    toggleMultiSelect(id);
    return;
  }
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
function clearMultiSelect() {
  if (state.multiSelected.size === 0) return;
  state.multiSelected.clear();
  document.querySelectorAll(".file-row.multi-selected").forEach((el) => el.classList.remove("multi-selected"));
  renderControls();
}

// ../../crates/scrutin-web/frontend/modules/levels.js
function pkgName() {
  return state.pkg?.name ?? "\u2014";
}
function selectedFile() {
  return state.selected ? state.files.get(state.selected) : null;
}
var LEVELS = {
  files: {
    id: "files",
    hidesLeftPane: false,
    renderLeft: (ctx) => ctx.renderFileList(),
    renderRight: (ctx) => ctx.renderTestListRight(),
    cursor: (n) => moveFileSelection(n),
    onEnter: () => enterDetail(),
    onPop: () => {
    },
    segments: () => [{ label: pkgName(), level: "files" }],
    counter: () => "",
    yankMessage: () => {
      const f = selectedFile();
      if (!f || !f.messages) return "";
      const bad = f.messages.find(isBadOutcome);
      return bad ? bad.message ?? "" : f.path ?? "";
    }
  },
  detail: {
    id: "detail",
    hidesLeftPane: false,
    renderLeft: (ctx) => ctx.renderTestListLeft(),
    renderRight: (ctx) => ctx.renderTestDetail(),
    cursor: (n) => moveTestSelection(n),
    onEnter: () => {
      const m = state.testFiltered[state.testCursor];
      if (m && isBadOutcome(m)) enterFailure();
    },
    onPop: () => exitDetail(),
    segments: () => {
      const f = selectedFile();
      return [
        { label: pkgName(), level: "files" },
        { label: f?.name ?? "\u2014", level: "detail" }
      ];
    },
    counter: () => "",
    yankMessage: () => state.testFiltered[state.testCursor]?.message ?? ""
  },
  failure: {
    id: "failure",
    hidesLeftPane: true,
    renderLeft: () => {
    },
    renderRight: (ctx) => ctx.renderFailureDetail(),
    cursor: (n) => moveFailureSelection(n),
    onEnter: () => {
    },
    // leaf
    onPop: () => exitFailure(),
    segments: () => {
      const f = selectedFile();
      const segs = [
        { label: pkgName(), level: "files" },
        { label: f?.name ?? "\u2014", level: "detail" }
      ];
      const ff = state.failures[state.failureCursor];
      if (ff) segs.push({ label: ff.test, level: "failure" });
      return segs;
    },
    counter: () => {
      const n = state.failures.length;
      if (n === 0) return "";
      return `(${state.failureCursor + 1}/${n})`;
    },
    yankMessage: () => state.failures[state.failureCursor]?.message ?? ""
  }
};
var currentLevel = () => LEVELS[state.level];

// ../../crates/scrutin-web/frontend/modules/sources.js
function renderSourceRows(src) {
  const start = src.start_line ?? 1;
  const hl = src.highlight_line;
  return src.lines.map((line, i) => {
    const lno = start + i;
    const cls = lno === hl ? "source-row highlight" : "source-row";
    return `<div class="${cls}"><span class="gutter">${lno}</span><span class="code">${line}</span></div>`;
  }).join("");
}
var LOADING_ROW = '<div class="source-row"><span class="gutter"></span><span class="code">loading\u2026</span></div>';
var UNAVAILABLE_ROW = '<div class="source-row"><span class="gutter"></span><span class="code">(source unavailable)</span></div>';
var NO_MAPPING_ROW = '<div class="source-row"><span class="gutter"></span><span class="code">(no source mapping)</span></div>';
var sourcePlaceholder = () => LOADING_ROW;
function renderTestSourceInto(elementId, fileId, line) {
  fetchSource(fileId, line).then((src) => {
    const el = $(elementId);
    if (!el) return;
    el.innerHTML = src ? renderSourceRows(src) : UNAVAILABLE_ROW;
  });
}
function renderFnSourceInto(elementId, fileId, onPath) {
  fetchSourceFor(fileId).then((src) => {
    const el = $(elementId);
    if (!el) return;
    if (src) {
      el.innerHTML = renderSourceRows(src);
      if (onPath) onPath(src.path);
    } else {
      el.innerHTML = NO_MAPPING_ROW;
    }
  });
}
function wireEditButtons(container, fileId, line) {
  container.querySelectorAll("[data-edit]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.edit === "source") openSourceInEditor();
      else openInEditor(fileId, line);
    });
  });
}

// ../../crates/scrutin-web/frontend/modules/palettes.js
function togglePalette(id, title, rows, footer, onClick) {
  const existing = $(id);
  if (existing) {
    closePalette(id);
    return;
  }
  const pal = document.createElement("div");
  pal.id = id;
  pal.className = "overlay-palette";
  const items = rows.map((r, i) => {
    const cls = [
      "pal-row",
      r.enabled === false ? "disabled" : "",
      r.active ? "active" : "",
      i === 0 ? "cursor" : ""
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
      cursor = (cursor + 1) % n;
      updateCursor();
      e.preventDefault();
      e.stopPropagation();
    } else if (k === "k" || k === "ArrowUp") {
      cursor = (cursor - 1 + n) % n;
      updateCursor();
      e.preventDefault();
      e.stopPropagation();
    } else if (k === "Enter") {
      selectCurrent();
      e.preventDefault();
      e.stopPropagation();
    }
  };
  window.addEventListener("keydown", onKey, true);
  pal._onKey = onKey;
  pal.addEventListener("click", (e) => {
    const row = e.target.closest(".pal-row");
    if (!row || row.classList.contains("disabled")) {
      close();
      return;
    }
    close();
    onClick(row.dataset.id);
  });
}
function closePalette(id) {
  const pal = $(id);
  if (!pal) return;
  if (pal._onKey) window.removeEventListener("keydown", pal._onKey, true);
  pal.remove();
}
function toggleSortPalette() {
  togglePalette(
    "sort-palette",
    "Sort",
    SORT_OPTIONS.map((o) => ({
      id: o.id,
      label: o.label,
      desc: o.desc + (o.id === state.sortMode ? state.sortReversed ? " \u2193" : " \u2191" : ""),
      active: o.id === state.sortMode
    })),
    "click to select \xB7 click active to reverse \xB7 s or Esc to close",
    (picked) => {
      if (picked === state.sortMode) {
        state.sortReversed = !state.sortReversed;
      } else {
        state.sortMode = picked;
        state.sortReversed = false;
      }
      renderFilterList();
      renderLeftPane();
    }
  );
}
var closeSortPalette = () => closePalette("sort-palette");
function toggleRunPalette() {
  if (state.currentRun?.in_progress) return;
  const nAll = state.fileOrder.length;
  const nVisible = state.filtered.length;
  const nSelected = state.multiSelected.size;
  const nFailed = state.currentRun?.bad_files?.length ?? 0;
  togglePalette(
    "run-palette",
    "Run",
    [
      { id: "all", label: "all", desc: `${nAll} file${nAll === 1 ? "" : "s"}`, enabled: nAll > 0 },
      { id: "visible", label: "visible", desc: `${nVisible} file${nVisible === 1 ? "" : "s"}`, enabled: nVisible > 0 },
      { id: "selected", label: "selected", desc: `${nSelected} file${nSelected === 1 ? "" : "s"}`, enabled: nSelected > 0 },
      { id: "failed", label: "failed", desc: `${nFailed} file${nFailed === 1 ? "" : "s"}`, enabled: nFailed > 0 }
    ],
    "click to run \xB7 r or Esc to close",
    (picked) => {
      if (picked === "all") runAll();
      else if (picked === "visible") runVisible();
      else if (picked === "selected") runMultiSelected();
      else if (picked === "failed") rerunFailing();
    }
  );
}
var closeRunPalette = () => closePalette("run-palette");
function toggleTestSortPalette() {
  const opts = SORT_OPTIONS.filter((o) => o.id !== "suite");
  togglePalette(
    "sort-palette",
    "Sort tests",
    opts.map((o) => ({
      id: o.id,
      label: o.label,
      desc: o.desc + (o.id === state.testSortMode ? state.testSortReversed ? " \u2193" : " \u2191" : ""),
      active: o.id === state.testSortMode
    })),
    "click to select \xB7 click active to reverse \xB7 s or Esc to close",
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
    }
  );
}

// ../../crates/scrutin-web/frontend/modules/render.js
function renderAll() {
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
function renderHeader() {
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
function renderBreadcrumb() {
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
      sep.textContent = "\u203A";
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
function renderHints() {
  const el = $("keyboard-hint");
  if (!el) return;
  const running = state.currentRun && state.currentRun.in_progress;
  const level = state.level;
  const seen = /* @__PURE__ */ new Set();
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
  el.innerHTML = parts.join(" <span class='sep'>\xB7</span> ");
}
function populatePluginDropdown() {
  const sel = $("plugin-select");
  if (!sel) return;
  const suitesInUse = /* @__PURE__ */ new Set();
  for (const [, f] of state.files) if (f.suite) suitesInUse.add(f.suite);
  sel.style.display = suitesInUse.size > 1 ? "" : "none";
  const existing = new Set(Array.from(sel.options).map((o) => o.value));
  const want = /* @__PURE__ */ new Set(["", ...(state.pkg?.suites ?? []).map((s) => s.name)]);
  if (existing.size !== want.size || [...existing].some((v) => !want.has(v))) {
    sel.innerHTML = "";
    const allOpt = document.createElement("option");
    allOpt.value = "";
    allOpt.textContent = "all";
    sel.appendChild(allOpt);
    for (const s of state.pkg?.suites ?? []) {
      const o = document.createElement("option");
      o.value = s.name;
      o.textContent = s.name;
      sel.appendChild(o);
    }
  }
  sel.value = state.pluginFilter;
}
function renderColHeaders() {
  const hdr = $("col-headers");
  if (!hdr) return;
  const suitesInUse = /* @__PURE__ */ new Set();
  for (const [, f] of state.files) if (f.suite) suitesInUse.add(f.suite);
  const multiSuite = suitesInUse.size > 1;
  hdr.className = multiSuite ? "file-row col-header" : "file-row col-header no-suite";
  const btn = (label, mode, compact) => {
    const active = state.sortMode === mode;
    const arrow = active && !compact ? state.sortReversed ? " \u2193" : " \u2191" : "";
    return `<button class="col-btn ${active ? "active" : ""}" data-sort="${mode}">${label}<span class="col-arrow">${arrow}</span></button>`;
  };
  hdr.innerHTML = `
    ${btn("\u25CF", "status", true)}
    ${btn("name", "name")}
    ${multiSuite ? btn("suite", "suite") : ""}
    ${btn("time", "time")}
  `;
  hdr.querySelectorAll(".col-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const mode = b.dataset.sort;
      if (mode === state.sortMode) state.sortReversed = !state.sortReversed;
      else {
        state.sortMode = mode;
        state.sortReversed = false;
      }
      renderColHeaders();
      renderFilterList();
      renderLeftPane();
    });
  });
}
function renderFilterList() {
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
      case "name":
        cmp = (fa?.name ?? "").localeCompare(fb?.name ?? "");
        break;
      case "suite":
        cmp = (fa?.suite ?? "").localeCompare(fb?.suite ?? "");
        break;
      case "status":
        cmp = fileStatusRank(fa) - fileStatusRank(fb);
        break;
      case "time":
        cmp = (fb?.last_duration_ms ?? -1) - (fa?.last_duration_ms ?? -1);
        break;
      default:
        cmp = 0;
        break;
    }
    return cmp !== 0 ? cmp : tiebreak(a, b);
  });
  if (state.sortReversed) filtered.reverse();
  state.filtered = filtered;
}
function cyclePlugin(delta) {
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
function cycleStatus(delta) {
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
function renderLeftPane() {
  const layout = $("layout");
  if (layout) layout.classList.toggle("failure-view", LEVELS.failure === currentLevel());
  currentLevel().renderLeft({
    renderFileList,
    renderTestListLeft
  });
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
  const suitesInUse = /* @__PURE__ */ new Set();
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
      else {
        clearMultiSelect();
        selectFile(id);
        state.lastClicked = id;
      }
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
  const pct = Math.max(4, Math.round(ms / maxMs * 100));
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
function renderRightPane() {
  const body = $("right-body");
  if (body) body.classList.toggle("failure-body", state.level === "failure");
  currentLevel().renderRight({
    renderTestListRight,
    renderTestDetail,
    renderFailureDetail
  });
}
function renderTestListRight() {
  const body = $("right-body");
  if (!body) return;
  if (!state.selected) {
    body.innerHTML = '<div class="detail-empty">select a file from the list</div>';
    return;
  }
  const f = state.files.get(state.selected);
  if (!f) {
    body.innerHTML = "";
    return;
  }
  const header = `<div class="detail-file-header">
    <h2>${escapeHtml(f.name)}</h2>
    <span class="status-pill ${displayStatus(f)}">${displayStatus(f)}</span>
    <button class="edit-btn" data-edit="test" title="Edit test file (e)">edit test</button>
    <button class="edit-btn" data-edit="source" title="Edit source file (E)">edit source</button>
  </div>`;
  if (!f.messages || f.messages.length === 0) {
    body.innerHTML = header + '<div class="detail-empty">no test messages yet \u2014 run this file to see results.</div>';
    wireEditButtons(body, f.id);
    return;
  }
  const sorted = sortMessages(f.messages);
  const rows = sorted.map((m, idx) => {
    const name = m.test_name ?? "<anon>";
    const dur = m.duration_ms != null ? formatMs(m.duration_ms) : "";
    const showMsg = m.message && (isBadOutcome(m) || m.outcome === "warn");
    const msgPreview = showMsg ? `<div class="test-row-message ${m.outcome}">${escapeHtml(m.message.split("\n")[0].slice(0, 120))}</div>` : "";
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
  const sortLink = body.querySelector(".test-sort-link");
  if (sortLink) sortLink.addEventListener("click", (e) => {
    e.preventDefault();
    toggleTestSortPalette();
  });
  body.querySelectorAll(".test-row").forEach((el) => {
    el.addEventListener("click", () => {
      state.testCursor = parseInt(el.dataset.idx, 10);
      enterDetail();
    });
  });
}
function renderTestDetail() {
  const body = $("right-body");
  if (!body) return;
  const f = state.files.get(state.selected);
  if (!f) return;
  const tests = state.testFiltered;
  const m = tests[state.testCursor];
  if (!m) {
    body.innerHTML = '<div class="detail-empty">no test selected</div>';
    return;
  }
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
  const correction = m.corrections && m.corrections[0];
  if (correction) {
    const chips = (correction.suggestions || []).slice(0, 9).map((sug, i) => {
      const n = i + 1;
      const best = i === 0 ? " suggestion--best" : "";
      const star = i === 0 ? " \u2605" : "";
      return `<button class="suggestion${best}" data-suggestion="${n}">
          <span class="suggestion-key">[${n}]</span>
          <span class="suggestion-word">${escapeHtml(sug)}${star}</span>
        </button>`;
    }).join("");
    html += `<div class="detail-section">
      <div class="detail-section-header">Replace with</div>
      <div class="suggestion-grid">${chips}</div>
      <button class="suggestion suggestion--dict" data-suggestion="0">
        <span class="suggestion-key">[0]</span>
        <span class="suggestion-word">Add \u201C${escapeHtml(correction.word)}\u201D to dictionary</span>
      </button>
    </div>`;
    html += `<div class="detail-section">
      <div class="detail-section-header">
        Context
        <button class="edit-btn" data-edit="test" title="Edit file (e)">edit</button>
      </div>
      <div class="source-snippet" id="detail-test-source">${sourcePlaceholder()}</div>
    </div>`;
    body.innerHTML = html;
    wireEditButtons(body, f.id, m.location?.line);
    $("focus-failure-btn")?.addEventListener("click", () => enterFailure());
    renderTestSourceInto("detail-test-source", f.id, m.location?.line);
    body.querySelectorAll("[data-suggestion]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const n = Number(btn.dataset.suggestion);
        if (n === 0) {
          applyCorrection(f.id, correction, null);
        } else {
          const replacement = correction.suggestions[n - 1];
          if (replacement != null) applyCorrection(f.id, correction, replacement);
        }
      });
    });
    return;
  }
  const suite = f ? (state.pkg?.suites ?? []).find((s) => s.name === f.suite) : null;
  const actions = suite?.actions ?? [];
  if (actions.length > 0) {
    const chips = actions.slice(0, 9).map((a, i) => {
      const n = i + 1;
      return `<button class="suggestion" data-action="${escapeHtml(a.name)}">
          <span class="suggestion-key">[${n}]</span>
          <span class="suggestion-word">${escapeHtml(a.label)}</span>
        </button>`;
    }).join("");
    html += `<div class="detail-section">
      <div class="detail-section-header">Actions</div>
      <div class="suggestion-grid">${chips}</div>
    </div>`;
  }
  if (m.metrics) {
    html += `<div class="detail-section">
      <div class="detail-section-header">Metrics</div>
      <div style="font-size: 12px; color: var(--fg-dim);">${formatMetrics(m.metrics)}</div>
    </div>`;
  }
  const isWarn = m.outcome === "warn";
  if (isWarn) {
    html += `<div class="detail-section">
      <div class="detail-section-header">
        Context
        <button class="edit-btn" data-edit="test" title="Edit file (e)">edit</button>
      </div>
      <div class="source-snippet" id="detail-test-source">${sourcePlaceholder()}</div>
    </div>`;
  } else {
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
  }
  body.innerHTML = html;
  wireEditButtons(body, f.id, m.location?.line);
  body.querySelectorAll("[data-action]").forEach((btn) => {
    btn.addEventListener("click", () => {
      runPluginAction(btn.dataset.action, f.id);
    });
  });
  $("focus-failure-btn")?.addEventListener("click", () => enterFailure());
  renderTestSourceInto("detail-test-source", f.id, m.location?.line);
  if (!isWarn) {
    renderFnSourceInto("detail-source-fn", f.id, (path) => {
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
  $("failure-next")?.addEventListener("click", () => moveFailureSelection(1));
  wireEditButtons(body, ff.fileId, ff.line);
  renderTestSourceInto("failure-test-source", ff.fileId, ff.line);
  renderFnSourceInto("failure-source-fn", ff.fileId, (path) => {
    const hdr = $("failure-source-title");
    if (hdr) hdr.textContent = `Source \u2014 ${path}`;
  });
}
var COUNT_ICONS = { pass: "\u25CF", fail: "\u2717", error: "\u26A0", warn: "\u26A1", skip: "\u2298", xfail: "\u2299" };
function renderCounts() {
  const t = state.totals;
  setCount("pass", t.pass);
  setCount("fail", t.fail);
  setCount("error", t.error);
  setCount("warn", t.warn);
  setCount("skip", t.skip);
  setCount("xfail", t.xfail);
}
function setCount(name, n) {
  const el = document.querySelector(`#countsbar .count.${name}`);
  if (el) el.textContent = `${COUNT_ICONS[name] || ""}${n}`;
}
function setStatus(s) {
  const el = $("status-text");
  if (el) el.textContent = s;
}
function renderControls() {
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
    visBtn.textContent = `\u25B6 run visible (${n})`;
    visBtn.disabled = running || n === 0;
  }
  const selBtn = $("btn-run-selected");
  if (selBtn) {
    const n = state.multiSelected.size;
    selBtn.textContent = n > 0 ? `\u25B6 run selected (${n})` : `\u25B6 run selected`;
    selBtn.disabled = running || !state.selected;
  }
}

// ../../crates/scrutin-web/frontend/modules/api.js
async function postJSON(path, body) {
  try {
    const res = await fetch(`${BASE}${path}`, {
      method: "POST",
      headers: body ? { "Content-Type": "application/json" } : {},
      body: body ? JSON.stringify(body) : void 0
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
async function fetchSnapshot() {
  try {
    const res = await fetch(`${BASE}/api/snapshot`);
    if (!res.ok) throw new Error(`snapshot http ${res.status}`);
    const snap = await res.json();
    state.pkg = snap.pkg;
    state.files = /* @__PURE__ */ new Map();
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
    if (Array.isArray(snap.outcome_order)) {
      const m = {};
      snap.outcome_order.forEach((o, i) => {
        m[o] = i;
      });
      setOutcomeRanks(m);
    }
    if (!state.selected && state.fileOrder.length > 0) {
      state.selected = state.fileOrder[0];
    }
  } catch (e) {
    toast(`snapshot failed: ${e}`, true);
  }
}
async function fetchSource(fileId, line) {
  const hasLine = line != null;
  const key = hasLine ? `${fileId}:${line}` : `${fileId}:top`;
  if (state.sourceCache.has(key)) return state.sourceCache.get(key);
  try {
    const url = hasLine ? `${BASE}/api/file/${fileId}/source?line=${line}&context=8` : `${BASE}/api/file/${fileId}/source`;
    const res = await fetch(url);
    if (!res.ok) return null;
    const data = await res.json();
    state.sourceCache.set(key, data);
    return data;
  } catch (_) {
    return null;
  }
}
async function fetchSourceFor(fileId) {
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
async function runAll() {
  setStatus("starting run\u2026");
  await postJSON("/api/run");
}
async function runVisible() {
  if (state.filtered.length === 0) {
    toast("nothing visible to run", true);
    return;
  }
  if (state.currentRun?.in_progress) return;
  setStatus(`running ${state.filtered.length} visible file${state.filtered.length === 1 ? "" : "s"}\u2026`);
  await postJSON("/api/rerun", { files: state.filtered.map(String) });
}
async function cancelRun() {
  if (!state.currentRun?.in_progress) return;
  setStatus("cancelling\u2026");
  await postJSON("/api/cancel");
}
async function rerunFailing() {
  if (!state.currentRun?.bad_files?.length) return;
  setStatus("rerunning failing\u2026");
  await postJSON("/api/rerun-failing");
}
async function runMultiSelected() {
  if (state.multiSelected.size === 0) return;
  const ids = [...state.multiSelected];
  setStatus(`running ${ids.length} selected file${ids.length === 1 ? "" : "s"}\u2026`);
  await postJSON("/api/rerun", { files: ids.map(String) });
}
async function rerunSelected() {
  if (state.multiSelected.size > 0) {
    runMultiSelected();
    return;
  }
  if (!state.selected) return;
  setStatus("rerunning selected\u2026");
  await postJSON("/api/rerun", { files: [String(state.selected)] });
}
async function toggleWatch() {
  state.watching = !state.watching;
  await postJSON("/api/watch", { enabled: state.watching });
}
async function runPluginAction(actionName, fileId) {
  const id = fileId ?? state.selected;
  if (!id) return;
  const res = await postJSON("/api/suite-action", { file_id: id, action: actionName });
  if (res !== null) {
    const label = actionName.replace(/_/g, " ");
    toast(res.rerun ? `${label}: done, re-running` : `${label}: done`);
  }
}
async function applyCorrection(fileId, correction, replacement) {
  const body = {
    file_id: String(fileId),
    word: correction.word,
    line: correction.line,
    col_start: correction.col_start,
    col_end: correction.col_end
  };
  if (replacement != null) body.replacement = replacement;
  const res = await postJSON("/api/correction", body);
  if (res !== null) {
    toast(res.message ?? "correction applied");
  }
}
async function openInEditor(fileId, line) {
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
async function openSourceInEditor() {
  if (!state.selected) return;
  const src = await fetchSourceFor(state.selected);
  if (!src || !src.path) {
    toast("no source mapping found", true);
    return;
  }
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

// ../../crates/scrutin-web/frontend/modules/events.js
var es = null;
var reconnectDelay = 500;
function connectEvents() {
  if (es) {
    try {
      es.close();
    } catch (_) {
    }
  }
  es = new EventSource(`${BASE}/api/events`);
  es.onopen = () => {
    reconnectDelay = 500;
  };
  es.onerror = () => {
    if (es) {
      try {
        es.close();
      } catch (_) {
      }
      es = null;
    }
    toast(`disconnected \u2014 reconnecting in ${Math.round(reconnectDelay)}ms`, true);
    setTimeout(connectEvents, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 1e4);
  };
  const kinds = [
    "run_started",
    "file_started",
    "file_finished",
    "run_complete",
    "run_cancelled",
    "watcher_triggered",
    "log",
    "heartbeat"
  ];
  for (const k of kinds) {
    es.addEventListener(k, (ev) => {
      try {
        apply(k, JSON.parse(ev.data));
      } catch (e) {
        console.error("event parse", k, e);
      }
    });
  }
}
function apply(kind, data) {
  switch (kind) {
    case "run_started":
      state.currentRun = {
        run_id: data.run_id,
        started_at: data.started_at,
        finished_at: null,
        in_progress: true,
        totals: { pass: 0, fail: 0, error: 0, skip: 0, xfail: 0, warn: 0 },
        bad_files: []
      };
      state.totals = state.currentRun.totals;
      state.sourceCache.clear();
      setStatus("running");
      for (const fid of data.files) {
        const f = state.files.get(fid);
        if (f) {
          f.status = "pending";
          f.counts = { pass: 0, fail: 0, error: 0, skip: 0, xfail: 0, warn: 0 };
          f.messages = [];
          f.bad = false;
        }
      }
      updateTestFiltered();
      renderAll();
      break;
    case "file_started": {
      const f = state.files.get(data.file_id);
      if (f) {
        f.status = "running";
        renderFilterList();
        renderLeftPane();
        if (state.selected === data.file_id) renderRightPane();
      }
      break;
    }
    case "file_finished": {
      const wf = data.file;
      state.files.set(wf.id, wf);
      if (!state.fileOrder.includes(wf.id)) state.fileOrder.push(wf.id);
      const c = wf.counts;
      state.totals.pass += c.pass;
      state.totals.fail += c.fail;
      state.totals.error += c.error;
      state.totals.skip += c.skip;
      state.totals.xfail += c.xfail;
      state.totals.warn += c.warn;
      if (wf.bad && state.currentRun && !state.currentRun.bad_files.includes(wf.id)) {
        state.currentRun.bad_files.push(wf.id);
      }
      renderFilterList();
      renderCounts();
      if (state.selected === wf.id) updateTestFiltered();
      renderLeftPane();
      if (state.selected === wf.id) renderRightPane();
      renderControls();
      break;
    }
    case "run_complete":
      if (state.currentRun) {
        state.currentRun.in_progress = false;
        state.currentRun.finished_at = data.finished_at;
        state.currentRun.totals = data.totals;
        state.currentRun.bad_files = data.bad_files;
      }
      state.totals = data.totals;
      setStatus(`done \xB7 ${data.totals.pass} pass \xB7 ${data.totals.fail + data.totals.error} bad`);
      renderCounts();
      renderControls();
      break;
    case "run_cancelled":
      setStatus("cancelled");
      if (state.currentRun) state.currentRun.in_progress = false;
      renderControls();
      break;
    case "watcher_triggered":
      setStatus(`watcher: ${data.changed_files.length} files changed`);
      break;
    case "log":
      break;
    case "heartbeat":
      state.busy = data.busy ?? 0;
      if (state.currentRun) state.currentRun.in_progress = data.in_progress === true;
      renderHeader();
      break;
  }
}

// ../../crates/scrutin-web/frontend/modules/help.js
function toggleHelp(force) {
  const el = $("help");
  if (!el) return;
  if (typeof force === "boolean") el.classList.toggle("hidden", !force);
  else el.classList.toggle("hidden");
  const dl = $("help-bindings");
  if (!dl || state.keymap.length === 0) return;
  const helpRow = (b) => {
    const sp = b.help.indexOf(" ");
    const key = sp > 0 ? b.help.slice(0, sp) : b.help;
    const desc = sp > 0 ? b.help.slice(sp + 1) : "";
    return `<dt>${escapeHtml(key)}</dt><dd>${escapeHtml(desc)}</dd>`;
  };
  let html = "";
  const seen = /* @__PURE__ */ new Set();
  for (const b of state.keymap) {
    if (!b.help || seen.has(b.action)) continue;
    seen.add(b.action);
    html += helpRow(b);
  }
  dl.innerHTML = html;
}

// ../../crates/scrutin-web/frontend/modules/theme.js
function toggleTheme() {
  const cur = document.documentElement.getAttribute("data-theme") || "dark";
  const next = cur === "dark" ? "light" : "dark";
  document.documentElement.setAttribute("data-theme", next);
  try {
    localStorage.setItem("scrutin-theme", next);
  } catch (_) {
  }
}
function applyStoredTheme() {
  let theme = "dark";
  const param = new URLSearchParams(window.location.search).get("theme");
  if (param === "light" || param === "dark") {
    theme = param;
  } else {
    try {
      const stored = localStorage.getItem("scrutin-theme");
      if (stored === "light" || stored === "dark") theme = stored;
    } catch (_) {
    }
  }
  document.documentElement.setAttribute("data-theme", theme);
}
function applyStoredSidebarWidth() {
  try {
    const stored = localStorage.getItem("scrutin-sidebar-w");
    if (stored) {
      const n = parseInt(stored, 10);
      if (Number.isFinite(n) && n >= 240 && n <= 900) {
        document.documentElement.style.setProperty("--sidebar-w", `${n}px`);
      }
    }
  } catch (_) {
  }
}
function wireSidebarResize() {
  const resizer = $("pane-resizer");
  if (!resizer) return;
  const MIN_W = 240;
  const MAX_W = 900;
  let dragging = false;
  const isHorizontal = () => $("layout")?.classList.contains("horizontal");
  const onMove = (e) => {
    if (!dragging) return;
    const layout = $("layout");
    const rect = layout.getBoundingClientRect();
    if (isHorizontal()) {
      const y = e.clientY - rect.top;
      const clamped = Math.max(100, Math.min(y, rect.height - 100));
      document.documentElement.style.setProperty("--topbar-h", `${clamped}px`);
    } else {
      const x = e.clientX - rect.left;
      const clamped = Math.max(MIN_W, Math.min(MAX_W, x));
      document.documentElement.style.setProperty("--sidebar-w", `${clamped}px`);
    }
  };
  const onUp = () => {
    if (!dragging) return;
    dragging = false;
    resizer.classList.remove("active");
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    if (!isHorizontal()) {
      const w = getComputedStyle(document.documentElement).getPropertyValue("--sidebar-w").trim();
      const n = parseInt(w, 10);
      if (Number.isFinite(n)) {
        try {
          localStorage.setItem("scrutin-sidebar-w", String(n));
        } catch (_) {
        }
      }
    }
  };
  resizer.addEventListener("pointerdown", (e) => {
    dragging = true;
    resizer.classList.add("active");
    document.body.style.userSelect = "none";
    document.body.style.cursor = isHorizontal() ? "row-resize" : "col-resize";
    e.preventDefault();
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  });
}
function resizeSidebar(delta) {
  const layout = $("layout");
  const horiz = layout && layout.classList.contains("horizontal");
  const root = document.documentElement;
  if (horiz) {
    const h = getComputedStyle(root).getPropertyValue("--topbar-h").trim();
    const cur = parseInt(h, 10) || Math.round(window.innerHeight * 0.5);
    const clamped = Math.max(100, Math.min(cur + delta, window.innerHeight - 100));
    root.style.setProperty("--topbar-h", `${clamped}px`);
  } else {
    const w = getComputedStyle(root).getPropertyValue("--sidebar-w").trim();
    const cur = parseInt(w, 10) || 420;
    const clamped = Math.max(200, Math.min(cur + delta, window.innerWidth - 200));
    root.style.setProperty("--sidebar-w", `${clamped}px`);
    try {
      localStorage.setItem("scrutin-sidebar-w", String(clamped));
    } catch (_) {
    }
  }
}

// ../../crates/scrutin-web/frontend/modules/keymap.js
function browserKeyToKeymapKey(e) {
  if (e.ctrlKey && e.key.length === 1) return `Ctrl-${e.key}`;
  switch (e.key) {
    case "ArrowUp":
      return "Up";
    case "ArrowDown":
      return "Down";
    case "ArrowLeft":
      return "Left";
    case "ArrowRight":
      return "Right";
    case " ":
      return "Space";
    case "Escape":
      return "Esc";
    default:
      return e.key;
  }
}
function resolveAction(keyStr, level) {
  const help = $("help");
  const sortPal = $("sort-palette");
  const runPal = $("run-palette");
  if (help && !help.classList.contains("hidden") || sortPal || runPal) {
    if (["Esc", "q", "?", "r", "s"].includes(keyStr)) return "pop";
    return null;
  }
  for (const b of state.keymap) {
    if (b.key === keyStr && b.levels.includes(level)) return b.action;
  }
  return null;
}
var ACTION_HANDLERS = {
  // Navigation \u2192 delegated to the current level handler.
  cursor_down: () => currentLevel().cursor(1),
  cursor_up: () => currentLevel().cursor(-1),
  cursor_top: () => currentLevel().cursor(-Infinity),
  cursor_bottom: () => currentLevel().cursor(Infinity),
  enter: (e) => {
    currentLevel().onEnter();
    e?.preventDefault();
  },
  pop: () => {
    const help = $("help");
    if (help && !help.classList.contains("hidden")) {
      toggleHelp(false);
      return;
    }
    if ($("run-palette")) {
      closeRunPalette();
      return;
    }
    if ($("sort-palette")) {
      closeSortPalette();
      return;
    }
    if (state.multiSelected.size > 0) {
      clearMultiSelect();
      return;
    }
    currentLevel().onPop();
  },
  quit: () => {
  },
  // Run control.
  open_run_menu: () => toggleRunPalette(),
  run_current_file: () => rerunSelected(),
  cancel_file: () => cancelRun(),
  cancel_all: () => cancelRun(),
  // Filtering.
  filter_name: (e) => {
    e?.preventDefault();
    $("filter-input")?.focus();
  },
  filter_status: () => cycleStatus(1),
  filter_status_back: () => cycleStatus(-1),
  filter_tool: () => cyclePlugin(1),
  filter_tool_back: () => cyclePlugin(-1),
  open_sort_menu: () => toggleSortPalette(),
  // Actions \u2192 also delegated to the current level handler.
  edit_test: () => openInEditor(),
  edit_source: () => openSourceInEditor(),
  yank_message: () => {
    const msg = currentLevel().yankMessage();
    if (!msg) return;
    navigator.clipboard.writeText(msg).then(
      () => toast("copied to clipboard"),
      () => toast("clipboard access denied", true)
    );
  },
  enter_log: () => {
  },
  enter_help: () => toggleHelp(),
  toggle_select: () => {
    if (state.selected) toggleMultiSelect(state.selected);
  },
  toggle_visual: () => {
  },
  shrink_list: () => resizeSidebar(-40),
  grow_list: () => resizeSidebar(40),
  toggle_orientation: () => {
    const layout = $("layout");
    if (layout) layout.classList.toggle("horizontal");
  },
  // J/K scroll the right pane (source/detail view) by one "line".
  // Mirrors the TUI's per-pane scroll without grabbing the cursor.
  source_scroll_down: () => scrollRightPane(1),
  source_scroll_up: () => scrollRightPane(-1)
};
function scrollRightPane(sign) {
  const root = document.getElementById("right-pane");
  if (!root) return;
  const step = sign * 40;
  const candidates = [
    document.querySelector("#right-body"),
    ...root.querySelectorAll(".detail-body, .test-list, .failure-body"),
    root
  ];
  for (const el of candidates) {
    if (!el) continue;
    if (el.scrollHeight > el.clientHeight) {
      el.scrollTop = Math.max(0, Math.min(el.scrollTop + step, el.scrollHeight - el.clientHeight));
      return;
    }
  }
}
function dispatchAction(action, e) {
  const handler = ACTION_HANDLERS[action];
  if (handler) handler(e);
}
var BLOCKED_EDITOR_KEYS = /* @__PURE__ */ new Set(["Ctrl-f", "Ctrl-b"]);
function wireKeyboard() {
  window.addEventListener("keydown", (e) => {
    const tag = e.target.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") {
      if (e.key === "Escape") e.target.blur();
      return;
    }
    const keyStr = browserKeyToKeymapKey(e);
    if (!keyStr) return;
    if (IS_EDITOR && BLOCKED_EDITOR_KEYS.has(keyStr)) return;
    const level = state.level;
    if (level === "detail" && /^[0-9]$/.test(e.key)) {
      const tests = state.testFiltered ?? [];
      const m = tests[state.testCursor];
      const correction = m?.corrections?.[0];
      const n = Number(e.key);
      if (correction) {
        e.preventDefault();
        if (n === 0) {
          applyCorrection(state.selected, correction, null);
        } else {
          const replacement = correction.suggestions?.[n - 1];
          if (replacement != null) applyCorrection(state.selected, correction, replacement);
        }
        return;
      }
      if (n > 0 && state.selected) {
        const f = state.files.get(state.selected);
        const suite = f ? (state.pkg?.suites ?? []).find((s) => s.name === f.suite) : null;
        const action2 = suite?.actions?.[n - 1];
        if (action2) {
          e.preventDefault();
          runPluginAction(action2.name, state.selected);
          return;
        }
      }
    }
    const action = resolveAction(keyStr, level);
    if (action) {
      dispatchAction(action, e);
      return;
    }
  });
}

// ../../crates/scrutin-web/frontend/app.js
document.addEventListener("DOMContentLoaded", () => {
  applyStoredTheme();
  applyStoredSidebarWidth();
  wireSidebarResize();
  const on = (id, ev, fn) => {
    const el = $(id);
    if (el) el.addEventListener(ev, fn);
  };
  on("btn-run-all", "click", runAll);
  on("btn-run-visible", "click", runVisible);
  on("btn-run-selected", "click", rerunSelected);
  on("btn-cancel", "click", cancelRun);
  on("btn-rerun-failing", "click", rerunFailing);
  on("toggle-watch", "change", toggleWatch);
  on("btn-theme", "click", toggleTheme);
  on("btn-back", "click", () => {
    if (state.level === "failure") exitFailure();
    else if (state.level === "detail") exitDetail();
  });
  on("filter-input", "input", (e) => {
    state.filterText = e.target.value;
    renderFilterList();
    renderLeftPane();
    renderControls();
  });
  on("plugin-select", "change", (e) => {
    state.pluginFilter = e.target.value;
    renderFilterList();
    renderLeftPane();
    renderControls();
  });
  on("status-select", "change", (e) => {
    state.statusFilter = e.target.value;
    renderFilterList();
    renderLeftPane();
    renderControls();
  });
  wireKeyboard();
  fetch(`${BASE}/syntect.css`).then((r) => r.ok ? r.text() : "").then((css) => {
    if (!css) return;
    const style = document.createElement("style");
    style.textContent = css;
    document.head.appendChild(style);
  }).catch(() => {
  });
  fetchSnapshot().then(() => {
    renderAll();
    connectEvents();
  });
});
