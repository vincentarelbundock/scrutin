// scrutin unified client
//
// Shared by the standalone web reporter and the VS Code / Positron
// webview. Auto-detects the environment:
//   - standalone: fetch("/api/..."), open-editor via POST
//   - VS Code:    fetch(BASE + "/api/..."), open-editor via postMessage
//
// Contract: GET /api/snapshot on load, then an EventSource on
// /api/events keeps things up-to-date. Everything flows through a
// reducer-style apply() so the render layer always sees a consistent
// snapshot.
//
// Navigation model (two-level shifting master-detail):
//   Level 1 ("files"):  left = file list,  right = test list for highlighted file
//   Level 2 ("detail"): left = test list,  right = scrollable detail for highlighted test

const IS_VSCODE = typeof acquireVsCodeApi === "function";
const vscode = IS_VSCODE ? acquireVsCodeApi() : null;
const BASE = IS_VSCODE ? (window.__SCRUTIN_BASE_URL__ || "http://127.0.0.1:7878") : "";
const IS_EDITOR = IS_VSCODE; // extend for RStudio, other editors as needed

const state = {
  pkg: null,
  files: new Map(),       // FileId -> WireFile
  fileOrder: [],          // stable display order
  filtered: [],           // visible slice after all filters
  selected: null,         // FileId (highlighted file)
  currentRun: null,       // WireRunSummary
  watching: false,
  nWorkers: 1,
  busy: 0,
  filterText: "",
  pluginFilter: "",       // "" = all, else suite name
  statusFilter: "",       // "" = all, else WireStatus
  sortMode: "status",     // sequential | status | name | plugin | time
  sortReversed: false,
  testSortMode: "sticky",
  testSortReversed: false,
  totals: { pass: 0, fail: 0, error: 0, skip: 0, xfail: 0, warn: 0 },
  sourceCache: new Map(),
  // Navigation state
  level: "files",         // "files" | "detail"
  testCursor: 0,          // index into sorted test list at current level
  testFiltered: [],       // sorted messages for the selected file
};

const STATUS_CYCLE = ["", "failed", "errored", "warned", "passed", "skipped", "running", "pending", "cancelled"];
const OUTCOME_RANK = { fail: 0, error: 1, warn: 2, pass: 3, skip: 4, xfail: 5 };

// ── Fetch + bootstrap ───────────────────────────────────────────────────────

async function fetchSnapshot() {
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
    if (!state.selected && state.fileOrder.length > 0) {
      state.selected = state.fileOrder[0];
    }
    renderAll();
  } catch (e) {
    toast(`snapshot failed: ${e}`, true);
  }
}

// ── SSE ─────────────────────────────────────────────────────────────────────

let es = null;
let reconnectDelay = 500;

function connectEvents() {
  if (es) { try { es.close(); } catch (_) {} }
  es = new EventSource(`${BASE}/api/events`);
  es.onopen = () => { reconnectDelay = 500; };
  es.onerror = () => {
    if (es) { try { es.close(); } catch (_) {} es = null; }
    toast(`disconnected — reconnecting in ${Math.round(reconnectDelay)}ms`, true);
    setTimeout(connectEvents, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 10000);
  };

  const kinds = [
    "run_started", "file_started", "file_finished",
    "run_complete", "run_cancelled", "watcher_triggered",
    "log", "heartbeat",
  ];
  for (const k of kinds) {
    es.addEventListener(k, (ev) => {
      try { apply(k, JSON.parse(ev.data)); }
      catch (e) { console.error("event parse", k, e); }
    });
  }
}

function apply(kind, data) {
  switch (kind) {
    case "run_started":
      state.currentRun = {
        run_id: data.run_id, started_at: data.started_at,
        finished_at: null, in_progress: true,
        totals: { pass: 0, fail: 0, error: 0, skip: 0, xfail: 0, warn: 0 },
        bad_files: [],
      };
      state.totals = state.currentRun.totals;
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
      renderAll();
      break;

    case "file_started": {
      const f = state.files.get(data.file_id);
      if (f) {
        f.status = "running";
        renderFilterList();
        renderLeftPane();
      }
      break;
    }

    case "file_finished": {
      const wf = data.file;
      state.files.set(wf.id, wf);
      if (!state.fileOrder.includes(wf.id)) state.fileOrder.push(wf.id);
      const c = wf.counts;
      state.totals.pass  += c.pass;
      state.totals.fail  += c.fail;
      state.totals.error += c.error;
      state.totals.skip  += c.skip;
      state.totals.xfail += c.xfail;
      state.totals.warn  += c.warn;
      if (wf.bad && state.currentRun && !state.currentRun.bad_files.includes(wf.id)) {
        state.currentRun.bad_files.push(wf.id);
      }
      renderFilterList();
      renderLeftPane();
      renderCounts();
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
      setStatus(`done · ${data.totals.pass} pass · ${data.totals.fail + data.totals.error} bad`);
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
      if (state.currentRun) {
        state.currentRun.in_progress = data.in_progress === true;
      }
      renderHeader();
      break;
  }
}

// ── Rendering ───────────────────────────────────────────────────────────────

function renderAll() {
  renderHeader();
  populatePluginDropdown();
  renderColHeaders();
  renderFilterList();
  renderLeftPane();
  renderRightPane();
  renderCounts();
  renderControls();
}

function $(id) { return document.getElementById(id); }

function renderHeader() {
  const pkgEl = $("pkg-name");
  if (pkgEl) pkgEl.textContent = state.pkg?.name ?? "—";
  const workersEl = $("workers");
  if (workersEl) {
    const busy = state.currentRun?.in_progress ? state.busy : 0;
    workersEl.textContent = `${busy}/${state.nWorkers} workers`;
  }
  const watch = $("toggle-watch");
  if (watch) watch.checked = state.watching;
}

function populatePluginDropdown() {
  const sel = $("plugin-select");
  const suitesInUse = new Set();
  for (const [, f] of state.files) { if (f.suite) suitesInUse.add(f.suite); }
  if (sel) {
    sel.style.display = suitesInUse.size > 1 ? "" : "none";
    const existing = new Set(Array.from(sel.options).map((o) => o.value));
    const want = new Set(["", ...(state.pkg?.suites ?? []).map((s) => s.name)]);
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
}

function renderColHeaders() {
  const hdr = $("col-headers");
  if (!hdr) return;
  const suitesInUse = new Set();
  for (const [, f] of state.files) { if (f.suite) suitesInUse.add(f.suite); }
  const multiSuite = suitesInUse.size > 1;
  hdr.className = multiSuite ? "file-row col-header" : "file-row col-header no-suite";

  const btn = (label, mode, compact) => {
    const active = state.sortMode === mode;
    const arrow = active && !compact ? (state.sortReversed ? " ↓" : " ↑") : "";
    return `<button class="col-btn ${active ? "active" : ""}" data-sort="${mode}">${label}<span class="col-arrow">${arrow}</span></button>`;
  };

  hdr.innerHTML = `
    ${btn("●", "status", true)}
    ${btn("name", "name")}
    ${multiSuite ? btn("suite", "suite") : ""}
    ${btn("time", "time")}
  `;
  hdr.querySelectorAll(".col-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const mode = b.dataset.sort;
      if (mode === state.sortMode) {
        state.sortReversed = !state.sortReversed;
      } else {
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
    if (q) {
      return f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q);
    }
    return true;
  });
  const origIdx = new Map(state.fileOrder.map((id, i) => [id, i]));
  const tiebreak = (a, b) => (origIdx.get(a) ?? 0) - (origIdx.get(b) ?? 0);
  const statusRank = (f) => {
    if (!f) return 99;
    switch (f.status) {
      case "errored": return 0;
      case "failed":  return 1;
      case "passed":  return (f.counts?.warn ?? 0) > 0 ? 2 : 6;
      case "running": return 3;
      case "cancelled": return 4;
      case "pending": return 5;
      case "skipped": return 7;
      default: return 8;
    }
  };
  filtered.sort((a, b) => {
    const fa = state.files.get(a);
    const fb = state.files.get(b);
    let cmp = 0;
    switch (state.sortMode) {
      case "name": cmp = (fa?.name ?? "").localeCompare(fb?.name ?? ""); break;
      case "plugin": cmp = (fa?.suite ?? "").localeCompare(fb?.suite ?? ""); break;
      case "status": cmp = statusRank(fa) - statusRank(fb); break;
      case "time": cmp = (fb?.last_duration_ms ?? -1) - (fa?.last_duration_ms ?? -1); break;
      default: cmp = 0; break;
    }
    return cmp !== 0 ? cmp : tiebreak(a, b);
  });
  if (state.sortReversed) filtered.reverse();
  state.filtered = filtered;
}

// ── Left pane ───────────────────────────────────────────────────────────────

function renderLeftPane() {
  if (state.level === "files") {
    renderFileList();
  } else {
    renderTestListLeft();
  }
  updateLeftTitle();
}

function updateLeftTitle() {
  const titleText = $("left-title-text");
  const subbar = $("left-subbar");
  const colHeaders = $("col-headers");
  const backBtn = $("btn-back");
  if (state.level === "files") {
    if (titleText) titleText.textContent = "Files";
    if (subbar) subbar.style.display = "";
    if (colHeaders) colHeaders.style.display = "";
    if (backBtn) backBtn.classList.add("hidden");
  } else {
    const f = state.files.get(state.selected);
    if (titleText) titleText.textContent = f ? `${f.name}` : "Tests";
    if (subbar) subbar.style.display = "none";
    if (colHeaders) colHeaders.style.display = "none";
    if (backBtn) backBtn.classList.remove("hidden");
  }
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
  for (const [, f] of state.files) { if (f.suite) suitesInUse.add(f.suite); }
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
    li.innerHTML = `
      <span class="status-dot ${f.status}"></span>
      <span class="fname">${escapeHtml(f.name)}</span>
      ${multiSuite ? `<span class="suite">${escapeHtml(f.suite)}</span>` : ""}
      ${renderDurationCell(f, maxMs)}
    `;
    li.addEventListener("click", () => selectFile(id));
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

// ── Right pane ──────────────────────────────────────────────────────────────

function renderRightPane() {
  if (state.level === "files") {
    renderTestListRight();
  } else {
    renderTestDetail();
  }
}

function renderTestListRight() {
  const title = $("right-title");
  const body = $("right-body");
  if (!title || !body) return;

  if (!state.selected) {
    title.textContent = "no file selected";
    body.innerHTML = '<div class="detail-empty">select a file from the list</div>';
    return;
  }
  const f = state.files.get(state.selected);
  if (!f) {
    title.textContent = "no file selected";
    body.innerHTML = "";
    return;
  }

  title.innerHTML = `<span>${escapeHtml(f.path)}</span>`;

  // Header with file name, status, and action buttons
  const actions = selectedSuiteActions();
  const actionBtns = actions.length > 0
    ? actions.map((a) =>
        `<button class="edit-btn action-btn" data-action="${escapeHtml(a.name)}" title="${escapeHtml(a.key)}">${escapeHtml(a.label)}</button>`
      ).join("")
    : "";

  const header = `<div class="detail-file-header">
    <h2>${escapeHtml(f.name)}</h2>
    <span class="status-pill ${f.status}">${f.status}</span>
    <button class="edit-btn" data-edit="test" title="Edit test file (e)">edit test</button>
    <button class="edit-btn" data-edit="source" title="Edit source file (E)">edit source</button>
    ${actionBtns}
  </div>`;

  if (!f.messages || f.messages.length === 0) {
    body.innerHTML = header +
      '<div class="detail-empty">no test messages yet — run this file to see results.</div>';
    wireRightPaneButtons(f);
    return;
  }

  const sorted = sortMessages(f.messages);
  const rows = sorted.map((m, idx) => {
    const name = m.test_name ?? "<anon>";
    const dur = m.duration_ms != null ? formatMs(m.duration_ms) : "";
    return `<li class="test-row" data-idx="${idx}">
      <span class="outcome-dot ${m.outcome}"></span>
      <span class="test-name">${escapeHtml(name)}</span>
      <span class="test-duration">${dur}</span>
    </li>`;
  }).join("");

  const summary = `<div class="detail-summary" style="padding: 6px 12px;">
    sort: <a href="#" class="test-sort-link">${state.testSortMode}${state.testSortReversed ? " ↓" : " ↑"}</a>
  </div>`;

  body.innerHTML = header + summary + `<ul class="test-list">${rows}</ul>`;

  wireRightPaneButtons(f);

  // Test sort link
  const sortLink = body.querySelector(".test-sort-link");
  if (sortLink) sortLink.addEventListener("click", (e) => { e.preventDefault(); toggleTestSortPalette(); });

  // Click test row to enter detail
  body.querySelectorAll(".test-row").forEach((el) => {
    el.addEventListener("click", () => {
      state.testCursor = parseInt(el.dataset.idx, 10);
      enterDetail();
    });
  });
}

function wireRightPaneButtons(f) {
  const body = $("right-body");
  if (!body) return;

  body.querySelectorAll("[data-edit]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.edit === "source") openSourceInEditor();
      else openInEditor(f.id);
    });
  });

  body.querySelectorAll(".action-btn").forEach((btn) => {
    btn.addEventListener("click", () => runPluginAction(btn.dataset.action, f.id));
  });
}

function renderTestDetail() {
  const title = $("right-title");
  const body = $("right-body");
  if (!title || !body) return;

  const f = state.files.get(state.selected);
  if (!f) return;

  const tests = state.testFiltered;
  const m = tests[state.testCursor];
  if (!m) {
    title.innerHTML = `<span>${escapeHtml(f.name)}</span>`;
    body.innerHTML = '<div class="detail-empty">no test selected</div>';
    return;
  }

  const name = m.test_name ?? "<anon>";
  title.innerHTML = `<span>${escapeHtml(f.name)} › ${escapeHtml(name)}</span>`;

  let html = "";

  // Section 1: Metadata
  html += `<div class="detail-section">
    <div class="detail-meta">
      <span class="outcome-tag ${m.outcome}">${m.outcome}</span>
      <span>${escapeHtml(name)}</span>
      ${m.duration_ms != null ? `<span>${formatMs(m.duration_ms)}</span>` : ""}
      ${m.location ? `<span>${escapeHtml(m.location.file)}${m.location.line != null ? `:${m.location.line}` : ""}</span>` : ""}
    </div>
  </div>`;

  // Section 2: Error message (if any)
  if (m.message) {
    const isBad = m.outcome === "fail" || m.outcome === "error";
    html += `<div class="detail-section">
      <div class="detail-section-header">Error</div>
      <div class="detail-error ${isBad ? "" : "warn-msg"}">${escapeHtml(m.message)}</div>
    </div>`;
  }

  // Section 3: Metrics (if any, for data validation)
  if (m.metrics) {
    html += `<div class="detail-section">
      <div class="detail-section-header">Metrics</div>
      <div style="font-size: 12px; color: var(--fg-dim);">${formatMetrics(m.metrics)}</div>
    </div>`;
  }

  // Section 4: Test source
  const wantTestSource = m.location?.line != null;
  html += `<div class="detail-section">
    <div class="detail-section-header">
      Test source
      <button class="edit-btn" data-edit="test" title="Edit test file (e)">edit</button>
    </div>
    <div class="source-snippet" id="detail-test-source">
      ${wantTestSource
        ? '<div class="source-row"><span class="gutter"></span><span class="code">loading…</span></div>'
        : '<div class="source-row"><span class="gutter"></span><span class="code">(no line info)</span></div>'}
    </div>
  </div>`;

  // Section 5: Source function (via dep map)
  html += `<div class="detail-section">
    <div class="detail-section-header">
      Source
      <button class="edit-btn" data-edit="source" title="Edit source file (E)">edit</button>
    </div>
    <div class="source-snippet" id="detail-source-fn">
      <div class="source-row"><span class="gutter"></span><span class="code">loading…</span></div>
    </div>
  </div>`;

  body.innerHTML = html;

  // Wire edit buttons
  body.querySelectorAll("[data-edit]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.edit === "source") openSourceInEditor();
      else openInEditor(state.selected, m.location?.line);
    });
  });

  // Fetch test source
  if (wantTestSource) {
    fetchSource(f.id, m.location.line).then((src) => {
      const el = $("detail-test-source");
      if (el && src) el.innerHTML = renderSourceRows(src);
    });
  }

  // Fetch source function via dep map
  fetchSourceFor(f.id).then((src) => {
    const el = $("detail-source-fn");
    if (!el) return;
    if (src) {
      el.innerHTML = renderSourceRows(src);
      // Update the section header to show the source file name
      const header = el.closest(".detail-section")?.querySelector(".detail-section-header");
      if (header) {
        const editBtn = header.querySelector(".edit-btn");
        header.textContent = `Source — ${src.path}`;
        header.style.textTransform = "none";
        if (editBtn) header.appendChild(editBtn);
      }
    } else {
      el.innerHTML = '<div class="source-row"><span class="gutter"></span><span class="code">(no source mapping)</span></div>';
    }
  });
}

// ── Rendering helpers ───────────────────────────────────────────────────────

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

function renderSourceRows(src) {
  const start = src.start_line ?? 1;
  const hl = src.highlight_line;
  return src.lines
    .map((line, i) => {
      const lno = start + i;
      const cls = lno === hl ? "source-row highlight" : "source-row";
      return `<div class="${cls}"><span class="gutter">${lno}</span><span class="code">${escapeHtml(line)}</span></div>`;
    })
    .join("");
}

function renderCounts() {
  const t = state.totals;
  setCount("pass", t.pass);
  setCount("fail", t.fail);
  setCount("error", t.error);
  setCount("warn", t.warn);
  setCount("skip", t.skip);
  setCount("xfail", t.xfail);
}
const COUNT_ICONS = { pass: "●", fail: "✗", error: "⚠", warn: "⚡", skip: "⊘", xfail: "⊙" };
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
    visBtn.textContent = `▶ run visible (${n})`;
    visBtn.disabled = running || n === 0;
  }
}

// ── Navigation ──────────────────────────────────────────────────────────────

function selectFile(id) {
  state.selected = id;
  // Reset test cursor when selecting a new file
  state.testCursor = 0;
  updateTestFiltered();
  // Update selection highlight in file list
  document.querySelectorAll(".file-row.selected").forEach((el) => el.classList.remove("selected"));
  const li = document.querySelector(`.file-row[data-id="${id}"]`);
  if (li) li.classList.add("selected");
  renderRightPane();
}

function moveFileSelection(delta) {
  if (state.filtered.length === 0) return;
  const idx = state.filtered.indexOf(state.selected);
  let next = idx + delta;
  if (next < 0) next = 0;
  if (next >= state.filtered.length) next = state.filtered.length - 1;
  selectFile(state.filtered[next]);
  const li = document.querySelector(`.file-row[data-id="${state.selected}"]`);
  if (li) li.scrollIntoView({ block: "nearest" });
}

function moveTestSelection(delta) {
  if (state.testFiltered.length === 0) return;
  let next = state.testCursor + delta;
  if (next < 0) next = 0;
  if (next >= state.testFiltered.length) next = state.testFiltered.length - 1;
  state.testCursor = next;
  renderLeftPane();
  renderRightPane();
  const row = document.querySelector(`.test-row[data-idx="${next}"]`);
  if (row) row.scrollIntoView({ block: "nearest" });
}

function moveToNextFailing(delta) {
  if (state.testFiltered.length === 0) return;
  const dir = delta > 0 ? 1 : -1;
  let idx = state.testCursor;
  for (let i = 0; i < state.testFiltered.length; i++) {
    idx = (idx + dir + state.testFiltered.length) % state.testFiltered.length;
    const m = state.testFiltered[idx];
    if (m.outcome === "fail" || m.outcome === "error") {
      state.testCursor = idx;
      renderLeftPane();
      renderRightPane();
      const row = document.querySelector(`.test-row[data-idx="${idx}"]`);
      if (row) row.scrollIntoView({ block: "nearest" });
      return;
    }
  }
}

function enterDetail() {
  if (!state.selected) return;
  const f = state.files.get(state.selected);
  if (!f || !f.messages || f.messages.length === 0) return;
  state.level = "detail";
  updateTestFiltered();
  renderLeftPane();
  renderRightPane();
}

function exitDetail() {
  if (state.level !== "detail") return;
  state.level = "files";
  state.testCursor = 0;
  renderLeftPane();
  renderRightPane();
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

// ── Sorting ─────────────────────────────────────────────────────────────────

function sortMessages(msgs) {
  const sorted = [...msgs];
  const mode = state.testSortMode;
  if (mode === "sequential") return sorted;
  sorted.sort((a, b) => {
    switch (mode) {
      case "sticky":
      case "status":
        return (OUTCOME_RANK[a.outcome] ?? 9) - (OUTCOME_RANK[b.outcome] ?? 9);
      case "name":
        return (a.test_name ?? "").localeCompare(b.test_name ?? "");
      case "time":
        return (b.duration_ms ?? 0) - (a.duration_ms ?? 0);
      default: return 0;
    }
  });
  if (state.testSortReversed) sorted.reverse();
  return sorted;
}

const SORT_OPTIONS = [
  { id: "sequential", label: "sequential", desc: "original order" },
  { id: "status", label: "status", desc: "failures first" },
  { id: "name",   label: "name",   desc: "alphabetical" },
  { id: "plugin", label: "suite", desc: "by suite" },
  { id: "time",   label: "time",   desc: "slowest first" },
];

function toggleSortPalette() {
  const existing = $("sort-palette");
  if (existing) { existing.remove(); return; }
  const pal = document.createElement("div");
  pal.id = "sort-palette";
  pal.className = "overlay-palette";
  const items = SORT_OPTIONS.map((o) => {
    const active = o.id === state.sortMode;
    const arrow = active ? (state.sortReversed ? " ↓" : " ↑") : "";
    return `<div class="pal-row ${active ? "active" : ""}" data-sort="${o.id}">
      <span class="pal-label">${o.label}</span>
      <span class="pal-desc">${o.desc}</span>
      <span class="pal-arrow">${arrow}</span>
    </div>`;
  }).join("");
  pal.innerHTML = `<div class="pal-card">
    <div class="pal-title">Sort</div>
    ${items}
    <div class="pal-footer">click to select · click active to reverse · s or Esc to close</div>
  </div>`;
  document.body.appendChild(pal);
  pal.addEventListener("click", (e) => {
    const row = e.target.closest(".pal-row");
    if (!row) { pal.remove(); return; }
    const picked = row.dataset.sort;
    if (picked === state.sortMode) {
      state.sortReversed = !state.sortReversed;
    } else {
      state.sortMode = picked;
      state.sortReversed = false;
    }
    pal.remove();
    renderFilterList();
    renderLeftPane();
  });
}

function closeSortPalette() {
  const pal = $("sort-palette");
  if (pal) pal.remove();
}

function toggleTestSortPalette() {
  const existing = $("sort-palette");
  if (existing) { existing.remove(); return; }
  const pal = document.createElement("div");
  pal.id = "sort-palette";
  pal.className = "overlay-palette";
  const opts = SORT_OPTIONS.filter((o) => o.id !== "plugin");
  const items = opts.map((o) => {
    const active = o.id === state.testSortMode;
    const arrow = active ? (state.testSortReversed ? " ↓" : " ↑") : "";
    return `<div class="pal-row ${active ? "active" : ""}" data-sort="${o.id}">
      <span class="pal-label">${o.label}</span>
      <span class="pal-desc">${o.desc}</span>
      <span class="pal-arrow">${arrow}</span>
    </div>`;
  }).join("");
  pal.innerHTML = `<div class="pal-card">
    <div class="pal-title">Sort tests</div>
    ${items}
    <div class="pal-footer">click to select · click active to reverse · Esc to close</div>
  </div>`;
  document.body.appendChild(pal);
  pal.addEventListener("click", (e) => {
    const row = e.target.closest(".pal-row");
    if (!row) { pal.remove(); return; }
    const picked = row.dataset.sort;
    if (picked === state.testSortMode) {
      state.testSortReversed = !state.testSortReversed;
    } else {
      state.testSortMode = picked;
      state.testSortReversed = false;
    }
    pal.remove();
    updateTestFiltered();
    renderRightPane();
    if (state.level === "detail") renderLeftPane();
  });
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

// ── Network actions ─────────────────────────────────────────────────────────

async function postJSON(path, body) {
  try {
    const res = await fetch(`${BASE}${path}`, {
      method: "POST",
      headers: body ? { "Content-Type": "application/json" } : {},
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      const txt = await res.text();
      toast(`${path} → ${res.status}: ${txt}`, true);
      return null;
    }
    return await res.json().catch(() => ({}));
  } catch (e) {
    toast(`${path} failed: ${e}`, true);
    return null;
  }
}

async function fetchSource(fileId, line) {
  const key = `${fileId}:${line}`;
  if (state.sourceCache.has(key)) return state.sourceCache.get(key);
  try {
    const res = await fetch(`${BASE}/api/file/${fileId}/source?line=${line}&context=8`);
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
  setStatus("starting run…");
  await postJSON("/api/run");
}
async function runVisible() {
  if (state.filtered.length === 0) { toast("nothing visible to run", true); return; }
  if (state.currentRun?.in_progress) return;
  setStatus(`running ${state.filtered.length} visible file${state.filtered.length === 1 ? "" : "s"}…`);
  await postJSON("/api/rerun", { files: state.filtered });
}
async function cancelRun() {
  if (!state.currentRun?.in_progress) return;
  setStatus("cancelling…");
  await postJSON("/api/cancel");
}
async function rerunFailing() {
  if (!(state.currentRun?.bad_files?.length)) return;
  setStatus("rerunning failing…");
  await postJSON("/api/rerun-failing");
}
async function rerunSelected() {
  if (!state.selected) return;
  setStatus("rerunning selected…");
  await postJSON("/api/rerun", { files: [state.selected] });
}
async function toggleWatch() {
  state.watching = !state.watching;
  renderHeader();
  await postJSON("/api/watch", { enabled: state.watching });
}

function selectedSuiteActions() {
  if (!state.selected) return [];
  const f = state.files.get(state.selected);
  if (!f) return [];
  const suite = (state.pkg?.suites ?? []).find((s) => s.name === f.suite);
  return suite?.actions ?? [];
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
    if (res !== null) toast("opened in $EDITOR");
  }
}

async function openSourceInEditor() {
  if (!state.selected) return;
  // Fetch source-for to get the path, then open it
  const src = await fetchSourceFor(state.selected);
  if (!src || !src.path) {
    toast("no source mapping found", true);
    return;
  }
  // The source-for path is relative; open via the open-editor endpoint
  // by constructing the absolute path.
  if (IS_VSCODE) {
    const root = state.pkg?.root ?? "";
    vscode.postMessage({ command: "openFile", path: `${root}/${src.path}` });
  } else {
    // Use the file_id endpoint but pass the source path instead.
    // We need to use the open-editor endpoint with a path rather than file_id.
    const root = state.pkg?.root ?? "";
    const body = { path: `${root}/${src.path}` };
    const res = await postJSON("/api/open-editor", body);
    if (res !== null) toast("opened in $EDITOR");
  }
}

// ── Theme ───────────────────────────────────────────────────────────────────

function toggleTheme() {
  const cur = document.documentElement.getAttribute("data-theme") || "dark";
  const next = cur === "dark" ? "light" : "dark";
  document.documentElement.setAttribute("data-theme", next);
  try { localStorage.setItem("scrutin-theme", next); } catch (_) {}
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
    } catch (_) {}
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
  } catch (_) {}
}

function wireSidebarResize() {
  const resizer = $("pane-resizer");
  if (!resizer) return;
  const MIN_W = 240;
  const MAX_W = 900;
  let dragging = false;

  const onMove = (e) => {
    if (!dragging) return;
    const layout = $("layout");
    const rect = layout.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const clamped = Math.max(MIN_W, Math.min(MAX_W, x));
    document.documentElement.style.setProperty("--sidebar-w", `${clamped}px`);
  };
  const onUp = () => {
    if (!dragging) return;
    dragging = false;
    resizer.classList.remove("active");
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    const w = getComputedStyle(document.documentElement)
      .getPropertyValue("--sidebar-w").trim();
    const n = parseInt(w, 10);
    if (Number.isFinite(n)) {
      try { localStorage.setItem("scrutin-sidebar-w", String(n)); } catch (_) {}
    }
  };
  resizer.addEventListener("pointerdown", (e) => {
    dragging = true;
    resizer.classList.add("active");
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";
    e.preventDefault();
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  });
}

function formatMs(ms) {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
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

// ── Events ──────────────────────────────────────────────────────────────────

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
  on("btn-cancel", "click", cancelRun);
  on("btn-rerun-failing", "click", rerunFailing);
  on("btn-back", "click", exitDetail);
  on("toggle-watch", "change", toggleWatch);
  on("btn-theme", "click", toggleTheme);
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

  // Hide keyboard hint in editor mode
  if (IS_EDITOR) {
    const hint = $("keyboard-hint");
    if (hint) hint.style.display = "none";
  }

  window.addEventListener("keydown", (e) => {
    if (e.target.tagName === "INPUT" || e.target.tagName === "TEXTAREA" || e.target.tagName === "SELECT") {
      if (e.key === "Escape") e.target.blur();
      return;
    }

    // Close overlays first
    if (e.key === "Escape") {
      const help = $("help");
      const sortPal = $("sort-palette");
      if (help && !help.classList.contains("hidden")) {
        toggleHelp(false);
        return;
      }
      if (sortPal) {
        closeSortPalette();
        return;
      }
      if (state.level === "detail") {
        exitDetail();
        return;
      }
      return;
    }

    // Navigation keys — work in all modes (standalone + editor)
    if (state.level === "files") {
      switch (e.key) {
        case "j": case "ArrowDown": moveFileSelection(+1); return;
        case "k": case "ArrowUp": moveFileSelection(-1); return;
        case "Enter": enterDetail(); e.preventDefault(); return;
      }
    } else {
      switch (e.key) {
        case "j": case "ArrowDown": moveTestSelection(+1); return;
        case "k": case "ArrowUp": moveTestSelection(-1); return;
      }
    }
    if (e.key === "/") { e.preventDefault(); $("filter-input")?.focus(); return; }

    // Action shortcuts — disabled in editor webviews
    if (IS_EDITOR) return;

    switch (e.key) {
      case "a": runAll(); break;
      case "v": runVisible(); break;
      case "r": rerunSelected(); break;
      case "R": rerunFailing(); break;
      case "c": cancelRun(); break;
      case "w": toggleWatch(); break;
      case "p": cyclePlugin(+1); break;
      case "P": cyclePlugin(-1); break;
      case "t": cycleStatus(+1); break;
      case "T": cycleStatus(-1); break;
      case "s": toggleSortPalette(); break;
      case "e": openInEditor(); break;
      case "E": openSourceInEditor(); break;
      case "n": if (state.level === "detail") moveToNextFailing(+1); break;
      case "N": if (state.level === "detail") moveToNextFailing(-1); break;
      case "?": toggleHelp(); break;
      default: {
        const pa = selectedSuiteActions().find((a) => a.key === e.key);
        if (pa) runPluginAction(pa.name);
        break;
      }
    }
  });

  fetchSnapshot().then(connectEvents);
});

function toggleHelp(force) {
  const el = $("help");
  if (!el) return;
  if (typeof force === "boolean") {
    el.classList.toggle("hidden", !force);
  } else {
    el.classList.toggle("hidden");
  }
  const hint = $("help-plugin-hint");
  if (hint) {
    const all = (state.pkg?.suites ?? []).flatMap((s) =>
      (s.actions ?? []).map((a) => `${a.key}: ${a.label} (${s.name})`)
    );
    hint.textContent = all.length > 0 ? `Plugin: ${all.join(" · ")}` : "";
  }
}

// ── Utilities ───────────────────────────────────────────────────────────────

let toastTimer = null;
function toast(msg, isError) {
  const el = $("toast");
  el.textContent = msg;
  el.classList.remove("hidden");
  el.classList.toggle("error", !!isError);
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), 4000);
}

function escapeHtml(s) {
  if (s == null) return "";
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("\"", "&quot;")
    .replaceAll("'", "&#039;");
}
const esc = escapeHtml;
