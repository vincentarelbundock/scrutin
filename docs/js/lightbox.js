// Wire medium-zoom to every screenshot on the docs site. `img.screenshot`
// is the class we apply in markdown via `{ .screenshot }`, so any image
// tagged that way gets click-to-zoom with a dark overlay.
document.addEventListener("DOMContentLoaded", () => {
  if (typeof mediumZoom !== "function") return;
  mediumZoom("img.screenshot, .md-typeset .screenshot", {
    background: "rgba(15, 17, 21, 0.9)",
    margin: 96,
  });
});
