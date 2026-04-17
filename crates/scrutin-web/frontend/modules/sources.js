// Source-snippet rendering + shared wiring helpers. Extracted from the
// three render functions that previously each duplicated these blocks.

import { $ } from "./util.js";
import { fetchSource, fetchSourceFor, openInEditor, openSourceInEditor } from "./api.js";

/// Render a `WireSource` `{ lines, start_line, highlight_line }` into the
/// numbered-gutter format. Each `lines[i]` is a plain HTML-escaped line
/// from the server (no syntax highlighting), safe to inject via innerHTML.
export function renderSourceRows(src) {
  const start = src.start_line ?? 1;
  const hl = src.highlight_line;
  return src.lines
    .map((line, i) => {
      const lno = start + i;
      const cls = lno === hl ? "source-row highlight" : "source-row";
      return `<div class="${cls}"><span class="gutter">${lno}</span><span class="code">${line}</span></div>`;
    })
    .join("");
}

const LOADING_ROW = '<div class="source-row"><span class="gutter"></span><span class="code">loading\u2026</span></div>';
const UNAVAILABLE_ROW = '<div class="source-row"><span class="gutter"></span><span class="code">(source unavailable)</span></div>';
const NO_MAPPING_ROW = '<div class="source-row"><span class="gutter"></span><span class="code">(no source mapping)</span></div>';

export const sourcePlaceholder = () => LOADING_ROW;

/// Fetch the test source around `line` and paint it into `elementId`.
/// `line` may be null — the server then returns the top of the file, so
/// file-level errors still get a useful view.
export function renderTestSourceInto(elementId, fileId, line) {
  fetchSource(fileId, line).then((src) => {
    const el = $(elementId);
    if (!el) return;
    el.innerHTML = src ? renderSourceRows(src) : UNAVAILABLE_ROW;
  });
}

/// Fetch dep-mapped production source for a test file and paint it.
/// `onPath(path)` is invoked if the fetch succeeds so callers can update
/// a surrounding header label (e.g. "Source \u2014 R/math.R").
export function renderFnSourceInto(elementId, fileId, onPath) {
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

/// Wire every `[data-edit]` button inside `container`. The button's
/// `data-edit` attr picks which file to open:
///   data-edit="test"   \u2192 openInEditor(fileId, line)
///   data-edit="source" \u2192 openSourceInEditor()
/// Previously duplicated in three render functions with subtly different
/// argument plumbing.
export function wireEditButtons(container, fileId, line) {
  container.querySelectorAll("[data-edit]").forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.edit === "source") openSourceInEditor();
      else openInEditor(fileId, line);
    });
  });
}
