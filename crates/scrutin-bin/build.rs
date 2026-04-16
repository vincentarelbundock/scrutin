// Resolve the canonical SKILL.md (at `<workspace>/skills/scrutin/SKILL.md`)
// into `OUT_DIR/SKILL.md` so `src/cli/mod.rs` can embed it via `include_str!`
// without walking outside the crate root. `cargo publish` rebuilds the crate
// from an isolated tarball that does not contain the workspace tree, so an
// in-crate copy at `crates/scrutin-bin/SKILL.md` is used as a fallback and is
// kept in sync on every workspace build.

use std::{fs, path::PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let workspace_skill = manifest_dir.join("../../skills/scrutin/SKILL.md");
    let in_crate_skill = manifest_dir.join("SKILL.md");
    let out_skill = out_dir.join("SKILL.md");

    println!("cargo:rerun-if-changed={}", workspace_skill.display());
    println!("cargo:rerun-if-changed={}", in_crate_skill.display());

    let content = if workspace_skill.exists() {
        let bytes = fs::read(&workspace_skill).expect("read workspace SKILL.md");
        let stale = fs::read(&in_crate_skill).ok().as_ref() != Some(&bytes);
        if stale {
            fs::write(&in_crate_skill, &bytes).expect("sync in-crate SKILL.md");
        }
        bytes
    } else if in_crate_skill.exists() {
        fs::read(&in_crate_skill).expect("read in-crate SKILL.md")
    } else {
        panic!(
            "SKILL.md not found at {} or {}",
            workspace_skill.display(),
            in_crate_skill.display()
        );
    };

    fs::write(&out_skill, &content).expect("write OUT_DIR SKILL.md");
}
