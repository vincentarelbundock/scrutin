// Global state and environment constants. No logic lives here — just the
// shared mutable object every other module imports.

export const IS_VSCODE = typeof acquireVsCodeApi === "function";
export const vscode = IS_VSCODE ? acquireVsCodeApi() : null;
export const BASE = IS_VSCODE ? (window.__SCRUTIN_BASE_URL__ || "http://127.0.0.1:7878") : "";
export const IS_EDITOR = IS_VSCODE; // extend for RStudio, other editors as needed

export const state = {
  pkg: null,
  files: new Map(),       // FileId -> WireFile
  fileOrder: [],          // stable display order
  filtered: [],           // visible slice after all filters
  selected: null,         // FileId (highlighted file)
  multiSelected: new Set(),
  lastClicked: null,      // anchor for shift-click range selection
  currentRun: null,       // WireRunSummary
  watching: false,
  nWorkers: 1,
  busy: 0,
  filterText: "",
  pluginFilter: "",
  statusFilter: "",
  groupFilter: "",
  groups: [],             // [{ name, include, exclude, tools }]
  sortMode: "status",
  sortReversed: false,
  testSortMode: "status",
  testSortReversed: false,
  totals: { pass: 0, fail: 0, error: 0, skip: 0, xfail: 0, warn: 0 },
  sourceCache: new Map(),
  keymap: [],
  level: "files",         // "files" | "detail" | "failure"
  testCursor: 0,
  testFiltered: [],       // sorted messages for the selected file
  failureCursor: 0,
  failures: [],           // global {fileId, file, test, message, line, outcome}
};

export const STATUS_CYCLE = [
  "", "failed", "errored", "warned", "passed", "skipped",
  "running", "pending", "cancelled",
];

// Outcome rank: populated from `/api/snapshot` at boot so the server's
// `Outcome::rank()` stays authoritative. This is a mutable object (not a
// `let` binding) so callers see live updates through the same reference.
export const OUTCOME_RANK = { fail: 0, error: 1, warn: 2, pass: 3, skip: 4, xfail: 5 };
export function setOutcomeRanks(ranks) {
  for (const k of Object.keys(OUTCOME_RANK)) delete OUTCOME_RANK[k];
  Object.assign(OUTCOME_RANK, ranks);
}
