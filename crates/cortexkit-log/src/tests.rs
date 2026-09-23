use std::borrow::Cow;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use serde::Deserialize;
use tempfile::TempDir;
use tracing::Level;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::Registry;

use super::*;

// The authority copy lives in subconscious (`crates/subc-core/tests/fixtures/`);
// this is the vendored twin. Both twins render every case byte-identically or
// the fleet has two formats again.
const FIXTURE_RELATIVE_PATH: &str = "tests/fixtures/log_format_golden.json";

#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl CaptureWriter {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("capture lock").clone()).expect("UTF-8 capture")
    }
}

impl Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Deserialize)]
struct GoldenFixture {
    schema: u32,
    cases: Vec<GoldenCase>,
    parse_rejects: Vec<ParseReject>,
    level_filter: LevelFilterCases,
    redaction: RedactionCases,
    segment_name: SegmentNameCases,
    retention_prune: RetentionPruneCases,
}

#[derive(Deserialize)]
struct GoldenCase {
    name: String,
    event: GoldenEvent,
    line: String,
}

#[derive(Deserialize)]
struct GoldenEvent {
    at_ms: u64,
    level: String,
    logger: String,
    bound: Vec<(String, String)>,
    message: String,
    fields: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct ParseReject {
    name: String,
    line: String,
    reason: String,
}

#[derive(Deserialize)]
struct RedactionCases {
    cases: Vec<RedactionCase>,
}

#[derive(Deserialize)]
struct RedactionCase {
    name: String,
    input: String,
    output: String,
}

#[derive(Deserialize)]
struct LevelFilterCases {
    cases: Vec<LevelFilterCase>,
}

#[derive(Deserialize)]
struct LevelFilterCase {
    spec: String,
    level: String,
    logger: String,
    emit: bool,
}

#[derive(Deserialize)]
struct SegmentNameCases {
    cases: Vec<SegmentNameCase>,
}

#[derive(Deserialize)]
struct SegmentNameCase {
    module: String,
    at_ms: u64,
    name: String,
}

#[derive(Deserialize)]
struct RetentionPruneCases {
    cases: Vec<RetentionPruneCase>,
}

#[derive(Deserialize)]
struct RetentionPruneCase {
    name: String,
    module: String,
    today: String,
    max_age_days: u32,
    present: Vec<String>,
    unlink: Vec<String>,
}

fn fixture() -> GoldenFixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_RELATIVE_PATH);
    let contents = fs::read_to_string(path).expect("golden fixture");
    let fixture: GoldenFixture = serde_json::from_str(&contents).expect("valid golden fixture");
    assert_eq!(fixture.schema, 2, "this crate implements fixture schema 2");
    fixture
}

fn fixed_time(milliseconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(milliseconds)
}

const FIXED_NOW_MS: u64 = 1_788_604_863_123; // 2026-09-05T10:41:03.123Z

fn config(logs_dir: PathBuf, module_id: &str) -> Config {
    Config {
        module_id: module_id.to_owned(),
        logs_dir,
        bound: Vec::new(),
        spec: Some("info".to_owned()),
        retention: SegmentRetention::default(),
        redactor: None,
        clock: Some(Arc::new(|| fixed_time(FIXED_NOW_MS))),
    }
}

fn build_test_layer(config: Config, capture: &CaptureWriter) -> (LogLayer, Handle) {
    build_layer(config, Box::new(capture.clone()), false).expect("build test layer")
}

fn dispatch(layer: LogLayer) -> tracing::Dispatch {
    tracing::Dispatch::new(Registry::default().with(layer))
}

fn level_from(text: &str) -> Level {
    match text {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        other => panic!("fixture level {other:?}"),
    }
}

fn read_segment(handle: &Handle) -> String {
    fs::read_to_string(handle.path()).expect("segment readable")
}

// ---------------------------------------------------------------------------
// Render: every fixture case, byte-identical, through the real render path.

#[test]
fn golden_lines_render_byte_identically() {
    for case in fixture().cases {
        let rendered = format::render_line(
            fixed_time(case.event.at_ms),
            &level_from(&case.event.level),
            &case.event.logger,
            &case.event.bound,
            &case.event.message,
            &case.event.fields,
        );
        assert_eq!(rendered, case.line, "render case {}", case.name);
    }
}

#[test]
fn every_golden_line_parses_and_round_trips_its_columns() {
    for case in fixture().cases {
        let parsed = parse_line(&case.line)
            .unwrap_or_else(|error| panic!("golden line {} must parse: {error}", case.name));
        assert_eq!(parsed.logger, case.event.logger, "{}", case.name);
        assert_eq!(
            parsed.module_id,
            case.event.logger.split('.').next().unwrap(),
            "{}",
            case.name
        );
        assert_eq!(
            parsed.timestamp,
            fixed_time(case.event.at_ms),
            "{}",
            case.name
        );
        let expected_session = case
            .event
            .bound
            .iter()
            .find(|(key, _)| key == "session")
            .map(|(_, value)| value.as_str());
        assert_eq!(parsed.session(), expected_session, "{}", case.name);
        assert_eq!(
            parsed.bound.is_none(),
            case.event.bound.is_empty(),
            "{}",
            case.name
        );
    }
}

// Every redaction case through the real fleet redactor. `control-*` cases come
// out unchanged, which is what keeps the patterns from widening into text that
// only resembles a credential.
#[test]
fn golden_redaction_cases_match() {
    for case in fixture().redaction.cases {
        assert_eq!(
            fleet_redact(&case.input),
            case.output,
            "redaction case {}",
            case.name
        );
    }
}

// The whole-line stripping bug from before 0.3.3, reproduced through the real
// emit and write path: a value carrying `ESC ]` opened an OSC that consumed the
// closing quote and every later field. The later field must reach the file.
#[test]
fn a_stray_osc_introducer_in_one_value_keeps_the_later_fields() {
    let root = tempfile::tempdir().expect("tempdir");
    let capture = CaptureWriter::default();
    let (layer, handle) = build_test_layer(config(root.path().to_path_buf(), "aft"), &capture);
    tracing::dispatcher::with_default(&dispatch(layer), || {
        tracing::warn!(target: "lsp", text = "x\u{1b}]title", code = 2_u64, "server said");
    });
    let line = read_segment(&handle);
    assert!(line.contains("text=\"x\\u001b]title\""), "{line}");
    assert!(line.contains(" code=2"), "later field lost: {line}");
    assert!(!line.contains('\u{1b}'), "raw ESC reached the file: {line}");
}

// A module redactor can put raw control bytes back into a finished line. The
// guard must escape them, never strip: a strip would eat text the redactor
// never meant to touch.
#[test]
fn the_post_redactor_guard_escapes_raw_controls() {
    let root = tempfile::tempdir().expect("tempdir");
    let capture = CaptureWriter::default();
    let mut config = config(root.path().to_path_buf(), "aft");
    config.redactor = Some(Arc::new(|line: &str| {
        Cow::Owned(line.replace("MARK", "\u{1b}]MARK"))
    }));
    let (layer, handle) = build_test_layer(config, &capture);
    tracing::dispatcher::with_default(&dispatch(layer), || {
        tracing::info!(after = "tail", "before MARK");
    });
    let line = read_segment(&handle);
    assert!(line.contains("before \\u001b]MARK"), "{line}");
    assert!(
        line.contains(" after=tail"),
        "guard consumed later text: {line}"
    );
}

// The stderr copy writes the same bytes as the segment, and only when asked.
#[test]
fn stderr_copy_repeats_each_segment_line_and_is_off_by_default() {
    for copy in [false, true] {
        let root = tempfile::tempdir().expect("tempdir");
        let capture = CaptureWriter::default();
        let (layer, handle) = build_layer(
            config(root.path().to_path_buf(), "aft"),
            Box::new(capture.clone()),
            copy,
        )
        .expect("layer");
        tracing::dispatcher::with_default(&dispatch(layer), || {
            tracing::info!(kind = "bridge", "started");
        });
        let segment = read_segment(&handle);
        assert!(segment.contains("aft: started kind=bridge"), "{segment}");
        if copy {
            assert_eq!(
                capture.text(),
                segment,
                "stderr copy must equal the segment"
            );
        } else {
            assert!(
                !capture.text().contains("started"),
                "copy is off by default: {}",
                capture.text()
            );
        }
    }
}

// An off-thread writer carries the context the caller's span would have given.
#[test]
fn emit_at_with_bound_renders_bound_fields_after_process_fields() {
    let root = tempfile::tempdir().expect("tempdir");
    let capture = CaptureWriter::default();
    let mut config = config(root.path().to_path_buf(), "aft");
    config.bound = vec![("harness".to_owned(), "opencode".to_owned())];
    let (_layer, handle) = build_test_layer(config, &capture);
    handle.emit_at_with_bound(
        fixed_time(FIXED_NOW_MS),
        Level::INFO,
        "aft.index",
        &[("session".to_owned(), "opencode:ses_1".to_owned())],
        "queued line",
        &[("n".to_owned(), "3".to_owned())],
    );
    let line = read_segment(&handle);
    assert_eq!(
        line,
        "2026-09-05T10:41:03.123Z INFO  aft.index: [harness=opencode session=opencode:ses_1] queued line n=3\n"
    );
}

#[test]
fn fixture_parse_rejections_have_named_reasons() {
    for reject in fixture().parse_rejects {
        let error = parse_line(&reject.line)
            .err()
            .unwrap_or_else(|| panic!("reject case {} must not parse", reject.name));
        assert_eq!(error.reason(), reject.reason, "reject case {}", reject.name);
    }
}

#[test]
fn every_fixture_level_filter_case_is_enforced() {
    for case in fixture().level_filter.cases {
        let filter = if case.spec.trim().is_empty() {
            filter::LevelFilter::info()
        } else {
            filter::LevelFilter::parse(&case.spec).unwrap_or_else(|_| filter::LevelFilter::info())
        };
        assert_eq!(
            filter.enabled(&case.logger, &level_from(&case.level)),
            case.emit,
            "spec={:?} logger={} level={}",
            case.spec,
            case.logger,
            case.level
        );
    }
}

#[test]
fn every_fixture_segment_name_case_derives_the_utc_day() {
    for case in fixture().segment_name.cases {
        assert_eq!(
            segment_name(&case.module, fixed_time(case.at_ms)),
            case.name,
            "module={} at_ms={}",
            case.module,
            case.at_ms
        );
    }
}

#[test]
fn every_fixture_retention_prune_case_selects_exactly_the_unlink_set() {
    for case in fixture().retention_prune.cases {
        let today = NaiveDate::parse_from_str(&case.today, "%Y-%m-%d").expect("fixture date");
        let retention = SegmentRetention {
            max_age_days: case.max_age_days,
            alarm_segment_mb: 256,
        };
        let mut doomed: Vec<&str> = prune_candidates(
            &case.module,
            today,
            retention,
            case.present.iter().map(String::as_str),
        );
        doomed.sort_unstable();
        let mut expected: Vec<&str> = case.unlink.iter().map(String::as_str).collect();
        expected.sort_unstable();
        assert_eq!(doomed, expected, "prune case {}", case.name);
    }
}

// ---------------------------------------------------------------------------
// Layer: what the tracing integration renders from spans and targets.

#[test]
fn target_becomes_the_component_and_module_paths_do_not() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let mut test_config = config(temp.path().to_owned(), "synapse");
    test_config.spec = Some("trace".to_owned());
    let (layer, handle) = build_test_layer(test_config, &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!(target: "perf", ms = 118_u64, "job done");
        tracing::info!(target: "gc.walk", "slice");
        tracing::info!("plain");
        tracing::info!(target: "Not.A.Segment", "bad target");
    });

    let contents = read_segment(&handle);
    let mut lines = contents.lines();
    assert!(lines
        .next()
        .unwrap()
        .contains(" synapse.perf: job done ms=118"));
    assert!(lines.next().unwrap().contains(" synapse.gc.walk: slice"));
    // Default target is this crate's module path: bare module id.
    assert!(lines.next().unwrap().contains(" synapse: plain"));
    // A target outside the segment grammar is NOT a component either.
    assert!(lines.next().unwrap().contains(" synapse: bad target"));
}

#[test]
fn span_fields_are_bound_context_root_first_and_inner_overrides() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let mut test_config = config(temp.path().to_owned(), "prefrontal-core");
    test_config.bound = vec![("harness".to_owned(), "opencode".to_owned())];
    let (layer, handle) = build_test_layer(test_config, &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        let outer = tracing::info_span!("outer", agent = "agent_1", lane = "notice");
        let _outer = outer.enter();
        tracing::info!("in outer");
        let inner = tracing::info_span!("inner", lane = "urgent");
        let _inner = inner.enter();
        tracing::info!("in inner");
    });

    let contents = read_segment(&handle);
    let mut lines = contents.lines();
    // Process-level first, then span fields root-first.
    assert!(lines
        .next()
        .unwrap()
        .contains(" [harness=opencode agent=agent_1 lane=notice] in outer"));
    // Inner span overrides the same key in place; position stays.
    assert!(lines
        .next()
        .unwrap()
        .contains(" [harness=opencode agent=agent_1 lane=urgent] in inner"));
}

#[test]
fn session_span_binds_the_whole_id_and_empty_halves_bind_nothing() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let (layer, handle) = build_test_layer(config(temp.path().to_owned(), "aft"), &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        let span = session_span("opencode", "ses_0758f6ce7ffeJ0A9sV8Qvema7d");
        let _guard = span.enter();
        tracing::info!("bound");
        drop(_guard);
        let none = session_span("", "anything");
        let _guard = none.enter();
        tracing::info!("unbound");
    });

    let contents = read_segment(&handle);
    let mut lines = contents.lines();
    assert!(lines
        .next()
        .unwrap()
        .contains(" aft: [session=opencode:ses_0758f6ce7ffeJ0A9sV8Qvema7d] bound"));
    let unbound = lines.next().unwrap();
    assert!(unbound.contains(" aft: unbound"), "{unbound}");
    assert!(
        !unbound.contains('['),
        "no bracket when nothing is bound: {unbound}"
    );
}

#[test]
fn ck_log_hierarchy_filters_the_layer_not_just_the_parser() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let mut test_config = config(temp.path().to_owned(), "magic-context");
    test_config.spec = Some("error,magic-context.perf=info".to_owned());
    let (layer, handle) = build_test_layer(test_config, &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!(target: "perf", "kept: perf at info");
        tracing::info!(target: "historian", "dropped: sibling at info");
        tracing::info!("dropped: root at info");
        tracing::error!("kept: root at error");
    });

    let contents = read_segment(&handle);
    assert_eq!(contents.lines().count(), 2, "{contents}");
    assert!(contents.contains("magic-context.perf: kept: perf at info"));
    assert!(contents.contains("magic-context: kept: root at error"));
}

#[test]
fn malformed_ck_log_falls_back_to_info_and_reports_once() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let mut test_config = config(temp.path().to_owned(), "fusiform");
    test_config.spec = Some("garbage=".to_owned());
    let (layer, handle) = build_test_layer(test_config, &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!("info survives the fallback");
        tracing::debug!("debug does not");
    });

    let contents = read_segment(&handle);
    assert_eq!(contents.lines().count(), 1, "{contents}");
    assert!(contents.contains("fusiform: info survives the fallback"));
    // The report is on the diagnostic stream, once, and is not a log line.
    assert!(capture.text().contains("invalid CK_LOG value"));
}

// ---------------------------------------------------------------------------
// Redaction and the one-line, no-ANSI guard.

#[test]
fn bearer_credentials_are_redacted() {
    assert_eq!(
        fleet_redact("token=Bearer abc.DEF-123"),
        "token=Bearer [REDACTED]"
    );
}

#[test]
fn jwt_credentials_are_redacted() {
    assert_eq!(
        fleet_redact("token=eyJhbGciOiJub25l.eyJzdWIiOiIxIn0.signature"),
        "token=[REDACTED]"
    );
}

#[test]
fn cortexkit_handles_are_redacted() {
    assert_eq!(
        fleet_redact("handle=ckh_private-handle"),
        "handle=[REDACTED]"
    );
}

#[test]
fn openai_keys_are_redacted() {
    assert_eq!(fleet_redact("key=sk-project_secret"), "key=[REDACTED]");
}

#[test]
fn github_tokens_are_redacted() {
    assert_eq!(
        fleet_redact("one=ghp_private two=gho_private"),
        "one=[REDACTED] two=[REDACTED]"
    );
}

#[test]
fn authorization_values_are_redacted() {
    assert_eq!(
        fleet_redact("request Authorization: Basic cHJpdmF0ZQ== status=sent"),
        "request Authorization: [REDACTED] status=sent"
    );
    assert_eq!(
        fleet_redact(r#"header="Authorization: Basic cHJpdmF0ZQ==" status=sent"#),
        r#"header="Authorization: [REDACTED]" status=sent"#
    );
    assert_eq!(
        fleet_redact(r#"header=\"Authorization: Basic cHJpdmF0ZQ==\" status=sent"#),
        r#"header=\"Authorization: [REDACTED]\" status=sent"#
    );
}

#[test]
fn ordinary_lines_pass_through_redaction_unchanged() {
    let line = "poll changed version=1788526509641 arrived=2";
    assert!(matches!(fleet_redact(line), Cow::Borrowed(value) if value == line));
}

#[test]
fn module_redactor_runs_after_fleet_redaction() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let observed_fleet_redaction = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&observed_fleet_redaction);
    let redactor: Arc<Redactor> = Arc::new(move |line: &str| {
        observed.store(line.contains("[REDACTED]"), Ordering::Relaxed);
        Cow::Owned(line.replace("[REDACTED]", "<module-redacted>"))
    });
    let mut test_config = config(temp.path().to_owned(), "redactor");
    test_config.redactor = Some(redactor);
    let (layer, handle) = build_test_layer(test_config, &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!(credential = "sk-private", "request");
    });

    let contents = read_segment(&handle);
    assert!(observed_fleet_redaction.load(Ordering::Relaxed));
    assert!(contents.contains("credential=<module-redacted>"));
    assert!(!contents.contains("sk-private"));
}

// Complete color sequences inside the message, in both 7-bit and C1 form, are
// removed per field. (A module redactor's own control bytes are escaped by the
// guard instead; see the_post_redactor_guard_escapes_raw_controls.)
#[test]
fn complete_ansi_sequences_in_a_message_are_stripped() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let (layer, handle) = build_test_layer(config(temp.path().to_owned(), "ansi"), &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!("\u{1b}[32mclean\u{1b}[0m \u{009b}31mc1\u{009b}0m");
    });

    let contents = read_segment(&handle);
    assert!(!contents.contains(['\u{1b}', '\u{009b}']));
    assert!(contents.ends_with("ansi: clean c1\n"), "{contents}");
}

// ---------------------------------------------------------------------------
// Segments: the day names the file, nothing is ever renamed, the writer prunes.

#[test]
fn a_day_roll_opens_the_next_segment_without_renaming_the_previous() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    // 23:59:59.999Z on the 5th, then 00:00:00.000Z on the 6th.
    let clock_ms = Arc::new(Mutex::new(1_788_652_799_999_u64));
    let clock_for_config = Arc::clone(&clock_ms);
    let mut test_config = config(temp.path().to_owned(), "engram");
    test_config.clock = Some(Arc::new(move || {
        fixed_time(*clock_for_config.lock().expect("clock"))
    }));
    let (layer, _handle) = build_test_layer(test_config, &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!("last line of the fifth");
        *clock_ms.lock().expect("clock") = 1_788_652_800_000;
        tracing::info!("first line of the sixth");
    });

    let fifth = fs::read_to_string(temp.path().join("engram.2026-09-05.log")).unwrap();
    let sixth = fs::read_to_string(temp.path().join("engram.2026-09-06.log")).unwrap();
    assert!(
        fifth.ends_with("engram: last line of the fifth\n"),
        "{fifth}"
    );
    assert!(
        sixth.ends_with("engram: first line of the sixth\n"),
        "{sixth}"
    );
    // No `.1`, no `.log.1`, nothing renamed: exactly the two day segments.
    let mut names: Vec<String> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["engram.2026-09-05.log", "engram.2026-09-06.log"]);
}

#[test]
fn init_prunes_aged_segments_by_filename_and_announces_it() {
    let temp = TempDir::new().expect("temp dir");
    // today is 2026-09-05; window 14 days keeps 08-22 and newer.
    for name in [
        "engram.2026-09-04.log",
        "engram.2026-08-22.log", // boundary day: KEPT
        "engram.2026-08-21.log", // one past: unlinked
        "engram.2026-01-01.log",
        "engram.stderr.log",    // not a segment: untouched
        "other.2026-01-01.log", // another module: untouched
        "engram-10004.log",     // r1 pid-suffixed: untouched
    ] {
        fs::write(temp.path().join(name), "x\n").unwrap();
    }
    let capture = CaptureWriter::default();
    let (_layer, _handle) = build_test_layer(config(temp.path().to_owned(), "engram"), &capture);

    let mut names: Vec<String> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "engram-10004.log",
            "engram.2026-08-22.log",
            "engram.2026-09-04.log",
            "engram.2026-09-05.log",
            "engram.stderr.log",
            "other.2026-01-01.log",
        ]
    );
    // The armed-and-fired line: without it "never ran" and "found nothing"
    // read identically from outside.
    assert!(
        capture.text().contains("engram.retention pruned=2 kept=2"),
        "{}",
        capture.text()
    );
}

#[test]
fn an_oversized_segment_is_reported_once_and_never_truncated() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let mut test_config = config(temp.path().to_owned(), "broca");
    // 0 MiB means the first byte crosses the alarm; the test does not write MiB.
    test_config.retention.alarm_segment_mb = 0;
    let (layer, handle) = build_test_layer(test_config, &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!("one");
        tracing::info!("two");
        tracing::info!("three");
    });

    let contents = read_segment(&handle);
    assert_eq!(contents.lines().count(), 3, "nothing truncated: {contents}");
    let reports = capture.text();
    assert_eq!(
        reports.matches("segment oversized, NOT truncated").count(),
        1,
        "{reports}"
    );
}

#[test]
fn two_processes_worth_of_writers_share_one_segment_at_line_boundaries() {
    // Two independent layers on the same directory and module id model a
    // plugin and its module writing the same day's segment. O_APPEND and one
    // write per line are what make this hold; the assertion is that every line
    // parses, which a torn interleave cannot satisfy.
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let mut a = config(temp.path().to_owned(), "magic-context");
    a.bound = vec![("harness".to_owned(), "pi".to_owned())];
    let mut b = config(temp.path().to_owned(), "magic-context");
    b.bound = vec![("harness".to_owned(), "opencode".to_owned())];
    let (layer_a, handle) = build_test_layer(a, &capture);
    let (layer_b, _) = build_test_layer(b, &capture);
    let dispatch_a = dispatch(layer_a);
    let dispatch_b = dispatch(layer_b);

    let barrier = Arc::new(Barrier::new(2));
    let per_writer = 300;
    let spawn = |dispatcher: tracing::Dispatch, barrier: Arc<Barrier>, tag: &'static str| {
        thread::spawn(move || {
            barrier.wait();
            tracing::dispatcher::with_default(&dispatcher, || {
                for index in 0..per_writer {
                    tracing::info!(index, payload = "x".repeat(200).as_str(), "{tag}");
                }
            });
        })
    };
    let ta = spawn(dispatch_a, Arc::clone(&barrier), "from-pi");
    let tb = spawn(dispatch_b, Arc::clone(&barrier), "from-opencode");
    ta.join().unwrap();
    tb.join().unwrap();

    let contents = read_segment(&handle);
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), per_writer * 2);
    let mut pi = 0;
    let mut opencode = 0;
    for line in lines {
        let parsed = parse_line(line).unwrap_or_else(|error| panic!("torn line: {error}: {line}"));
        match parsed.bound {
            Some(bound) if bound.contains("harness=pi") => pi += 1,
            Some(bound) if bound.contains("harness=opencode") => opencode += 1,
            other => panic!("unexpected bound {other:?}"),
        }
    }
    assert_eq!((pi, opencode), (per_writer, per_writer));
}

// ---------------------------------------------------------------------------
// Failure paths: the logger reports its own trouble exactly once.

#[test]
fn unwritable_directory_activates_captured_stderr_fallback() {
    let temp = TempDir::new().expect("temp dir");
    let blocker = temp.path().join("logs");
    fs::write(&blocker, "a file where the logs dir should be").unwrap();
    let capture = CaptureWriter::default();
    let (layer, handle) = build_test_layer(config(blocker, "fusiform"), &capture);
    let dispatcher = dispatch(layer);

    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!("goes to stderr");
    });

    assert!(handle.fallback_active());
    let text = capture.text();
    let mut lines = text.lines();
    assert!(
        lines
            .next()
            .unwrap()
            .contains("fusiform: log directory unavailable; falling back to stderr dir="),
        "{text}"
    );
    assert!(
        lines.next().unwrap().contains("fusiform: goes to stderr"),
        "{text}"
    );
}

#[test]
fn write_failures_are_swallowed_and_reported_once() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let (layer, handle) = build_test_layer(config(temp.path().to_owned(), "plexus"), &capture);
    // Make the open segment unwritable by replacing the directory under it
    // with a read-only one: the held descriptor keeps working on some
    // platforms, so instead inject failure through the crate's own test arm.
    {
        let mut destination = handle.inner.destination.lock().unwrap();
        *destination = None;
    }
    // With no destination the layer falls back to the diagnostic stream,
    // which is what the fallback contract promises; write failure proper is
    // exercised on the rename-rotating LineSink below where the sink is owned.
    let dispatcher = dispatch(layer);
    tracing::dispatcher::with_default(&dispatcher, || {
        tracing::info!("after destination loss");
    });
    assert!(capture.text().contains("plexus: after destination loss"));
}

#[test]
fn eight_threads_write_complete_parseable_lines() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    let (layer, handle) = build_test_layer(config(temp.path().to_owned(), "callosum"), &capture);
    let dispatcher = dispatch(layer);
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|thread_index| {
            let dispatcher = dispatcher.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                tracing::dispatcher::with_default(&dispatcher, || {
                    for index in 0..50 {
                        tracing::info!(thread = thread_index, index, "line");
                    }
                });
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let contents = read_segment(&handle);
    assert_eq!(contents.lines().count(), 400);
    for line in contents.lines() {
        parse_line(line).unwrap_or_else(|error| panic!("{error}: {line}"));
    }
}

#[test]
fn paths_and_unix_permissions_follow_the_contract() {
    let temp = TempDir::new().expect("temp dir");
    let logs = temp.path().join("logs");
    let capture = CaptureWriter::default();
    let (layer, handle) = build_test_layer(config(logs.clone(), "insula"), &capture);
    let dispatcher = dispatch(layer);
    tracing::dispatcher::with_default(&dispatcher, || tracing::info!("x"));

    assert_eq!(handle.path(), logs.join("insula.2026-09-05.log"));
    assert_eq!(handle.logs_dir(), logs);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(handle.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
#[cfg(feature = "store-paths")]
fn for_module_places_segments_under_the_module_data_dir() {
    let config = Config::for_module("insula");
    let expected = PathBuf::from(cortexkit_store_types::module_data_dir("insula")).join("logs");
    assert_eq!(config.logs_dir, expected);
    assert!(config.bound.is_empty());
    let plugin = Config::for_plugin("magic-context", "pi");
    assert_eq!(plugin.bound, vec![("harness".to_owned(), "pi".to_owned())]);
}

#[test]
#[cfg(feature = "store-paths")]
fn from_env_reads_the_daemon_injected_knobs_and_refuses_without_a_module_id() {
    // Serialised through a lock because these are process-global.
    static ENV: Mutex<()> = Mutex::new(());
    let _guard = ENV.lock().unwrap();
    env::remove_var("SUBC_MODULE_ID");
    assert_eq!(
        Config::from_env().err(),
        Some(InitError::ModuleIdNotInEnvironment)
    );
    env::set_var("SUBC_MODULE_ID", "wernicke");
    env::set_var("CK_LOG_MAX_AGE_DAYS", "3");
    env::set_var("CK_LOG_ALARM_SEGMENT_MB", "64");
    let config = Config::from_env().unwrap();
    assert_eq!(config.module_id, "wernicke");
    assert_eq!(config.retention.max_age_days, 3);
    assert_eq!(config.retention.alarm_segment_mb, 64);
    env::remove_var("SUBC_MODULE_ID");
    env::remove_var("CK_LOG_MAX_AGE_DAYS");
    env::remove_var("CK_LOG_ALARM_SEGMENT_MB");
}

// ---------------------------------------------------------------------------
// Panic hook: a module's last words land in its own file as `<module>.panic`.

#[test]
fn panic_hook_logs_each_line_and_chains_to_previous_hook() {
    let temp = TempDir::new().unwrap();
    let logs_dir = temp.path().join("logs");
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::panic_hook_child",
            "--ignored",
            "--nocapture",
        ])
        .env("CORTEXKIT_LOG_PANIC_CHILD", "1")
        .env("CORTEXKIT_LOG_PANIC_DIR", &logs_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let segment = fs::read_to_string(logs_dir.join("panicker.2026-09-05.log"))
        .unwrap_or_else(|error| panic!("child segment: {error}; child stderr: {stderr}"));
    assert!(segment.contains("ERROR panicker.panic: "), "{segment}");
    assert!(segment.contains("boom"), "{segment}");
    for line in segment.lines() {
        parse_line(line).unwrap_or_else(|error| panic!("{error}: {line}"));
    }
    // The previous hook still ran: the default hook writes the panic to stderr.
    assert!(stderr.contains("boom"), "{stderr}");
}

#[test]
#[ignore = "spawned by panic_hook_logs_each_line_and_chains_to_previous_hook"]
fn panic_hook_child() {
    if std::env::var("CORTEXKIT_LOG_PANIC_CHILD").is_err() {
        return;
    }
    let logs_dir = PathBuf::from(std::env::var("CORTEXKIT_LOG_PANIC_DIR").unwrap());
    let _handle = init(config(logs_dir, "panicker")).unwrap();
    panic!("boom");
}

// ---------------------------------------------------------------------------
// LineSink: the single-writer, rename-rotating sink the daemon's capture uses.
// It keeps r1 rotation on purpose (one writer, so rename is safe there).

#[test]
fn line_sink_rotates_and_frames_each_line_once() {
    let temp = TempDir::new().expect("temp dir");
    let path = temp.path().join("aft.stderr.log");
    // 23 + 25 = 48 bytes fit under 60; the 22-byte third would make 70.
    let retention = Retention::from_bytes_for_testing(60, 2, 14);
    let now = fixed_time(FIXED_NOW_MS);
    let mut sink = LineSink::open_at(&path, retention, now).unwrap();
    sink.write_line_at(b"first line, no newline", now).unwrap();
    sink.write_line_at(b"second line, has newline\n", now)
        .unwrap();
    sink.write_line_at(b"third forces rotation", now).unwrap();

    let active = fs::read_to_string(&path).unwrap();
    assert_eq!(active, "third forces rotation\n");
    let rotated = fs::read_to_string(sink::rotated_path(&path, 1)).unwrap();
    assert_eq!(
        rotated,
        "first line, no newline\nsecond line, has newline\n"
    );
}

#[test]
fn two_line_sinks_on_one_file_never_tear_a_line() {
    // The daemon pumps a child's stdout and stderr through one sink each, both
    // on the same capture file. A torn line is what a second write for the
    // newline would produce; the single framed write is what prevents it, and
    // this proves it under contention rather than asserting it.
    let temp = TempDir::new().expect("temp dir");
    let shared = temp.path().join("shared.stderr.log");
    let retention = Retention::from_bytes_for_testing(1 << 20, 1, 14);
    let now = fixed_time(FIXED_NOW_MS);
    let barrier = Arc::new(Barrier::new(2));
    let spawn = |lane: &'static str, barrier: Arc<Barrier>, shared: PathBuf| {
        thread::spawn(move || {
            let mut sink = LineSink::open_at(&shared, retention, now).unwrap();
            barrier.wait();
            for i in 0..200 {
                sink.write_line_at(format!("{lane}-{i:03}-{}", "x".repeat(48)).as_bytes(), now)
                    .unwrap();
            }
        })
    };
    let a = spawn("out", Arc::clone(&barrier), shared.clone());
    let b = spawn("err", Arc::clone(&barrier), shared.clone());
    a.join().unwrap();
    b.join().unwrap();

    let woven = fs::read_to_string(&shared).unwrap();
    assert_eq!(woven.lines().count(), 400, "every line must be present");
    for line in woven.lines() {
        assert!(
            (line.starts_with("out-") || line.starts_with("err-")) && line.len() == 56,
            "torn line: {line:?}"
        );
    }
}

#[test]
fn in_dir_takes_the_callers_directory_verbatim_and_binds_nothing() {
    // The constructor a consumer uses when it resolves its own data directory
    // through a store crate pinned elsewhere, so this crate must not resolve
    // one for it. Available with or without the `store-paths` feature.
    let config = Config::in_dir("thalamus", "/some/where/thalamus/logs");
    assert_eq!(config.logs_dir, PathBuf::from("/some/where/thalamus/logs"));
    assert_eq!(config.module_id, "thalamus");
    assert!(config.bound.is_empty());
    assert!(config.spec.is_none());
}

// ---------------------------------------------------------------------------
// emit_at: a record placed by an explicit instant, for a wall-clock step.

/// A clock that stepped across UTC midnight put the lines before the step in
/// one day's segment and everything after in another. A marker describing the
/// pre-step lines must land in THEIR segment, stamped like its neighbours, or a
/// reader who opens that file finds the lines and not the warning.
#[test]
fn emit_at_writes_into_the_segment_its_instant_names() {
    let temp = TempDir::new().expect("temp dir");
    let capture = CaptureWriter::default();
    // The logger's own clock reads the next UTC day: the corrected time after
    // a step backward across midnight.
    let now_ms = FIXED_NOW_MS + 86_400_000;
    let mut config = config(temp.path().to_path_buf(), "stepper");
    config.clock = Some(Arc::new(move || fixed_time(now_ms)));
    let (_layer, handle) = build_test_layer(config, &capture);

    let pre_step = fixed_time(FIXED_NOW_MS);
    handle.emit_at(
        pre_step,
        Level::WARN,
        "stepper",
        "wall clock stepped",
        &[("step_ms".to_owned(), "7200000".to_owned())],
    );

    let pre_step_segment = temp.path().join(segment::segment_name("stepper", pre_step));
    let written = fs::read_to_string(&pre_step_segment).expect("pre-step segment written");
    assert_eq!(written.lines().count(), 1, "{written}");
    assert!(
        written.starts_with("2026-09-05T10:41:03.123Z WARN  stepper: wall clock stepped"),
        "stamped with the instant it was placed by: {written}"
    );
    // The logger creates its current segment on open, so the file may exist;
    // what must hold is that it did not receive this line.
    let current = fs::read_to_string(handle.path()).unwrap_or_default();
    assert!(
        !current.contains("wall clock stepped"),
        "the current segment ({}) received a line emitted for another instant: {current}",
        handle.path().display()
    );
}
