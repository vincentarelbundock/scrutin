// Theme toggle, persisted sidebar width, drag-to-resize.

import { $ } from "./util.js";

export function toggleTheme() {
  const cur = document.documentElement.getAttribute("data-theme") || "dark";
  const next = cur === "dark" ? "light" : "dark";
  document.documentElement.setAttribute("data-theme", next);
  try { localStorage.setItem("scrutin-theme", next); } catch (_) {}
}

export function applyStoredTheme() {
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

export function applyStoredSidebarWidth() {
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

export function wireSidebarResize() {
  const resizer = $("pane-resizer");
  if (!resizer) return;
  const MIN_W = 240;
  const MAX_W = 900;
  let dragging = false;

  const isHorizontal = () => $("layout")?.classList.contains("horizontal");

  const onMove = (e) => {
    if (!dragging) return;
    const layout = $("layout");
    const rect = layout.getBoundingClientRect();
    if (isHorizontal()) {
      const y = e.clientY - rect.top;
      const clamped = Math.max(100, Math.min(y, rect.height - 100));
      document.documentElement.style.setProperty("--topbar-h", `${clamped}px`);
    } else {
      const x = e.clientX - rect.left;
      const clamped = Math.max(MIN_W, Math.min(MAX_W, x));
      document.documentElement.style.setProperty("--sidebar-w", `${clamped}px`);
    }
  };
  const onUp = () => {
    if (!dragging) return;
    dragging = false;
    resizer.classList.remove("active");
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
    if (!isHorizontal()) {
      const w = getComputedStyle(document.documentElement).getPropertyValue("--sidebar-w").trim();
      const n = parseInt(w, 10);
      if (Number.isFinite(n)) {
        try { localStorage.setItem("scrutin-sidebar-w", String(n)); } catch (_) {}
      }
    }
  };
  resizer.addEventListener("pointerdown", (e) => {
    dragging = true;
    resizer.classList.add("active");
    document.body.style.userSelect = "none";
    document.body.style.cursor = isHorizontal() ? "row-resize" : "col-resize";
    e.preventDefault();
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  });
}

export function resizeSidebar(delta) {
  const layout = $("layout");
  const horiz = layout && layout.classList.contains("horizontal");
  const root = document.documentElement;
  if (horiz) {
    const h = getComputedStyle(root).getPropertyValue("--topbar-h").trim();
    const cur = parseInt(h, 10) || Math.round(window.innerHeight * 0.5);
    const clamped = Math.max(100, Math.min(cur + delta, window.innerHeight - 100));
    root.style.setProperty("--topbar-h", `${clamped}px`);
  } else {
    const w = getComputedStyle(root).getPropertyValue("--sidebar-w").trim();
    const cur = parseInt(w, 10) || 420;
    const clamped = Math.max(200, Math.min(cur + delta, window.innerWidth - 200));
    root.style.setProperty("--sidebar-w", `${clamped}px`);
    try { localStorage.setItem("scrutin-sidebar-w", String(clamped)); } catch (_) {}
  }
}
