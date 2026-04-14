//! Git integration: detect availability and collect uncommitted paths.
//!
//! Shells out to `git` rather than linking a Rust git library — git is
//! cross-platform, the porcelain v1 format is stable, and the only feature
//! we need is "files that differ from HEAD." All failures degrade gracefully
//! into one of four [`GitAvailability`] / [`GitError`] states; nothing here
//! ever panics or bubbles an error to the user as a crash.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Whether the git CLI is usable in the project's directory. Computed once
/// at startup and cached on `AppState` for the rest of the session.
#[derive(Clone, Debug)]
pub enum GitAvailability {
    /// `git` is on PATH and the project lives inside a repository.
    Available { repo_root: PathBuf },
    /// `git` works but the project is not inside a git repository.
    NotARepo,
    /// `git` is not installed (or not on PATH).
    NotInstalled,
    /// `git` ran but failed for some other reason — typically permissions
    /// or `safe.directory` ("dubious ownership") inside CI containers,
    /// which is increasingly common in 2026. The first stderr line is
    /// preserved so the TUI can surface the actual cause.
    ProbeFailed { stderr: String },
}

impl GitAvailability {
    /// One-line label for the disabled menu hint, or `None` if available.
    pub fn disabled_reason(&self) -> Option<String> {
        match self {
            GitAvailability::Available { .. } => None,
            GitAvailability::NotARepo => Some("(not a git repo)".into()),
            GitAvailability::NotInstalled => Some("(git not found)".into()),
            GitAvailability::ProbeFailed { stderr } => Some(format!("(git: {stderr})")),
        }
    }
}

/// Errors that can occur when collecting uncommitted paths after we've
/// already established git is available. Distinct from `GitAvailability`
/// because these are transient — a repo can become locked, a status call
/// can fail mid-session — and the user might fix it and try again.
#[derive(Debug)]
pub enum GitError {
    /// `git status` failed (lock file, corrupt index, etc.). Stderr's first
    /// line is included so the TUI can surface it.
    CommandFailed(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::CommandFailed(s) => write!(f, "git status failed: {s}"),
        }
    }
}

impl std::error::Error for GitError {}

/// Probe `git` once. Runs `git -C <project_root> rev-parse --show-toplevel`
/// — that handles worktrees, submodules, and `GIT_DIR` overrides for free.
///
/// On non-zero exit we look at stderr: if it contains "not a git repository"
/// we return [`GitAvailability::NotARepo`], otherwise [`GitAvailability::ProbeFailed`]
/// with the stderr line so the user can see *why* git is unhappy (e.g.
/// `safe.directory` violations in CI containers).
pub fn detect_git(project_root: &Path) -> GitAvailability {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["rev-parse", "--show-toplevel"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                GitAvailability::NotARepo
            } else {
                GitAvailability::Available {
                    repo_root: PathBuf::from(s),
                }
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let first = stderr.lines().next().unwrap_or("").trim().to_string();
            if first.is_empty() {
                GitAvailability::NotARepo
            } else if first.contains("not a git repository") {
                GitAvailability::NotARepo
            } else {
                GitAvailability::ProbeFailed { stderr: first }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => GitAvailability::NotInstalled,
        // Other I/O errors (permission denied, ENOEXEC, ...) are
        // indistinguishable from "not installed" without more probing.
        // Surface them as ProbeFailed so the user at least sees the cause.
        Err(e) => GitAvailability::ProbeFailed {
            stderr: e.to_string(),
        },
    }
}

/// Collect every path that differs from `HEAD` according to `git status`.
///
/// Returns absolute paths under `project_root`. Includes staged, unstaged,
/// and untracked files (`-uall` so directories expand to individual files).
/// Renames are reported as the new path.
pub fn changed_paths(repo_root: &Path, project_root: &Path) -> Result<Vec<PathBuf>, GitError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["status", "--porcelain=v1", "-uall", "-z"])
        .output()
        .map_err(|e| GitError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let first = stderr.lines().next().unwrap_or("unknown error").to_string();
        return Err(GitError::CommandFailed(first));
    }

    let parsed = parse_porcelain_z(&output.stdout);
    let mut paths: Vec<PathBuf> = parsed
        .into_iter()
        .map(|rel| repo_root.join(rel))
        .filter(|p| p.starts_with(project_root))
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Parse `git status --porcelain=v1 -z` output into a list of repo-relative
/// paths. The format is a stream of NUL-terminated entries; each entry is
/// `XY <space> path`. Per `git-status(1)`, an oldpath field follows iff
/// **X** is `R` or `C` (a staged rename or copy). The Y column never
/// triggers an oldpath field — worktree-only renames are reported with
/// `Y == ' '`/`'M'`/`'D'`, never `'R'`/`'C'`, even with rename detection on.
fn parse_porcelain_z(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let fields: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    let mut i = 0;
    while i < fields.len() {
        let entry = fields[i];
        i += 1;
        if entry.len() < 4 {
            // Trailing empty entry from final NUL, or malformed line.
            continue;
        }
        let x = entry[0];
        // entry[1] is the Y column; entry[2] is the space separator.
        let path = &entry[3..];
        out.push(String::from_utf8_lossy(path).into_owned());
        if x == b'R' || x == b'C' {
            // Skip the oldpath field that follows a staged rename/copy.
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_modifications() {
        let input = b" M src/foo.rs\0?? new.txt\0";
        let got = parse_porcelain_z(input);
        assert_eq!(got, vec!["src/foo.rs", "new.txt"]);
    }

    #[test]
    fn parses_rename_and_skips_oldpath() {
        // Staged rename: X='R', oldpath follows.
        let input = b"R  newpath\0oldpath\0 M other.rs\0";
        let got = parse_porcelain_z(input);
        assert_eq!(got, vec!["newpath", "other.rs"]);
    }

    #[test]
    fn parses_staged_copy_and_skips_oldpath() {
        // Staged copy: X='C', oldpath follows.
        let input = b"C  newcopy\0source.rs\0?? other.txt\0";
        let got = parse_porcelain_z(input);
        assert_eq!(got, vec!["newcopy", "other.txt"]);
    }

    #[test]
    fn parses_rm_status_without_extra_skip() {
        // Staged rename + worktree modify (`RM`). Only the staged side
        // (X='R') triggers an oldpath field — Y='M' does not. The old
        // code's defensive `y == 'R' || y == 'C'` check would have
        // skipped a non-existent extra field here and desynced the stream.
        let input = b"RM newname\0oldname\0?? added.txt\0";
        let got = parse_porcelain_z(input);
        assert_eq!(got, vec!["newname", "added.txt"]);
    }

    #[test]
    fn parses_filename_with_spaces() {
        // The `XY ` prefix is fixed-width 3 bytes; everything after is
        // the path verbatim, so spaces inside the filename round-trip.
        let input = b" M src/foo bar.rs\0?? a b c.txt\0";
        let got = parse_porcelain_z(input);
        assert_eq!(got, vec!["src/foo bar.rs", "a b c.txt"]);
    }

    #[test]
    fn handles_empty_input() {
        assert!(parse_porcelain_z(b"").is_empty());
    }

    #[test]
    fn probe_failed_disabled_reason_includes_stderr() {
        let av = GitAvailability::ProbeFailed {
            stderr: "fatal: detected dubious ownership in /repo".into(),
        };
        let reason = av.disabled_reason().unwrap();
        assert!(reason.contains("dubious ownership"));
    }
}
