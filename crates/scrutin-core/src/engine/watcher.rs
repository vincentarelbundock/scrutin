use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::analysis::walk;
use crate::project::package::Package;

/// Lightweight wrapper around a debounced event. Unlike
/// `notify_debouncer_mini::DebouncedEvent`, this preserves the event kind
/// from the underlying `notify::Event` so callers can distinguish real
/// modifications from access-only events.
#[derive(Clone, Debug)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: EventKind,
}

pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    /// Wrapped in `Option` so callers can `take()` ownership of the receiver
    /// (e.g. to move it into a forwarder task) while keeping the `FileWatcher`
    /// — and thus the underlying watcher — alive.
    pub rx: Option<mpsc::UnboundedReceiver<Vec<WatchEvent>>>,
}

impl FileWatcher {
    pub fn new(pkg: &Package, debounce_ms: u64) -> Result<Self> {
        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<WatchEvent>();
        let (tx, rx) = mpsc::unbounded_channel::<Vec<WatchEvent>>();
        let pkg_clone = pkg.clone();

        // Raw notify callback: filter noise, forward survivors.
        let mut watcher = notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only keep events that indicate real content changes.
                    // On Linux, inotify fires Access(Close(Write)) when R's
                    // source()/load_all() opens .R files with O_RDWR then
                    // closes without writing. macOS FSEvents doesn't have
                    // this problem. Filtering at the event-kind level avoids
                    // false retriggers without hacks like cooldown timers or
                    // content hashing.
                    if is_access_only(&event.kind) {
                        return;
                    }

                    for path in event.paths {
                        if !should_ignore(&path, &pkg_clone) {
                            let _ = raw_tx.send(WatchEvent {
                                path,
                                kind: event.kind,
                            });
                        }
                    }
                }
            },
        )
        .context("Failed to create file watcher")?;

        for p in pkg
            .resolved_source_dirs()
            .into_iter()
            .chain(pkg.resolved_test_dirs())
        {
            watcher
                .watch(&p, RecursiveMode::Recursive)
                .with_context(|| format!("Failed to watch {}", p.display()))?;
        }

        // Debounce: collect events over a time window, then emit one batch
        // with deduplicated paths. Runs on a dedicated tokio task.
        let debounce = Duration::from_millis(debounce_ms);
        tokio::spawn(async move {
            loop {
                // Wait for the first event (or channel close).
                let first = match raw_rx.recv().await {
                    Some(e) => e,
                    None => break,
                };

                // Collect more events that arrive within the debounce window.
                let mut batch = vec![first];
                let deadline = tokio::time::sleep(debounce);
                tokio::pin!(deadline);
                loop {
                    tokio::select! {
                        maybe = raw_rx.recv() => {
                            match maybe {
                                Some(e) => batch.push(e),
                                None => {
                                    // Channel closed; emit what we have and exit.
                                    if !batch.is_empty() {
                                        let _ = tx.send(batch);
                                    }
                                    return;
                                }
                            }
                        }
                        _ = &mut deadline => break,
                    }
                }

                if !batch.is_empty() {
                    let _ = tx.send(batch);
                }
            }
        });

        Ok(FileWatcher {
            _watcher: watcher,
            rx: Some(rx),
        })
    }
}

/// True for events that indicate a file was accessed (opened/read/closed)
/// without any content modification. These are the spurious events that
/// inotify fires when R's source()/load_all() opens source files.
fn is_access_only(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Access(_))
}

fn should_ignore(path: &Path, pkg: &Package) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return true,
    };

    // Editor scratch files (`.#foo`, `foo~`) and R session history.
    if name.starts_with(".#") || name.ends_with('~') || name == ".Rhistory" {
        return true;
    }
    // Anything inside a noise directory (build dirs, vcs dirs, language
    // tooling caches). Centralized list shared with `analysis::walk`.
    for component in path.components() {
        if let std::path::Component::Normal(c) = component
            && let Some(s) = c.to_str()
            && walk::is_ignored_dir(s)
        {
            return true;
        }
    }
    if name.ends_with(".pyc") || name.ends_with(".pyo") {
        return true;
    }

    !(pkg.is_any_source_file(path) || pkg.is_any_test_file(path))
}

/// Deduplicate changed paths from a batch of events.
pub fn unique_paths(events: &[WatchEvent]) -> Vec<PathBuf> {
    let set: HashSet<&PathBuf> = events.iter().map(|e| &e.path).collect();
    let mut paths: Vec<PathBuf> = set.into_iter().cloned().collect();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, ModifyKind};

    fn evt(path: &str, kind: EventKind) -> WatchEvent {
        WatchEvent {
            path: PathBuf::from(path),
            kind,
        }
    }

    #[test]
    fn unique_paths_dedupes_and_sorts() {
        let events = vec![
            evt("b.R", EventKind::Modify(ModifyKind::Any)),
            evt("a.R", EventKind::Modify(ModifyKind::Any)),
            evt("a.R", EventKind::Create(CreateKind::File)),
            evt("c.R", EventKind::Modify(ModifyKind::Any)),
            evt("b.R", EventKind::Modify(ModifyKind::Any)),
        ];
        let paths = unique_paths(&events);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("a.R"),
                PathBuf::from("b.R"),
                PathBuf::from("c.R"),
            ]
        );
    }

    #[test]
    fn unique_paths_empty_input() {
        assert!(unique_paths(&[]).is_empty());
    }

    #[test]
    fn is_access_only_filters_access_events() {
        // Locks the inotify noise filter. Access(Close(Write)) fires on
        // Linux when R's source() opens .R files with O_RDWR and closes
        // without writing: those must not trigger reruns.
        assert!(is_access_only(&EventKind::Access(AccessKind::Any)));
        assert!(is_access_only(&EventKind::Access(AccessKind::Read)));
        assert!(is_access_only(&EventKind::Access(AccessKind::Close(
            notify::event::AccessMode::Read,
        ))));
        // Real content changes must pass through the filter.
        assert!(!is_access_only(&EventKind::Modify(ModifyKind::Any)));
        assert!(!is_access_only(&EventKind::Create(CreateKind::File)));
        assert!(!is_access_only(&EventKind::Remove(
            notify::event::RemoveKind::File,
        )));
    }
}
