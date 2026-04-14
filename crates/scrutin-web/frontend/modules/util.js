// Small, dependency-free helpers used throughout the frontend.

export const $ = (id) => document.getElementById(id);

export function escapeHtml(s) {
  if (s == null) return "";
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("\"", "&quot;")
    .replaceAll("'", "&#039;");
}

/// Single predicate for "this event is a hard failure." Used in ~8 places
/// so it lives in one spot to avoid typo drift.
export const isBadOutcome = (m) =>
  m?.outcome === "fail" || m?.outcome === "error";

/// Promote "passed" to "warned" for files whose warn count > 0, so the
/// sidebar status pill tells the user something useful.
export function displayStatus(f) {
  if (f.status === "passed" && (f.counts?.warn ?? 0) > 0) return "warned";
  return f.status;
}

export function formatMs(ms) {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function formatMetrics(m) {
  if (m.total != null && m.failed != null) {
    const frac = m.fraction != null ? (m.fraction * 100).toFixed(2) : "0.00";
    return `${m.failed} of ${m.total} failed (${frac}%)`;
  }
  if (m.total != null) return `${m.total} checked`;
  if (m.failed != null) return `${m.failed} failed`;
  return "";
}

let toastTimer = null;
export function toast(msg, isError) {
  const el = $("toast");
  if (!el) return;
  el.textContent = msg;
  el.classList.remove("hidden");
  el.classList.toggle("error", !!isError);
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), 4000);
}
