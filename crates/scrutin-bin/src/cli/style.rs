//! Minimal ANSI styling helpers. Avoids pulling in a coloring crate for
//! the handful of styled lines in the plain reporter and stats output.
//!
//! All helpers return a `String`; callers print via `eprintln!`.

use std::fmt::Display;

// Standard SGR codes.
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";

/// Whether color output is enabled. Defaults to true; set to false via
/// `[run] color = false` in .scrutin/config.toml.
static COLOR_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

pub fn disable_color() {
    COLOR_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
}

fn styled(codes: &str, v: impl Display) -> String {
    if COLOR_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        format!("{codes}{v}{RESET}")
    } else {
        v.to_string()
    }
}

pub fn dim(v: impl Display) -> String { styled(DIM, v) }
pub fn bold(v: impl Display) -> String { styled(BOLD, v) }
pub fn red_bold(v: impl Display) -> String { styled(&format!("{RED}{BOLD}"), v) }
pub fn green_bold(v: impl Display) -> String { styled(&format!("{GREEN}{BOLD}"), v) }
pub fn yellow(v: impl Display) -> String { styled(YELLOW, v) }
pub fn yellow_bold(v: impl Display) -> String { styled(&format!("{YELLOW}{BOLD}"), v) }
pub fn cyan(v: impl Display) -> String { styled(CYAN, v) }
pub fn cyan_bold(v: impl Display) -> String { styled(&format!("{CYAN}{BOLD}"), v) }
