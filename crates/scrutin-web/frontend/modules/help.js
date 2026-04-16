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

  // Flat listing in keymap array order. Grouping is encoded by the order
  // of entries in scrutin-core::keymap::DEFAULT_KEYMAP, so "related"
  // bindings (filters, editor shortcuts, etc.) appear adjacent in the
  // help overlay as long as they're adjacent in the keymap source.
  //
  // Plugin actions are intentionally *not* listed here: they render as
  // numbered chips in the Detail view (digits 1-9), which is
  // self-documenting.
  let html = "";
  const seen = new Set();
  for (const b of state.keymap) {
    if (!b.help || seen.has(b.action)) continue;
    seen.add(b.action);
    html += helpRow(b);
  }

  dl.innerHTML = html;
}
