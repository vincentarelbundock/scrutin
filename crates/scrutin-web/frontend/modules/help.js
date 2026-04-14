// Help overlay (? key). Populated from the shared keymap.

import { state } from "./state.js";
import { $, escapeHtml } from "./util.js";

export function toggleHelp(force) {
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
  const sectionHeader = (title) => `<dt class="section-header">${escapeHtml(title)}</dt>`;
  const isShared = (b) => b.levels.includes("files") && b.levels.includes("detail");

  // Shared bindings (files + detail).
  let html = "";
  const seen = new Set();
  for (const b of state.keymap) {
    if (!b.help || seen.has(b.action) || !isShared(b)) continue;
    seen.add(b.action);
    html += helpRow(b);
  }

  // Per-level sections for exclusive bindings.
  const sections = [
    ["Files only",             "files"],
    ["Detail / Failure only",  "detail"],
  ];
  for (const [title, level] of sections) {
    const sectionSeen = new Set();
    let rows = "";
    for (const b of state.keymap) {
      if (!b.help || sectionSeen.has(b.action)) continue;
      if (!b.levels.includes(level) || isShared(b)) continue;
      sectionSeen.add(b.action);
      rows += helpRow(b);
    }
    if (rows) html += sectionHeader(title) + rows;
  }

  const hasSuiteActions = (state.pkg?.suites ?? []).some((s) => (s.actions ?? []).length > 0);
  if (hasSuiteActions) {
    html += sectionHeader("Actions (a to open menu)");
    for (const s of (state.pkg?.suites ?? [])) {
      for (const a of (s.actions ?? [])) {
        html += `<dt>${escapeHtml(a.label)}</dt><dd></dd>`;
      }
    }
  }
  dl.innerHTML = html;
}
