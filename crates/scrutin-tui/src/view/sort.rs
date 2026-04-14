//! Test-list sorting (file-list sorting lives in `state.rs::AppState::sort_files`).

use crate::state::{SortMode, TestEntry};

/// Sort a test list according to the active sort mode. `Suite` is a no-op
/// for tests because tests don't carry a suite independent from their
/// owning file.
pub(crate) fn sort_tests(tests: &mut [TestEntry], mode: SortMode, reversed: bool) {
    match mode {
        SortMode::Sequential => {} // preserve original order
        SortMode::Status     => tests.sort_by_key(|t| t.outcome.rank()),
        SortMode::Name       => tests.sort_by(|a, b| a.name.cmp(&b.name)),
        SortMode::Time       => tests.sort_by(|a, b| b.duration_ms.cmp(&a.duration_ms)),
        SortMode::Suite      => {}
    }
    if reversed {
        tests.reverse();
    }
}
