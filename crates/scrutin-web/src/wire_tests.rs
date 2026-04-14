//! Unit tests for wire-type round-tripping. Lives as a sibling module to
//! wire.rs so every field is exercised through the full serde pipeline.

#[cfg(test)]
mod tests {
    use crate::wire::*;
    use scrutin_core::engine::protocol::{Counts, FileStatus};

    #[test]
    fn file_id_roundtrips_through_hex() {
        let id = FileId::of(std::path::Path::new("tests/test_foo.R"));
        let hex = id.to_string();
        let parsed: FileId = hex.parse().unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn outcome_serializes_snake_case() {
        let o = WireOutcome::Xfail;
        let s = serde_json::to_string(&o).unwrap();
        assert_eq!(s, "\"xfail\"");
    }

    #[test]
    fn status_from_counts_handles_empty_and_bad() {
        let empty = Counts::default();
        assert_eq!(FileStatus::from_counts(&empty, false), FileStatus::Pending);
        assert_eq!(FileStatus::from_counts(&empty, true), FileStatus::Cancelled);

        let passed = Counts { pass: 3, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&passed, false), FileStatus::Passed);

        let failed = Counts { pass: 3, fail: 1, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&failed, false), FileStatus::Failed);

        let errored = Counts { pass: 3, error: 1, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&errored, false), FileStatus::Failed);
    }

    #[test]
    fn status_from_counts_skip_only_is_skipped() {
        let skipped = Counts { skip: 5, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&skipped, false), FileStatus::Skipped);

        // skip + pass = passed (not skipped)
        let mixed = Counts { pass: 1, skip: 5, ..Default::default() };
        assert_eq!(FileStatus::from_counts(&mixed, false), FileStatus::Passed);
    }

    #[test]
    fn wire_status_distinguishes_failed_and_errored() {
        let failed_counts = WireCounts { fail: 1, ..Default::default() };
        assert_eq!(wire_status(FileStatus::Failed, &failed_counts), WireStatus::Failed);

        let errored_counts = WireCounts { error: 1, ..Default::default() };
        assert_eq!(wire_status(FileStatus::Failed, &errored_counts), WireStatus::Errored);
    }

    #[test]
    fn counts_bad_rule_excludes_warn_and_skip_and_xfail() {
        let c = Counts { warn: 5, skip: 2, xfail: 1, ..Default::default() };
        assert!(!c.bad(), "warn+skip+xfail alone should not be bad");

        let c2 = Counts { fail: 1, ..Default::default() };
        assert!(c2.bad());
    }

    #[test]
    fn wire_event_roundtrip() {
        let ev = WireEvent::RunStarted {
            run_id: RunId::new(),
            started_at: "2026-04-09T12:00:00Z".into(),
            files: vec![FileId::of(std::path::Path::new("a.R"))],
        };
        let s = serde_json::to_string(&ev).unwrap();
        let back: WireEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind_str(), "run_started");
    }
}
