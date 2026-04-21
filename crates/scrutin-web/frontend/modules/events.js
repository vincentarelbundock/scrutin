// Server-Sent Events: subscribe to `/api/events` and apply each event
// to the state via a reducer. Every mutation ends by calling whichever
// render function(s) the event affects.

import { state, BASE } from "./state.js";
import { toast } from "./util.js";
import { updateTestFiltered } from "./sort.js";
import {
  renderAll, renderFilterList, renderLeftPane, renderRightPane,
  renderCounts, renderControls, renderHeader, setStatus,
} from "./render.js";

let es = null;
let reconnectDelay = 500;

export function connectEvents() {
  if (es) { try { es.close(); } catch (_) {} }
  es = new EventSource(`${BASE}/api/events`);
  es.onopen = () => { reconnectDelay = 500; };
  es.onerror = () => {
    if (es) { try { es.close(); } catch (_) {} es = null; }
    toast(`disconnected \u2014 reconnecting in ${Math.round(reconnectDelay)}ms`, true);
    setTimeout(connectEvents, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 10000);
  };

  const kinds = [
    "run_started", "file_started", "file_finished",
    "run_complete", "run_cancelled", "watcher_triggered",
    "notice", "log", "heartbeat",
  ];
  for (const k of kinds) {
    es.addEventListener(k, (ev) => {
      try { apply(k, JSON.parse(ev.data)); }
      catch (e) { console.error("event parse", k, e); }
    });
  }
}

/// Reducer. Mutates `state` in-place, then calls the minimal set of
/// render functions needed.
function apply(kind, data) {
  switch (kind) {
    case "run_started":
      state.currentRun = {
        run_id: data.run_id,
        started_at: data.started_at,
        finished_at: null,
        in_progress: true,
        totals: { pass: 0, fail: 0, error: 0, skip: 0, xfail: 0, warn: 0 },
        bad_files: [],
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
      setStatus(`done \u00b7 ${data.totals.pass} pass \u00b7 ${data.totals.fail + data.totals.error} bad`);
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

    case "notice":
      toast(data.message);
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
