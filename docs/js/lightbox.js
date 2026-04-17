// Wire medium-zoom to every screenshot on the docs site. `img.screenshot`
// is the class we apply in markdown via `{ .screenshot }`, so any image
// tagged that way gets click-to-zoom with a dark overlay.
document.addEventListener("DOMContentLoaded", () => {
  if (typeof mediumZoom !== "function") return;
  mediumZoom("img.screenshot, .md-typeset .screenshot", {
    background: "rgba(15, 17, 21, 0.9)",
    margin: 96,
  });

  document.querySelectorAll(".hero-video-poster").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      const src = btn.getAttribute("data-video");
      if (!src) return;
      const overlay = document.createElement("div");
      overlay.className = "video-lightbox-overlay";
      const vid = document.createElement("video");
      vid.src = src;
      vid.controls = true;
      vid.autoplay = true;
      vid.playsInline = true;
      vid.className = "video-zoomed";
      overlay.appendChild(vid);
      document.body.appendChild(overlay);
      const close = () => overlay.remove();
      overlay.addEventListener("click", (ev) => {
        if (ev.target === overlay) close();
      });
      document.addEventListener("keydown", function esc(ev) {
        if (ev.key === "Escape") {
          close();
          document.removeEventListener("keydown", esc);
        }
      });
    });
  });
});
