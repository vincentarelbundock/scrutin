//! Run provenance capture.
//!
//! Collects automatic metadata about a test run (git state, host, OS, CI
//! provider, scrutin version, build info) into a typed [`Provenance`]
//! struct whose fields map 1:1 to the `runs` table columns. User-supplied
//! `[extras]` labels are carried alongside in [`RunMetadata::labels`].

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::git::{self, GitAvailability};

/// Run-level provenance. One-to-one with the `runs` table columns in
/// `SCHEMA.md`. Every field is optional *except* `scrutin_version`: the
/// binary always knows its own version. Filled in by
/// [`capture_provenance`]; consumed by the SQLite writer and the JUnit
/// reporter.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    pub hostname: Option<String>,
    pub ci: Option<String>,
    pub scrutin_version: String,

    pub git_commit: Option<String>,
    pub git_branch: Option<String>,
    pub git_dirty: Option<bool>,

    pub repo_name: Option<String>,
    pub repo_url: Option<String>,
    pub repo_root: Option<String>,

    pub build_number: Option<String>,
    pub build_id: Option<String>,
    pub build_name: Option<String>,
    pub build_url: Option<String>,

    pub os_platform: Option<String>,
    pub os_release: Option<String>,
    pub os_version: Option<String>,
    pub os_arch: Option<String>,

    /// Tool names string (e.g. `"tinytest+testthat+pytest"`). Not a column
    /// on `runs` (the per-tool value lives on `results.tool`) but carried
    /// through so the JUnit reporter can emit it as a `<property>`.
    pub tool: Option<String>,
    /// Number of workers. Not a column on `runs`; same rationale as `tool`.
    pub workers: Option<usize>,
}

impl Provenance {
    /// Key/value iterator used by the JUnit `<properties>` renderer. Keeps
    /// the old flat-dict shape so JUnit output stays unchanged during the
    /// storage swap.
    pub fn junit_pairs(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        out.push(("scrutin.version", self.scrutin_version.clone()));
        if let Some(ref v) = self.os_platform {
            out.push(("os", v.clone()));
        }
        if let Some(ref v) = self.os_arch {
            out.push(("arch", v.clone()));
        }
        if let Some(ref v) = self.hostname {
            out.push(("hostname", v.clone()));
        }
        if let Some(ref v) = self.ci {
            out.push(("ci", v.clone()));
        }
        if let Some(ref v) = self.git_commit {
            out.push(("git.sha", v.clone()));
        }
        if let Some(ref v) = self.git_branch {
            out.push(("git.branch", v.clone()));
        }
        if let Some(v) = self.git_dirty {
            out.push(("git.dirty", if v { "true" } else { "false" }.into()));
        }
        if let Some(ref v) = self.repo_name {
            out.push(("repo.name", v.clone()));
        }
        if let Some(ref v) = self.repo_url {
            out.push(("repo.url", v.clone()));
        }
        if let Some(ref v) = self.tool {
            out.push(("tool", v.clone()));
        }
        if let Some(v) = self.workers {
            out.push(("workers", v.to_string()));
        }
        out
    }
}

/// One run's worth of provenance plus user labels.
///
/// `provenance` is the typed automatic capture; `labels` carries the
/// free-form `[extras]` key/value pairs from `.scrutin/config.toml` and
/// `--set extras.KEY=VALUE` CLI overrides.
#[derive(Debug, Clone, Default)]
pub struct RunMetadata {
    /// Automatic provenance. Populated by [`capture_provenance`]; an
    /// all-`None` value means `[metadata] enabled = false`.
    pub provenance: Provenance,
    /// User labels from `[extras]` in `.scrutin/config.toml`.
    pub labels: BTreeMap<String, String>,
}

impl RunMetadata {
    /// Iterate provenance pairs (flat strings) then label pairs, in
    /// deterministic order. Used by the JUnit `<properties>` writer.
    pub fn iter(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.provenance
            .junit_pairs()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .chain(self.labels.iter().map(|(k, v)| (k.clone(), v.clone())))
    }

    pub fn is_empty(&self) -> bool {
        self.provenance.scrutin_version.is_empty() && self.labels.is_empty()
    }
}

/// Capture provenance for a project at `project_root`. Cheap (~3 ms);
/// safe to call once per run. When `enabled` is false, returns a
/// [`Provenance`] with only `scrutin_version` set (the binary always
/// knows its own version, and JUnit rendering keys off it).
pub fn capture_provenance(project_root: &Path, enabled: bool) -> Provenance {
    let mut p = Provenance {
        scrutin_version: env!("CARGO_PKG_VERSION").to_string(),
        ..Default::default()
    };
    if !enabled {
        return p;
    }

    p.os_platform = Some(std::env::consts::OS.to_string());
    p.os_arch = Some(std::env::consts::ARCH.to_string());
    p.os_release = os_release();
    p.os_version = os_version();
    p.hostname = hostname();
    p.ci = detect_ci().map(String::from);

    // Git: only run if the project sits inside a repo.
    if let GitAvailability::Available { repo_root } = git::detect_git(project_root) {
        if let Some(out) = run_git(&repo_root, &["rev-parse", "HEAD", "--abbrev-ref", "HEAD"]) {
            let mut lines = out.lines();
            if let Some(sha) = lines.next()
                && !sha.is_empty()
            {
                p.git_commit = Some(sha.to_string());
            }
            if let Some(branch) = lines.next() {
                let branch = if branch == "HEAD" { "(detached)" } else { branch };
                p.git_branch = Some(branch.to_string());
            }
        }
        if let Some(porcelain) = run_git(&repo_root, &["status", "--porcelain"]) {
            p.git_dirty = Some(!porcelain.is_empty());
        }

        p.repo_root = repo_root.to_str().map(String::from);
        p.repo_url = run_git(&repo_root, &["config", "--get", "remote.origin.url"])
            .filter(|s| !s.is_empty());
        p.repo_name = p
            .repo_url
            .as_deref()
            .and_then(repo_name_from_url)
            .or_else(|| {
                repo_root
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(String::from)
            });
    }

    // CI build fields. Only one provider will have its env vars present.
    populate_build_fields(&mut p);

    p
}

/// Populate `build_*` columns from CI provider env vars. Dispatches on the
/// same signature `detect_ci` uses.
fn populate_build_fields(p: &mut Provenance) {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());

    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        p.build_number = get("GITHUB_RUN_NUMBER");
        p.build_id = get("GITHUB_RUN_ID");
        p.build_name = get("GITHUB_WORKFLOW");
        if let (Some(server), Some(repo), Some(run_id)) = (
            get("GITHUB_SERVER_URL"),
            get("GITHUB_REPOSITORY"),
            get("GITHUB_RUN_ID"),
        ) {
            p.build_url = Some(format!("{server}/{repo}/actions/runs/{run_id}"));
        }
    } else if std::env::var_os("GITLAB_CI").is_some() {
        p.build_number = get("CI_PIPELINE_IID");
        p.build_id = get("CI_PIPELINE_ID");
        p.build_name = get("CI_JOB_NAME");
        p.build_url = get("CI_PIPELINE_URL");
    } else if std::env::var_os("BUILDKITE").is_some() {
        p.build_number = get("BUILDKITE_BUILD_NUMBER");
        p.build_id = get("BUILDKITE_BUILD_ID");
        p.build_name = get("BUILDKITE_PIPELINE_SLUG");
        p.build_url = get("BUILDKITE_BUILD_URL");
    } else if std::env::var_os("CIRCLECI").is_some() {
        p.build_number = get("CIRCLE_BUILD_NUM");
        p.build_id = get("CIRCLE_WORKFLOW_ID");
        p.build_name = get("CIRCLE_JOB");
        p.build_url = get("CIRCLE_BUILD_URL");
    } else if std::env::var_os("JENKINS_URL").is_some() {
        p.build_number = get("BUILD_NUMBER");
        p.build_id = get("BUILD_ID");
        p.build_name = get("JOB_NAME");
        p.build_url = get("BUILD_URL");
    } else if std::env::var_os("TF_BUILD").is_some() {
        p.build_number = get("BUILD_BUILDNUMBER");
        p.build_id = get("BUILD_BUILDID");
        p.build_name = get("BUILD_DEFINITIONNAME");
        if let (Some(base), Some(proj), Some(bid)) = (
            get("SYSTEM_TEAMFOUNDATIONCOLLECTIONURI"),
            get("SYSTEM_TEAMPROJECT"),
            get("BUILD_BUILDID"),
        ) {
            let base = base.trim_end_matches('/');
            p.build_url = Some(format!("{base}/{proj}/_build/results?buildId={bid}"));
        }
    } else if std::env::var_os("TRAVIS").is_some() {
        p.build_number = get("TRAVIS_BUILD_NUMBER");
        p.build_id = get("TRAVIS_BUILD_ID");
        p.build_name = get("TRAVIS_REPO_SLUG");
        p.build_url = get("TRAVIS_BUILD_WEB_URL");
    }
}

/// Detect a known CI provider from its environment-variable signature.
fn detect_ci() -> Option<&'static str> {
    if std::env::var_os("GITHUB_ACTIONS").is_some() {
        return Some("github");
    }
    if std::env::var_os("GITLAB_CI").is_some() {
        return Some("gitlab");
    }
    if std::env::var_os("BUILDKITE").is_some() {
        return Some("buildkite");
    }
    if std::env::var_os("CIRCLECI").is_some() {
        return Some("circleci");
    }
    if std::env::var_os("JENKINS_URL").is_some() {
        return Some("jenkins");
    }
    if std::env::var_os("TF_BUILD").is_some() {
        return Some("azure-pipelines");
    }
    if std::env::var_os("TRAVIS").is_some() {
        return Some("travis");
    }
    if std::env::var("CI").map(|v| v == "true" || v == "1").unwrap_or(false) {
        return Some("ci");
    }
    None
}

fn hostname() -> Option<String> {
    #[cfg(windows)]
    if let Ok(h) = std::env::var("COMPUTERNAME")
        && !h.is_empty()
    {
        return Some(h);
    }
    Command::new("hostname")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Kernel release: `uname -r` on unix. On Windows we return `None` and
/// leave the column NULL; `os_version` below carries the user-facing
/// version string.
fn os_release() -> Option<String> {
    #[cfg(unix)]
    {
        Command::new("uname")
            .arg("-r")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Human-readable OS version: `sw_vers -productVersion` on macOS,
/// `/etc/os-release`'s PRETTY_NAME on other unixes, `ver` output on
/// Windows.
fn os_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                format!(
                    "macOS {}",
                    String::from_utf8_lossy(&o.stdout).trim()
                )
            })
            .filter(|s| s != "macOS ");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
                    let rest = rest.trim().trim_matches('"');
                    if !rest.is_empty() {
                        return Some(rest.to_string());
                    }
                }
            }
        }
        return None;
    }
    #[cfg(windows)]
    {
        return Command::new("cmd")
            .args(["/C", "ver"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
    }
    #[allow(unreachable_code)]
    None
}

/// Extract an `owner/name` identifier from a git remote URL. Handles
/// HTTPS (`https://host/owner/name(.git)?`), SSH (`git@host:owner/name.git`),
/// and `git://` forms. Returns `None` if the URL doesn't have at least two
/// path segments (bare hosts or local paths).
fn repo_name_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    // Strip scheme or SSH user prefix, then find the owner/name suffix.
    let after_host = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        rest.split_once('/').map(|(_, r)| r)?
    } else if let Some((_, rest)) = url.split_once('@') {
        rest.split_once(':').map(|(_, r)| r)?
    } else {
        url
    };
    let trimmed = after_host.trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<&str> = trimmed.rsplitn(3, '/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    Some(format!("{}/{}", parts[1], parts[0]))
}

fn run_git(repo_root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_disabled_returns_version_only() {
        let p = capture_provenance(Path::new("."), false);
        assert_eq!(p.scrutin_version, env!("CARGO_PKG_VERSION"));
        assert!(p.os_platform.is_none());
        assert!(p.hostname.is_none());
    }

    #[test]
    fn provenance_enabled_populates_version_os_arch() {
        let p = capture_provenance(Path::new("."), true);
        assert_eq!(p.scrutin_version, env!("CARGO_PKG_VERSION"));
        assert!(p.os_platform.is_some());
        assert!(p.os_arch.is_some());
    }

    #[test]
    fn repo_name_from_https_url() {
        assert_eq!(
            repo_name_from_url("https://github.com/vincentarelbundock/scrutin.git"),
            Some("vincentarelbundock/scrutin".into())
        );
        assert_eq!(
            repo_name_from_url("https://github.com/vincentarelbundock/scrutin"),
            Some("vincentarelbundock/scrutin".into())
        );
    }

    #[test]
    fn repo_name_from_ssh_url() {
        assert_eq!(
            repo_name_from_url("git@github.com:vincentarelbundock/scrutin.git"),
            Some("vincentarelbundock/scrutin".into())
        );
    }

    #[test]
    fn repo_name_from_git_url() {
        assert_eq!(
            repo_name_from_url("git://example.com/group/subgroup/project.git"),
            Some("subgroup/project".into())
        );
    }

    #[test]
    fn repo_name_from_invalid_url_returns_none() {
        assert_eq!(repo_name_from_url(""), None);
        assert_eq!(repo_name_from_url("not-a-url"), None);
    }

    #[test]
    fn junit_pairs_includes_scrutin_version() {
        let p = Provenance {
            scrutin_version: "0.0.1".into(),
            ..Default::default()
        };
        let pairs = p.junit_pairs();
        assert!(pairs.iter().any(|(k, _)| *k == "scrutin.version"));
    }
}
