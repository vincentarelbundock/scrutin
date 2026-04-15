//! Embedded frontend assets. The entire dashboard (HTML, CSS, JS) is baked
//! into the binary at compile time via `include_bytes!`. No build step, no
//! proc macro, no runtime file I/O.
//!
//! Adding a module: drop the file in `frontend/modules/` and add one line
//! below. `<script type="module">` in index.html imports them transitively
//! from `./modules/*.js`, so every referenced file must appear here or the
//! browser will 404 at module-resolution time.

pub struct EmbeddedFile {
    pub data: &'static [u8],
    pub mime: &'static str,
}

const JS: &str = "application/javascript; charset=utf-8";

pub fn get(path: &str) -> Option<EmbeddedFile> {
    let data: &'static [u8] = match path {
        "index.html"              => include_bytes!("../frontend/index.html"),
        "app.js"                  => include_bytes!("../frontend/app.js"),
        "style.css"               => include_bytes!("../frontend/style.css"),
        "catppuccin-palette.css"  => include_bytes!("../frontend/catppuccin-palette.css"),
        "modules/state.js"        => include_bytes!("../frontend/modules/state.js"),
        "modules/util.js"         => include_bytes!("../frontend/modules/util.js"),
        "modules/api.js"          => include_bytes!("../frontend/modules/api.js"),
        "modules/events.js"       => include_bytes!("../frontend/modules/events.js"),
        "modules/sources.js"      => include_bytes!("../frontend/modules/sources.js"),
        "modules/sort.js"         => include_bytes!("../frontend/modules/sort.js"),
        "modules/navigation.js"   => include_bytes!("../frontend/modules/navigation.js"),
        "modules/levels.js"       => include_bytes!("../frontend/modules/levels.js"),
        "modules/palettes.js"     => include_bytes!("../frontend/modules/palettes.js"),
        "modules/help.js"         => include_bytes!("../frontend/modules/help.js"),
        "modules/theme.js"        => include_bytes!("../frontend/modules/theme.js"),
        "modules/keymap.js"       => include_bytes!("../frontend/modules/keymap.js"),
        "modules/render.js"       => include_bytes!("../frontend/modules/render.js"),
        _ => return None,
    };
    let mime = match path {
        "index.html" => "text/html; charset=utf-8",
        "style.css" | "catppuccin-palette.css" => "text/css; charset=utf-8",
        _            => JS,
    };
    Some(EmbeddedFile { data, mime })
}
