//! skyspell spell-check plugin (command mode).
//!
//! skyspell (https://github.com/your-tools/skyspell) is a source-aware CLI
//! spell checker: it tokenizes code + prose, skips CamelCase / identifiers
//! by default, and ships a personal + per-project ignore list. It prints
//! machine-readable JSON via `--output-format json`, which we parse in Rust
//! exactly like the jarl and ruff plugins.
//!
//! Opt-in: detection only fires when `skyspell.toml` exists at the project
//! root. The marker is purely an on/off flag; per-suite configuration lives
//! in `.scrutin/config.toml [skyspell]`, mirroring the `[pytest]` pattern:
//!
//! ```toml
//! [skyspell]
//! extra_args = ["--lang", "en_US"]   # args before the subcommand
//! add_args = ["--project"]            # args after `add`, before <WORD>
//! ```

pub mod plugin;

use std::path::Path;

use crate::engine::protocol::Correction;

/// Marker file that opts a project into the skyspell suite. Mirrors the
/// `jarl.toml` convention.
pub const MARKER: &str = "skyspell.toml";

/// Result of [`add_word_to_dict`]: which ignore list the word landed in,
/// for display in the log / toast.
pub enum AddScope {
    Project(std::path::PathBuf),
    Global,
}

/// Read `file_path`, replace the byte range described by `correction` with
/// `replacement`, and write it back. Errors when the line isn't found, the
/// range is out of bounds, or the bytes at that range don't match the word
/// the correction was generated for (a mismatch means the file drifted
/// since the spell-check run).
///
/// skyspell reports `col_start`/`col_end` as 1-based byte offsets within
/// the line (exact for ASCII, approximate for multi-byte UTF-8 prose).
pub fn apply_correction_to_file(
    file_path: &Path,
    correction: &Correction,
    replacement: &str,
) -> Result<(), String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| format!("read {}: {}", file_path.display(), e))?;

    let mut line_start = if correction.line == 1 { Some(0) } else { None };
    if line_start.is_none() {
        let mut cur: u32 = 1;
        for (i, b) in content.bytes().enumerate() {
            if b == b'\n' {
                cur += 1;
                if cur == correction.line {
                    line_start = Some(i + 1);
                    break;
                }
            }
        }
    }
    let line_start =
        line_start.ok_or_else(|| format!("line {} not found", correction.line))?;

    let byte_start = line_start + (correction.col_start as usize).saturating_sub(1);
    let byte_end = line_start + correction.col_end as usize;
    if byte_end > content.len() || byte_start >= byte_end {
        return Err(format!(
            "byte range {}..{} out of bounds",
            byte_start, byte_end
        ));
    }
    let actual = &content[byte_start..byte_end];
    if actual != correction.word {
        return Err(format!(
            "word mismatch at {}:{}: expected {:?}, found {:?}",
            correction.line, correction.col_start, correction.word, actual
        ));
    }

    let mut out = String::with_capacity(
        content.len() + replacement.len().saturating_sub(correction.word.len()),
    );
    out.push_str(&content[..byte_start]);
    out.push_str(replacement);
    out.push_str(&content[byte_end..]);
    std::fs::write(file_path, out)
        .map_err(|e| format!("write {}: {}", file_path.display(), e))
}

/// Shell out to
/// `skyspell --project-path <SUITE> <EXTRA_ARGS> add <ADD_ARGS> <WORD>`.
/// Returns which scope the word landed in so callers can log a descriptive
/// message. Any subprocess error bubbles up as `Err(stderr or message)`.
pub fn add_word_to_dict(
    suite_root: &Path,
    extra_args: &[String],
    add_args: &[String],
    word: &str,
) -> Result<AddScope, String> {
    let mut cmd = std::process::Command::new("skyspell");
    cmd.arg("--project-path")
        .arg(suite_root)
        .args(extra_args)
        .arg("add")
        .args(add_args)
        .arg(word);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("skyspell add exited {}", out.status)
        } else {
            err
        });
    }
    let scope = if add_args.iter().any(|a| a == "--project") {
        AddScope::Project(suite_root.join("skyspell-ignore.toml"))
    } else {
        AddScope::Global
    };
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::protocol::Correction;

    fn correction(word: &str, line: u32, col_start: u32, col_end: u32) -> Correction {
        Correction {
            word: word.into(),
            line,
            col_start,
            col_end,
            suggestions: Vec::new(),
        }
    }

    #[test]
    fn apply_correction_line_1() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("one.md");
        std::fs::write(&path, "teh quick brown fox\n").unwrap();

        apply_correction_to_file(&path, &correction("teh", 1, 1, 3), "the").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "the quick brown fox\n");
    }

    #[test]
    fn apply_correction_mid_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.md");
        std::fs::write(&path, "first line\nsecond teh line\nthird line\n").unwrap();

        // "teh" on line 2 starts at column 8 (1-based, bytes).
        apply_correction_to_file(&path, &correction("teh", 2, 8, 10), "the").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "first line\nsecond the line\nthird line\n",
        );
    }

    #[test]
    fn apply_correction_replacement_shorter_or_longer() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.md");
        std::fs::write(&path, "aaa behaviour bbb\n").unwrap();

        apply_correction_to_file(&path, &correction("behaviour", 1, 5, 13), "behavior").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "aaa behavior bbb\n");

        // Now back the other way (grow).
        apply_correction_to_file(&path, &correction("behavior", 1, 5, 12), "behaviour").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "aaa behaviour bbb\n");
    }

    #[test]
    fn apply_correction_detects_word_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.md");
        std::fs::write(&path, "teh quick\n").unwrap();

        let err = apply_correction_to_file(
            &path,
            &correction("foo", 1, 1, 3),
            "bar",
        )
        .expect_err("word mismatch must error instead of clobbering");
        assert!(err.contains("word mismatch"));
        // File on disk must be untouched when the mismatch fires.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "teh quick\n");
    }

    #[test]
    fn apply_correction_out_of_range_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.md");
        std::fs::write(&path, "only one line\n").unwrap();

        let err = apply_correction_to_file(&path, &correction("teh", 5, 1, 3), "the")
            .expect_err("line out of range must error");
        assert!(err.contains("line 5"));
    }

    #[test]
    fn apply_correction_out_of_range_column() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.md");
        std::fs::write(&path, "ab\n").unwrap();

        let err = apply_correction_to_file(&path, &correction("cd", 1, 10, 11), "de")
            .expect_err("column beyond line length must error");
        assert!(err.contains("out of bounds"));
    }
}
