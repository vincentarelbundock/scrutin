// Sort modes + file/test sort comparators.

import { state, OUTCOME_RANK } from "./state.js";

export const SORT_OPTIONS = [
  { id: "sequential", label: "sequential", desc: "original order" },
  { id: "status",     label: "status",     desc: "failures first" },
  { id: "name",       label: "name",       desc: "alphabetical" },
  { id: "suite",      label: "suite",      desc: "by suite" },
  { id: "time",       label: "time",       desc: "slowest first" },
];

/// File-status rank for the "status" sort of the file list. Richer than
/// `OUTCOME_RANK` because files have statuses (errored/failed/running/...)
/// that don't correspond 1:1 to outcomes.
export function fileStatusRank(f) {
  if (!f) return 99;
  switch (f.status) {
    case "errored":   return 0;
    case "failed":    return 1;
    case "passed":    return (f.counts?.warn ?? 0) > 0 ? 2 : 6;
    case "running":   return 3;
    case "cancelled": return 4;
    case "pending":   return 5;
    case "skipped":   return 7;
    default:          return 8;
  }
}

/// Sort a message list according to `state.testSortMode`.
export function sortMessages(msgs) {
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
      default: return 0;
    }
  });
  if (state.testSortReversed) sorted.reverse();
  return sorted;
}

/// Rebuild `state.testFiltered` from the currently selected file, clamping
/// the test cursor if the new list is shorter.
export function updateTestFiltered() {
  const f = state.files.get(state.selected);
  if (!f || !f.messages) { state.testFiltered = []; return; }
  state.testFiltered = sortMessages(f.messages);
  if (state.testCursor >= state.testFiltered.length) {
    state.testCursor = Math.max(0, state.testFiltered.length - 1);
  }
}
