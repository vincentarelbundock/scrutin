//! Tell cargo to re-run (and re-embed) whenever a frontend file changes.
//! rust-embed reads files at compile time; without explicit
//! `rerun-if-changed` directives cargo won't know to invalidate the
//! crate when only HTML/CSS/JS is touched, so the embedded assets can
//! go stale and the browser sees a mismatch.

fn main() {
    let frontend = std::path::Path::new("frontend");
    if frontend.is_dir() {
        emit_rerun_recursive(frontend);
    }
    println!("cargo:rerun-if-changed=build.rs");
}

fn emit_rerun_recursive(dir: &std::path::Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                emit_rerun_recursive(&path);
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}
