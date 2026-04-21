//! Lightweight notice buffer: short, UI-level messages pushed from anywhere
//! and drained by frontends (TUI, web) on startup to populate their
//! notification bars.
//!
//! Unlike [`crate::logbuf::LogBuffer`], which is a high-volume ring buffer
//! for subprocess stderr, this is a small FIFO of human-readable notices
//! (config warnings, compatibility downgrades, etc.). Notices also get
//! forwarded into the `LogBuffer` so they persist in the `L` overlay.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Clone, Default)]
pub struct NoticeBuffer {
    inner: Arc<Mutex<VecDeque<String>>>,
}

impl NoticeBuffer {
    pub fn push(&self, msg: impl Into<String>) {
        self.lock().push_back(msg.into());
    }

    pub fn drain_all(&self) -> Vec<String> {
        self.lock().drain(..).collect()
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<String>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
