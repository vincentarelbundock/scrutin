//! Shared ring-buffered log for subprocess stderr and internal messages.
//!
//! A `LogBuffer` is cheap to clone (it's an `Arc`) and is safe to write to
//! from any thread. The TUI's Log pane reads a `snapshot()` each frame.
//!
//! When the buffer is full, the oldest line is dropped to make room. The
//! cumulative count of dropped lines is exposed via [`LogBuffer::dropped`]
//! so the UI can render a "(N lines lost)" hint instead of silently losing
//! diagnostic output.
//!
//! Mutex poisoning (a writer panicked while holding the lock) is handled
//! by recovering the inner state — the log keeps working, which matters
//! because a panic is exactly when you most need to read the log.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Default capacity. Picked to balance scroll latency and memory; 1000
/// short stderr lines is ~80 KB. Exposed `pub` so an upcoming `[ui]` config
/// section can override it without touching this file.
pub const DEFAULT_MAX_LINES: usize = 1000;

struct Inner {
    queue: VecDeque<String>,
    /// Cumulative count of lines dropped because the queue was at capacity.
    /// Monotonically increasing for the lifetime of the buffer.
    dropped: u64,
    capacity: usize,
}

#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<Inner>>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_LINES)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                queue: VecDeque::with_capacity(capacity),
                dropped: 0,
                capacity,
            })),
        }
    }

    /// Append one line. Trailing `\r`/`\n` are stripped; empty lines are
    /// dropped. The line is prefixed with `[source]` so the UI can tell
    /// where it came from. Line dropouts due to full-buffer eviction
    /// increment the counter exposed by [`Self::dropped`].
    pub fn push(&self, source: &str, line: &str) {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed.is_empty() {
            return;
        }
        let formatted = format!("[{source}] {trimmed}");
        let mut inner = self.lock();
        if inner.queue.len() == inner.capacity {
            inner.queue.pop_front();
            inner.dropped += 1;
        }
        inner.queue.push_back(formatted);
    }

    /// Snapshot every line currently in the buffer. Cloned because the
    /// caller (the TUI render path) wants an owned `Vec<String>` per frame.
    pub fn snapshot(&self) -> Vec<String> {
        self.lock().queue.iter().cloned().collect()
    }

    /// Cumulative count of lines dropped due to capacity. Cheap (single
    /// lock acquisition) so the TUI can poll it every frame.
    pub fn dropped(&self) -> u64 {
        self.lock().dropped
    }

    pub fn len(&self) -> usize {
        self.lock().queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Acquire the inner mutex, recovering from poisoning. A poisoned mutex
    /// means a prior writer panicked mid-update; the queue may be in an
    /// imperfect state but is still consistent enough to use, and silently
    /// dropping every subsequent log line would defeat the buffer's whole
    /// purpose during incident investigation.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_snapshot_round_trip() {
        let buf = LogBuffer::new();
        buf.push("worker", "hello");
        buf.push("worker", "world");
        let snap = buf.snapshot();
        assert_eq!(snap, vec!["[worker] hello", "[worker] world"]);
    }

    #[test]
    fn empty_lines_are_dropped() {
        let buf = LogBuffer::new();
        buf.push("w", "");
        buf.push("w", "\n");
        buf.push("w", "\r\n");
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn trailing_newline_stripped() {
        let buf = LogBuffer::new();
        buf.push("w", "line\n");
        buf.push("w", "line\r\n");
        assert_eq!(buf.snapshot(), vec!["[w] line", "[w] line"]);
    }

    #[test]
    fn dropped_counter_tracks_evictions() {
        let buf = LogBuffer::with_capacity(3);
        for i in 0..5 {
            buf.push("w", &format!("line {i}"));
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.dropped(), 2);
        assert_eq!(
            buf.snapshot(),
            vec!["[w] line 2", "[w] line 3", "[w] line 4"]
        );
    }
}
