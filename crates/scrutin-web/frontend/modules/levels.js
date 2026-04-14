// The three drill levels as first-class handler objects. Every
// level-dependent operation in the frontend routes through here instead
// of being a free-standing if/switch on `state.level` string.
//
// Each level implements the same shape so the dispatchers (renderLeftPane,
// cursor_down, enter, pop, yank_message, breadcrumb builder) are thin.

import { state } from "./state.js";
import { isBadOutcome } from "./util.js";
import {
  enterDetail, exitDetail, enterFailure, exitFailure,
  moveFileSelection, moveTestSelection, moveFailureSelection,
} from "./navigation.js";

/// Shape of a level handler:
///   id           \u2014 string key for DOM class naming
///   renderLeft() \u2014 paints the left pane (may be a no-op when hidden)
///   renderRight()\u2014 paints the right pane
///   cursor(n)    \u2014 moves the level's cursor by n (signed; \xb1Infinity \u2192 top/bottom)
///   onEnter()    \u2014 what "drill in" does; no-op at the deepest level
///   onPop()      \u2014 what "Esc / back" does
///   segments()   \u2014 breadcrumb path, array of { label, level|null }
///   counter()    \u2014 optional trailing counter (e.g. "(3/9)") or ""
///   yankMessage()\u2014 string to copy for the y binding
///   hidesLeftPane\u2014 CSS hook: collapse the left pane when true

function pkgName()       { return state.pkg?.name ?? "\u2014"; }
function selectedFile()  { return state.selected ? state.files.get(state.selected) : null; }

export const LEVELS = {
  files: {
    id: "files",
    hidesLeftPane: false,
    renderLeft:  (ctx) => ctx.renderFileList(),
    renderRight: (ctx) => ctx.renderTestListRight(),
    cursor:      (n) => moveFileSelection(n),
    onEnter:     () => enterDetail(),
    onPop:       () => {},
    segments:    () => [{ label: pkgName(), level: "files" }],
    counter:     () => "",
    yankMessage: () => {
      const f = selectedFile();
      if (!f || !f.messages) return "";
      const bad = f.messages.find(isBadOutcome);
      return bad ? (bad.message ?? "") : (f.path ?? "");
    },
  },

  detail: {
    id: "detail",
    hidesLeftPane: false,
    renderLeft:  (ctx) => ctx.renderTestListLeft(),
    renderRight: (ctx) => ctx.renderTestDetail(),
    cursor:      (n) => moveTestSelection(n),
    onEnter:     () => {
      // Drill deeper only if the current test is a real failure.
      const m = state.testFiltered[state.testCursor];
      if (m && isBadOutcome(m)) enterFailure();
    },
    onPop:       () => exitDetail(),
    segments:    () => {
      const f = selectedFile();
      return [
        { label: pkgName(),           level: "files"  },
        { label: f?.name ?? "\u2014", level: "detail" },
      ];
    },
    counter:     () => "",
    yankMessage: () => state.testFiltered[state.testCursor]?.message ?? "",
  },

  failure: {
    id: "failure",
    hidesLeftPane: true,
    renderLeft:  () => { /* sidebar hidden in CSS; nothing to paint */ },
    renderRight: (ctx) => ctx.renderFailureDetail(),
    cursor:      (n) => moveFailureSelection(n),
    onEnter:     () => {},                    // leaf
    onPop:       () => exitFailure(),
    segments:    () => {
      const f = selectedFile();
      const segs = [
        { label: pkgName(),           level: "files"  },
        { label: f?.name ?? "\u2014", level: "detail" },
      ];
      const ff = state.failures[state.failureCursor];
      if (ff) segs.push({ label: ff.test, level: "failure" });
      return segs;
    },
    counter:     () => {
      const n = state.failures.length;
      if (n === 0) return "";
      return `(${state.failureCursor + 1}/${n})`;
    },
    yankMessage: () => state.failures[state.failureCursor]?.message ?? "",
  },
};

/// Current level handler. Always use this; never read `state.level` as a
/// string outside of this module.
export const currentLevel = () => LEVELS[state.level];
