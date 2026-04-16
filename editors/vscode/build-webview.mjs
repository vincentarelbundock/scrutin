// Build the VS Code webview bundle from the scrutin-web frontend.
//
// Two outputs:
//   - webview/app.js:    esbuild bundle of frontend/app.js + modules/*.js
//   - webview/index.html: transformed frontend/index.html with template
//                         markers the extension substitutes at panel creation
//
// The transform strategy is "strip, then inject": we delete all <link
// rel="stylesheet"> and <script src=...> tags (regardless of attribute
// order or new additions), then inject our own. That way, frontend HTML
// changes (new imports, reordered attrs, type="module") can't silently
// break the CSP without us noticing.
//
// Sanity checks at the end assert every required marker made it into the
// output. If the frontend HTML restructures enough that an injection
// site disappears, we fail loudly here rather than shipping a broken
// VSIX.

import { readFileSync, writeFileSync, mkdirSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";
import * as esbuild from "esbuild";

const __dirname = dirname(fileURLToPath(import.meta.url));
const FRONTEND = join(__dirname, "..", "..", "crates", "scrutin-web", "frontend");
const WEBVIEW = join(__dirname, "webview");

mkdirSync(WEBVIEW, { recursive: true });

// ── Bundle JS ───────────────────────────────────────────────────────────────

await esbuild.build({
  entryPoints: [join(FRONTEND, "app.js")],
  bundle: true,
  format: "esm",
  target: "es2020",
  outfile: join(WEBVIEW, "app.js"),
  logLevel: "info",
});

// ── Build style.css ─────────────────────────────────────────────────────────
// VS Code theme overrides prepend; strip only the palette :root block(s)
// from the frontend's style.css so our overrides win. We strip the *block*
// by regex, not by line count, so layout rules like `body { height: 100vh }`
// that live right after the palette are preserved.

const overrides = readFileSync(join(__dirname, "webview-overrides.css"), "utf-8");
let frontendStyle = readFileSync(join(FRONTEND, "style.css"), "utf-8");

// Drop every top-level `:root { ... }` or `:root[data-theme="..."] { ... }`
// block. These carry scrutin's palette; the VS Code variables live in our
// overrides file instead.
frontendStyle = frontendStyle.replace(
  /:root(?:\[[^\]]*\])?(?:\s*,\s*:root(?:\[[^\]]*\])?)*\s*\{[^}]*\}\s*/g,
  "",
);

writeFileSync(join(WEBVIEW, "style.css"), overrides + frontendStyle);

// Sanity: the frontend MUST still have a body rule after we strip :root,
// or the webview will have no layout container.
if (!/\bbody\s*\{[^}]*height\s*:/.test(frontendStyle)) {
  console.error(
    "build-webview: frontend style.css no longer contains a `body { height: ... }` rule; webview layout will break.",
  );
  process.exit(1);
}

// ── Transform index.html ────────────────────────────────────────────────────

let html = readFileSync(join(FRONTEND, "index.html"), "utf-8");

html = html
  // The theme toggle is redundant in VS Code; VS Code owns theming.
  .replace(/\s*<button id="btn-theme"[^>]*>[\s\S]*?<\/button>/g, "")
  // Strip all stylesheet links; the extension injects a single one.
  .replace(/\s*<link rel="stylesheet"[^>]*>/g, "")
  // Strip all external script references; the extension injects one app.js.
  .replace(/\s*<script[^>]*src=[^>]*><\/script>/g, "")
  // After <title>, inject CSP meta tag and our stylesheet.
  .replace(
    /<title>[^<]*<\/title>/,
    `<meta http-equiv="Content-Security-Policy" content="{{csp}}" />\n  <title>Scrutin</title>\n  <link rel="stylesheet" href="{{styleUri}}" />`,
  )
  // Before </body>, inject the base URL global and the bundled app.js.
  .replace(
    "</body>",
    `  <script nonce="{{nonce}}">window.__SCRUTIN_BASE_URL__ = "{{baseUrl}}";</script>\n  <script nonce="{{nonce}}" type="module" src="{{scriptUri}}"></script>\n</body>`,
  );

// ── Sanity checks ───────────────────────────────────────────────────────────
// If the frontend HTML ever drifts in a way that breaks our transforms,
// we'd rather fail now than ship a webview with missing CSP or broken
// script references.

const requiredMarkers = [
  "{{csp}}",
  "{{styleUri}}",
  "{{nonce}}",
  "{{baseUrl}}",
  "{{scriptUri}}",
];
for (const marker of requiredMarkers) {
  if (!html.includes(marker)) {
    console.error(
      `build-webview: required marker ${marker} missing from output.`,
    );
    console.error(
      `  The frontend HTML may have changed shape. Check that <title> and </body>`,
    );
    console.error(
      `  still exist and that build-webview.mjs's transforms still match.`,
    );
    process.exit(1);
  }
}

// No leftover raw stylesheet/script tags that should have been stripped.
if (/<link rel="stylesheet"[^>]*href="\/[^"]*"/.test(html)) {
  console.error("build-webview: leftover <link> tag not stripped");
  process.exit(1);
}
if (/<script[^>]*src="\/[^"]*"/.test(html)) {
  console.error("build-webview: leftover <script src=\"/...\"> tag not stripped");
  process.exit(1);
}

writeFileSync(join(WEBVIEW, "index.html"), html);
console.log(`build-webview: wrote ${join(WEBVIEW, "index.html")}`);
