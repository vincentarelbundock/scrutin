// Keyboard dispatch. The ACTION_HANDLERS map is the only place that
// names individual actions; everything else routes through the shared
// keymap the server ships in `/api/snapshot`.

import { state, IS_EDITOR } from "./state.js";
import { $, toast } from "./util.js";
import { currentLevel } from "./levels.js";
import {
  cancelRun, rerunSelected, openInEditor, openSourceInEditor,
  toggleWatch, applyCorrection, runPluginAction,
} from "./api.js";
import {
  toggleSortPalette, toggleRunPalette,
  closeSortPalette, closeRunPalette,
} from "./palettes.js";
import { toggleHelp } from "./help.js";
import {
  toggleMultiSelect, clearMultiSelect,
} from "./navigation.js";
import {
  cyclePlugin, cycleStatus,
} from "./render.js";
import { resizeSidebar } from "./theme.js";

export function browserKeyToKeymapKey(e) {
  if (e.ctrlKey && e.key.length === 1) return `Ctrl-${e.key}`;
  switch (e.key) {
    case "ArrowUp":    return "Up";
    case "ArrowDown":  return "Down";
    case "ArrowLeft":  return "Left";
    case "ArrowRight": return "Right";
    case " ":          return "Space";
    case "Escape":     return "Esc";
    default:           return e.key;
  }
}

export function resolveAction(keyStr, level) {
  // Overlay mode swallows most keys and only accepts close triggers.
  const help = $("help");
  const sortPal = $("sort-palette");
  const runPal = $("run-palette");
  if ((help && !help.classList.contains("hidden")) || sortPal || runPal) {
    if (["Esc", "q", "?", "r", "s"].includes(keyStr)) return "pop";
    return null;
  }
  for (const b of state.keymap) {
    if (b.key === keyStr && b.levels.includes(level)) return b.action;
  }
  return null;
}

export const ACTION_HANDLERS = {
  // Navigation \u2192 delegated to the current level handler.
  cursor_down:   () => currentLevel().cursor(+1),
  cursor_up:     () => currentLevel().cursor(-1),
  cursor_top:    () => currentLevel().cursor(-Infinity),
  cursor_bottom: () => currentLevel().cursor(+Infinity),

  enter: (e) => { currentLevel().onEnter(); e?.preventDefault(); },

  pop: () => {
    // Overlays take priority. Then multi-select clear. Then level pop.
    const help = $("help");
    if (help && !help.classList.contains("hidden")) { toggleHelp(false); return; }
    if ($("run-palette"))    { closeRunPalette();    return; }
    if ($("sort-palette"))   { closeSortPalette();   return; }
    if (state.multiSelected.size > 0) { clearMultiSelect(); return; }
    currentLevel().onPop();
  },

  quit: () => {},

  // Run control.
  open_run_menu:    () => toggleRunPalette(),
  run_current_file: () => rerunSelected(),
  cancel_file:      () => cancelRun(),
  cancel_all:       () => cancelRun(),

  // Filtering.
  open_filter:              (e) => { e?.preventDefault(); $("filter-input")?.focus(); },
  open_sort_menu:           () => toggleSortPalette(),
  cycle_status_filter:      () => cycleStatus(+1),
  cycle_status_filter_back: () => cycleStatus(-1),
  cycle_tool_filter:        () => cyclePlugin(+1),
  cycle_tool_filter_back:   () => cyclePlugin(-1),

  // Actions \u2192 also delegated to the current level handler.
  edit_test:    () => openInEditor(),
  edit_source:  () => openSourceInEditor(),
  yank_message: () => {
    const msg = currentLevel().yankMessage();
    if (!msg) return;
    navigator.clipboard.writeText(msg).then(
      () => toast("copied to clipboard"),
      () => toast("clipboard access denied", true),
    );
  },

  enter_log:     () => {},
  enter_help:    () => toggleHelp(),
  toggle_select: () => { if (state.selected) toggleMultiSelect(state.selected); },
  toggle_visual: () => {},
  shrink_list:   () => resizeSidebar(-40),
  grow_list:     () => resizeSidebar(+40),
  toggle_orientation: () => {
    const layout = $("layout");
    if (layout) layout.classList.toggle("horizontal");
  },
  // Source-scroll actions are TUI-only; no-op in the web.
  source_scroll_down: () => {},
  source_scroll_up:   () => {},
};

export function dispatchAction(action, e) {
  const handler = ACTION_HANDLERS[action];
  if (handler) handler(e);
}

/// Keys that IDE webview hosts reliably intercept before the page sees
/// them (VS Code's find box, sidebar toggle). Dropping them in editor
/// contexts avoids binding an action that will only fire half the time.
const BLOCKED_EDITOR_KEYS = new Set(["Ctrl-f", "Ctrl-b"]);

/// Install the global keydown listener. Delegates to ACTION_HANDLERS
/// via resolveAction + dispatchAction.
export function wireKeyboard() {
  window.addEventListener("keydown", (e) => {
    // Typing into a form control: let the control handle everything,
    // but swallow Esc so blur returns focus to the app.
    const tag = e.target.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") {
      if (e.key === "Escape") e.target.blur();
      return;
    }

    const keyStr = browserKeyToKeymapKey(e);
    if (!keyStr) return;
    if (IS_EDITOR && BLOCKED_EDITOR_KEYS.has(keyStr)) return;

    const level = state.level;

    // Detail level + 0-9: accept the Nth spell-check suggestion (1-9)
    // or whitelist the word (0) when the cursor event has corrections.
    // Falls through to the Nth plugin action (ruff/jarl fix variants)
    // when no corrections are attached. Mirrors the TUI binding.
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
        const action = suite?.actions?.[n - 1];
        if (action) {
          e.preventDefault();
          runPluginAction(action.name, state.selected);
          return;
        }
      }
    }

    const action = resolveAction(keyStr, level);
    if (action) { dispatchAction(action, e); return; }
  });
}
