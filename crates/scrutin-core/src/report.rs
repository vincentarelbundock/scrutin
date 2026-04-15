//! JUnit XML report writer.
//!
//! Translates scrutin's per-file `Vec<Message>` stream into the JUnit XML
//! schema that GitHub Actions, GitLab, Buildkite, CircleCI, Jenkins, etc.
//! all consume natively. Hand-rolled to avoid pulling in an XML dependency
//! for what is essentially a templated string emitter.
//!
//! ## Counting policy (single source of truth)
//!
//! Workers emit a stream of `Result | Skip | Error` per test, optionally
//! followed by a `Summary` carrying the worker's own tally. Those two can
//! disagree — pytest in particular can emit a `Summary` whose counts reflect
//! collection-time errors that never produced per-test `Result`s. We resolve
//! this once, in `render_suite`, by preferring the streamed per-case tally
//! (since each streamed event becomes a `<testcase>` and the suite-level
//! totals must sum to the case count by JUnit schema rule). The `Summary`
//! is used only for `time`, since worker wall-clock is more accurate than
//! the sum of per-test `ms`. The `<testsuites>` top-level totals are then
//! a straight sum of the per-suite totals — same numbers, no drift.
//!
//! ## Atomicity
//!
//! Watch mode rewrites the report every cycle. Concurrent CI/IDE consumers
//! reading the file must never see a truncated write, so we write to
//! `<path>.tmp` and `rename` over the destination.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::ops::AddAssign;
use std::path::Path;

use anyhow::{Context, Result};

use crate::engine::protocol::Message;
use crate::metadata::RunMetadata;

/// Per-suite (and aggregate) test counts. The four fields exactly mirror
/// the JUnit schema attributes on `<testsuite>` / `<testsuites>`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    tests: u32,
    failures: u32,
    errors: u32,
    skipped: u32,
}

impl AddAssign for Counts {
    fn add_assign(&mut self, rhs: Self) {
        self.tests += rhs.tests;
        self.failures += rhs.failures;
        self.errors += rhs.errors;
        self.skipped += rhs.skipped;
    }
}

struct RenderedSuite {
    xml: String,
    counts: Counts,
}

/// Write a JUnit XML report describing every test file's results.
///
/// `suites` is the same `(file_name, messages)` shape that the run
/// accumulator already produces. `flaky_files` names files that needed at
/// least one rerun before passing — those get a per-suite `scrutin.flaky=true`
/// property. `metadata`, when present, becomes a top-level `<properties>`
/// block on `<testsuites>`.
pub fn write_report(
    path: &Path,
    suites: &[(String, Vec<Message>)],
    total_elapsed_secs: f64,
    flaky_files: &HashSet<String>,
    metadata: Option<&RunMetadata>,
) -> Result<()> {
    // Render every suite first, accumulating top-level totals as we go.
    // Explicit fold (no side-effecting `.map()`) so the data dependency
    // between suite rendering and the aggregate is obvious.
    let mut totals = Counts::default();
    let mut rendered: Vec<String> = Vec::with_capacity(suites.len());
    for (file, msgs) in suites {
        let suite = render_suite(file, msgs, flaky_files.contains(file));
        totals += suite.counts;
        rendered.push(suite.xml);
    }

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    writeln!(
        out,
        "<testsuites name=\"scrutin\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\">",
        totals.tests,
        totals.failures,
        totals.errors,
        totals.skipped,
        fmt_secs(total_elapsed_secs)
    )
    .unwrap();

    if let Some(md) = metadata
        && !md.is_empty()
    {
        out.push_str("  <properties>\n");
        for (k, v) in md.iter() {
            out.push_str("    <property name=\"");
            xml_escape_into(&mut out, &k);
            out.push_str("\" value=\"");
            xml_escape_into(&mut out, &v);
            out.push_str("\"/>\n");
        }
        out.push_str("  </properties>\n");
    }
    for xml in rendered {
        out.push_str(&xml);
    }
    out.push_str("</testsuites>\n");

    write_atomic(path, out.as_bytes())
}

/// Write `bytes` to `path` atomically: stage at `<path>.tmp`, then `rename`.
/// `rename` is atomic on POSIX and on Windows ≥ Vista when target exists,
/// so concurrent readers never see a partial write.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating parent dir for {}", path.display()))?;
    }
    let mut tmp = path.to_path_buf();
    let tmp_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => format!(".{n}.tmp"),
        None => ".scrutin-junit.tmp".to_string(),
    };
    tmp.set_file_name(tmp_name);
    fs::write(&tmp, bytes)
        .with_context(|| format!("writing JUnit report tempfile {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        format!(
            "renaming {} → {} (JUnit report)",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn render_suite(file: &str, messages: &[Message], is_flaky: bool) -> RenderedSuite {
    use crate::engine::protocol::Outcome;

    let classname = classname_for(file);

    let mut counts = Counts::default();
    let mut cases = String::new();
    let mut anon_idx: u32 = 0;
    let mut summary_ms: Option<u64> = None;

    for msg in messages {
        match msg {
            Message::Event(e) => {
                let test_name_opt = if e.subject.name.is_empty() {
                    None
                } else {
                    Some(e.subject.name.as_str())
                };
                let name = case_name(test_name_opt, e.line, &mut anon_idx);
                let time = ms_to_secs(e.duration_ms);
                let body = e.message.as_deref().unwrap_or("");
                match e.outcome {
                    Outcome::Pass => {
                        counts.tests += 1;
                        writeln!(
                            cases,
                            "    <testcase classname=\"{}\" name=\"{}\" time=\"{}\"/>",
                            XmlAttr(&classname),
                            XmlAttr(&name),
                            fmt_secs(time)
                        )
                        .unwrap();
                    }
                    Outcome::Fail => {
                        counts.tests += 1;
                        counts.failures += 1;
                        write_failure_or_error(
                            &mut cases,
                            &classname,
                            &name,
                            time,
                            "failure",
                            body,
                        );
                    }
                    Outcome::Error => {
                        counts.tests += 1;
                        counts.errors += 1;
                        write_failure_or_error(
                            &mut cases,
                            &classname,
                            &name,
                            time,
                            "error",
                            body,
                        );
                    }
                    Outcome::Skip => {
                        counts.tests += 1;
                        counts.skipped += 1;
                        write_skipped(&mut cases, &classname, &name, body, "");
                    }
                    Outcome::Xfail => {
                        // JUnit has no native xfail. Render as `<skipped>`
                        // with a `message="expected"` so CI consumers can
                        // filter on it without conflating with regular skips.
                        counts.tests += 1;
                        counts.skipped += 1;
                        write_skipped(&mut cases, &classname, &name, body, "expected");
                    }
                    Outcome::Warn => {
                        // No JUnit equivalent; drop. Spec note: warn does
                        // not break the build.
                    }
                }
            }
            Message::Summary(s) => summary_ms = Some(s.duration_ms),
            Message::Deps(_) | Message::Done => {}
        }
    }

    // Per the counting-policy comment at the top: streamed per-case counts
    // are authoritative for the totals (so they sum to the case count by
    // JUnit schema rule); summary_ms is used only for the wall-clock time
    // since worker wall time is more accurate than the sum of per-test ms.
    let suite_time = summary_ms.map(ms_to_secs).unwrap_or(0.0);

    let mut xml = String::new();
    writeln!(
        xml,
        "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\">",
        XmlAttr(file),
        counts.tests,
        counts.failures,
        counts.errors,
        counts.skipped,
        fmt_secs(suite_time)
    )
    .unwrap();
    if is_flaky {
        xml.push_str(
            "    <properties>\n      <property name=\"scrutin.flaky\" value=\"true\"/>\n    </properties>\n",
        );
    }
    xml.push_str(&cases);
    xml.push_str("  </testsuite>\n");

    RenderedSuite { xml, counts }
}

fn write_failure_or_error(
    cases: &mut String,
    classname: &str,
    name: &str,
    time: f64,
    tag: &str, // "failure" or "error"
    body: &str,
) {
    write!(
        cases,
        "    <testcase classname=\"{}\" name=\"{}\" time=\"{}\">\n      <{tag} message=\"{}\" type=\"{tag}\">",
        XmlAttr(classname),
        XmlAttr(name),
        fmt_secs(time),
        XmlAttr(first_line(body)),
    )
    .unwrap();
    write_cdata(cases, body);
    write!(cases, "</{tag}>\n    </testcase>\n").unwrap();
}

/// Emit a `<testcase>` with a `<skipped>` child. `attr_message_override`,
/// when non-empty, is used in the attribute instead of the body's first
/// line — the spec uses `"expected"` for `xfail` so CI consumers can filter.
fn write_skipped(
    cases: &mut String,
    classname: &str,
    name: &str,
    body: &str,
    attr_message_override: &str,
) {
    let attr = if attr_message_override.is_empty() {
        first_line(body)
    } else {
        attr_message_override
    };
    write!(
        cases,
        "    <testcase classname=\"{}\" name=\"{}\" time=\"0.000\">\n      <skipped message=\"{}\"/>\n    </testcase>\n",
        XmlAttr(classname),
        XmlAttr(name),
        XmlAttr(attr),
    )
    .unwrap();
}

/// Emit `body` wrapped in CDATA. Splits any literal `]]>` in the body so
/// the CDATA section can't be terminated early — the standard trick is to
/// replace `]]>` with `]]]]><![CDATA[>` (close, escape `]`, reopen, `>`).
fn write_cdata(out: &mut String, body: &str) {
    out.push_str("<![CDATA[");
    let mut rest = body;
    while let Some(idx) = rest.find("]]>") {
        out.push_str(&rest[..idx]);
        out.push_str("]]]]><![CDATA[>");
        rest = &rest[idx + 3..];
    }
    out.push_str(rest);
    out.push_str("]]>");
}

/// Strip the file extension, if any, leaving the stem. Empty stems (e.g.
/// from `.hidden`) fall back to the original filename so we never emit
/// `classname=""`. JUnit convention is dotted-package; we use the basename
/// stem because that's all the writer has access to today. Future work:
/// thread the test file's relative path through `suites` and convert
/// `tests/testthat/test-math.R` → `tests.testthat.test-math`.
fn classname_for(file: &str) -> String {
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => file.to_string(),
    }
}

fn case_name(test: Option<&str>, line: Option<u32>, anon_idx: &mut u32) -> String {
    if let Some(t) = test
        && !t.is_empty()
    {
        return t.to_string();
    }
    // Always bump the counter so two anonymous tests on the same line
    // produce distinct (classname, name) pairs — JUnit consumers merge
    // duplicates by that key, which is exactly the wrong UX.
    *anon_idx += 1;
    match line {
        Some(l) => format!("anon line {l} #{}", *anon_idx),
        None => format!("anon #{}", *anon_idx),
    }
}

fn ms_to_secs(ms: u64) -> f64 {
    (ms as f64) / 1000.0
}

/// Format seconds as a JUnit-schema-safe `xs:decimal`. Clamps NaN, Inf,
/// and negatives to `0.000` so a stray `f64::NAN` from upstream doesn't
/// break consumers that schema-validate.
fn fmt_secs(s: f64) -> String {
    if !s.is_finite() || s < 0.0 {
        "0.000".to_string()
    } else {
        format!("{s:.3}")
    }
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

/// `Display`-adapter that XML-escapes a string for use inside an attribute
/// value. Lets us write `format_args!`-style and avoid intermediate
/// `String`s in the hot path.
struct XmlAttr<'a>(&'a str);

impl std::fmt::Display for XmlAttr<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for c in self.0.chars() {
            match c {
                '&' => f.write_str("&amp;")?,
                '<' => f.write_str("&lt;")?,
                '>' => f.write_str("&gt;")?,
                '"' => f.write_str("&quot;")?,
                '\'' => f.write_str("&apos;")?,
                c if is_xml_disallowed(c) => {}
                c => f.write_char(c)?,
            }
        }
        Ok(())
    }
}

/// Escape `s` for general XML text, appending into `out`. Used for the
/// metadata `<property>` block where we don't need the `Display` adapter.
fn xml_escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if is_xml_disallowed(c) => {}
            c => out.push(c),
        }
    }
}

/// Codepoints that XML 1.0 §2.2 forbids in document content: NUL, most C0
/// controls (excluding `\t \n \r`), and the U+FFFE / U+FFFF non-characters.
/// We do *not* strip the C1 range (`U+0080..=U+009F`) — XML 1.0 allows it
/// in content (only XML 1.1 restricts it), and stripping would silently
/// mangle UTF-8 text from non-Latin-1 sources.
fn is_xml_disallowed(c: char) -> bool {
    matches!(
        c as u32,
        0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0xFFFE | 0xFFFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::protocol::Message;

    fn parse(line: &str) -> Message {
        serde_json::from_str(line).unwrap()
    }

    // Helper: build an event JSON line for the new wire format.
    fn ev(file: &str, outcome: &str, name: &str, extras: &str) -> Message {
        let s = format!(
            r#"{{"type":"event","file":"{file}","outcome":"{outcome}","subject":{{"kind":"function","name":"{name}"}}{extras}}}"#
        );
        parse(&s)
    }

    // ── render_suite ────────────────────────────────────────────────────────

    #[test]
    fn renders_pass_fail_skip_error_xfail() {
        let msgs = vec![
            ev("f.R", "pass", "adds", r#","duration_ms":5"#),
            ev(
                "f.R",
                "fail",
                "sub",
                r#","message":"expected 2 got 6","line":10,"duration_ms":3"#,
            ),
            ev("f.R", "skip", "net", r#","message":"offline""#),
            ev("f.R", "error", "<crash>", r#","message":"crash","duration_ms":1"#),
            ev("f.R", "xfail", "wip", r#","message":"known broken""#),
            parse(
                r#"{"type":"summary","file":"f.R","duration_ms":42,
                    "counts":{"pass":1,"fail":1,"error":1,"skip":1,"xfail":1,"warn":0}}"#,
            ),
        ];
        let suite = render_suite("f.R", &msgs, false);
        assert_eq!(suite.counts.tests, 5);
        assert_eq!(suite.counts.failures, 1);
        assert_eq!(suite.counts.errors, 1);
        // skip + xfail both project onto JUnit `<skipped>`.
        assert_eq!(suite.counts.skipped, 2);
        assert!(suite.xml.contains("classname=\"f\""));
        assert!(suite.xml.contains("name=\"adds\""));
        assert!(suite.xml.contains("<failure message=\"expected 2 got 6\""));
        assert!(suite.xml.contains("type=\"failure\""));
        assert!(suite.xml.contains("<![CDATA[expected 2 got 6]]>"));
        assert!(suite.xml.contains("<skipped message=\"offline\""));
        // xfail uses the spec's "expected" attribute marker so consumers
        // can filter on it.
        assert!(suite.xml.contains("<skipped message=\"expected\""));
        assert!(suite.xml.contains("<error message=\"crash\""));
        assert!(suite.xml.contains("type=\"error\""));
        assert!(suite.xml.contains("time=\"0.042\""));
    }

    #[test]
    fn warn_outcome_drops_silently_no_testcase() {
        // warn has no JUnit equivalent and must not appear as a testcase.
        let msgs = vec![
            ev("f.R", "pass", "a", ""),
            ev("f.R", "warn", "b", r#","message":"deprecated""#),
        ];
        let suite = render_suite("f.R", &msgs, false);
        assert_eq!(suite.counts.tests, 1);
        assert!(!suite.xml.contains("name=\"b\""));
    }

    #[test]
    fn xml_escapes_special_chars_in_attributes() {
        let msgs = vec![ev(
            "f.R",
            "fail",
            "a &lt; b &amp; c",
            r#","message":"x > y""#,
        )];
        // Subject name uses literal `<` etc. via JSON escaping below.
        let raw = r#"{"type":"event","file":"f.R","outcome":"fail",
            "subject":{"kind":"function","name":"a < b & c"},"message":"x > y"}"#;
        let suite = render_suite("f.R", &[parse(raw)], false);
        assert!(suite.xml.contains("name=\"a &lt; b &amp; c\""));
        assert!(suite.xml.contains("&gt;"));
        assert!(!suite.xml.contains(" & ")); // no unescaped ampersand
        // Suppress unused-warning for the helper-built msgs.
        let _ = msgs;
    }

    #[test]
    fn empty_subject_name_falls_back_to_anon_with_counter() {
        let raw = r#"{"type":"event","file":"f.R","outcome":"pass",
            "subject":{"kind":"function","name":""},"line":7}"#;
        let suite = render_suite("f.R", &[parse(raw)], false);
        assert_eq!(suite.counts.tests, 1);
        assert!(suite.xml.contains("name=\"anon line 7 #1\""));
    }

    #[test]
    fn anon_names_on_same_line_do_not_collide() {
        let raw1 = r#"{"type":"event","file":"f.R","outcome":"pass",
            "subject":{"kind":"function","name":""},"line":7}"#;
        let suite = render_suite("f.R", &[parse(raw1), parse(raw1)], false);
        assert!(suite.xml.contains("name=\"anon line 7 #1\""));
        assert!(suite.xml.contains("name=\"anon line 7 #2\""));
    }

    #[test]
    fn renders_flaky_property_when_file_in_set() {
        let msgs = vec![ev("f.R", "pass", "t1", "")];
        let suite = render_suite("f.R", &msgs, true);
        assert!(
            suite
                .xml
                .contains("<property name=\"scrutin.flaky\" value=\"true\"")
        );
    }

    // ── classname_for ───────────────────────────────────────────────────────

    #[test]
    fn classname_handles_dotted_filenames() {
        assert_eq!(classname_for("test_foo.bar.py"), "test_foo.bar");
        assert_eq!(classname_for("Dockerfile"), "Dockerfile");
        // .hidden has empty stem; fall back to the original name rather
        // than emitting classname="".
        assert_eq!(classname_for(".hidden"), ".hidden");
        assert_eq!(classname_for("test-math.R"), "test-math");
    }

    // ── XML safety ──────────────────────────────────────────────────────────

    #[test]
    fn strips_xml_disallowed_codepoints() {
        // NUL, U+FFFE, U+FFFF must be removed; \t \n \r preserved.
        let body = "before\u{0000}\u{FFFE}\u{FFFF}after\n\tline2";
        let mut buf = String::new();
        xml_escape_into(&mut buf, body);
        assert_eq!(buf, "beforeafter\n\tline2");
    }

    #[test]
    fn cdata_splits_terminator_in_body() {
        let mut out = String::new();
        write_cdata(&mut out, "before]]>after");
        assert_eq!(out, "<![CDATA[before]]]]><![CDATA[>after]]>");
    }

    // ── fmt_secs ────────────────────────────────────────────────────────────

    #[test]
    fn fmt_secs_clamps_nan_inf_negative() {
        assert_eq!(fmt_secs(0.5), "0.500");
        assert_eq!(fmt_secs(f64::NAN), "0.000");
        assert_eq!(fmt_secs(f64::INFINITY), "0.000");
        assert_eq!(fmt_secs(-1.0), "0.000");
    }

    // ── write_report (atomic, file I/O) ─────────────────────────────────────

    #[test]
    fn renders_metadata_properties_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.xml");

        let suites = vec![(
            "test-a.R".to_string(),
            vec![ev("test-a.R", "pass", "t1", "")],
        )];
        let mut md = RunMetadata::default();
        md.provenance.scrutin_version = "0.0.1-test".into();
        md.provenance.git_commit = Some("abcdef".into());
        md.provenance.os_platform = Some("linux".into());
        md.labels.insert("build".into(), "4521".into());
        write_report(&path, &suites, 0.1, &HashSet::new(), Some(&md)).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("<properties>"));
        assert!(contents.contains("<property name=\"git.sha\" value=\"abcdef\""));
        assert!(contents.contains("<property name=\"os\" value=\"linux\""));
        assert!(contents.contains("<property name=\"build\" value=\"4521\""));
    }

    #[test]
    fn write_report_creates_file_with_aggregate_totals() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.xml");

        let suites = vec![
            (
                "test-a.R".to_string(),
                vec![
                    ev("test-a.R", "pass", "t1", r#","duration_ms":1"#),
                    ev("test-a.R", "fail", "t2", r#","message":"nope","duration_ms":1"#),
                    parse(
                        r#"{"type":"summary","file":"test-a.R","duration_ms":10,
                            "counts":{"pass":1,"fail":1}}"#,
                    ),
                ],
            ),
            (
                "test-b.R".to_string(),
                vec![
                    ev("test-b.R", "skip", "t3", r#","message":"todo""#),
                    parse(
                        r#"{"type":"summary","file":"test-b.R","duration_ms":2,
                            "counts":{"skip":1}}"#,
                    ),
                ],
            ),
        ];

        write_report(&path, &suites, 0.5, &HashSet::new(), None).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains(
            "<testsuites name=\"scrutin\" tests=\"3\" failures=\"1\" errors=\"0\" skipped=\"1\" time=\"0.500\""
        ));
        assert!(contents.contains("<testsuite name=\"test-a.R\""));
        assert!(contents.contains("<testsuite name=\"test-b.R\""));
    }

    #[test]
    fn write_report_is_idempotent_overwrite() {
        // Watch mode rewrites the same path repeatedly. Verify that a
        // second write replaces the first cleanly via the temp+rename path.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.xml");

        let suites_v1 = vec![(
            "f.R".to_string(),
            vec![ev("f.R", "pass", "t1", "")],
        )];
        let suites_v2 = vec![(
            "f.R".to_string(),
            vec![ev("f.R", "fail", "t2", r#","message":"new""#)],
        )];

        write_report(&path, &suites_v1, 0.1, &HashSet::new(), None).unwrap();
        write_report(&path, &suites_v2, 0.1, &HashSet::new(), None).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("name=\"t2\""));
        assert!(!contents.contains("name=\"t1\""));
        // No tempfile leak.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "tempfile leaked: {leftovers:?}");
    }
}
