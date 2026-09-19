//! A synchronous `tracing` layer for CortexKit's module-owned fleet logs.
//!
//! One crate, one format, one file per module per day. See
//! `subconscious/docs/specs/fleet-logging.md` (r2) for the contract every line
//! here implements; the golden fixture beside it is what the tests pin to.

mod filter;
mod format;
mod redaction;
mod segment;
mod sink;

pub use segment::{prune_candidates, segment_day, segment_name, SegmentRetention};
pub use sink::LineSink;

use std::backtrace::Backtrace;
use std::borrow::Cow;
use std::env;
use std::fmt;
use std::io::{self, Write};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use filter::LevelFilter;
pub use format::{ParseError, ParsedLevel, ParsedLine};
use redaction::fleet_redact;
use segment::{SegmentDestination, SegmentNotice};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

static WRITE_FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);
static FILTER_FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);

/// Size and age bounds for the rename-rotated [`LineSink`], which has exactly
/// one writer (the daemon's per-child stderr capture). The logger's own
/// segments use [`SegmentRetention`]; the two are different files with
/// different writer counts, and only the single-writer one may rename.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Retention {
    /// Maximum active-file size in mebibytes.
    pub max_file_mb: u32,
    /// Number of rotated generations to retain.
    pub keep: u8,
    /// Maximum age of rotated generations in days.
    pub max_age_days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            max_file_mb: 32,
            keep: 2,
            max_age_days: 14,
        }
    }
}

impl Retention {
    const TEST_BYTES_MARKER: u32 = 1 << 31;

    /// Constructs a byte-sized cap so retention tests do not write whole MiB files.
    #[doc(hidden)]
    pub fn from_bytes_for_testing(max_file_bytes: u32, keep: u8, max_age_days: u32) -> Self {
        assert!(max_file_bytes < Self::TEST_BYTES_MARKER);
        Self {
            max_file_mb: Self::TEST_BYTES_MARKER | max_file_bytes,
            keep,
            max_age_days,
        }
    }

    pub(crate) fn max_bytes(self) -> u64 {
        if self.max_file_mb & Self::TEST_BYTES_MARKER != 0 {
            return u64::from(self.max_file_mb & !Self::TEST_BYTES_MARKER);
        }

        u64::from(self.max_file_mb) * 1024 * 1024
    }
}

/// A complete-line redactor composed after the fleet credential redactor.
pub type Redactor = dyn for<'line> Fn(&'line str) -> Cow<'line, str> + Send + Sync;

/// Logger configuration for one process.
pub struct Config {
    /// Stable fleet module identifier: the root of every logger name and the
    /// first part of every segment's file name.
    pub module_id: String,
    /// The directory the segments live in. Fleet callers never assemble this:
    /// [`Config::for_module`] derives it from the module id through the same
    /// resolver every store uses, so a log cannot land beside the wrong data.
    /// Public for tests and for the daemon's own `run/logs/` lane, which is not
    /// a module data directory.
    pub logs_dir: PathBuf,
    /// Fields bound for the whole process and rendered in the bracket on every
    /// line, in this order. A harness-hosted plugin binds `harness=<name>`
    /// here: after r2 every lane shares one segment and this is what tells
    /// them apart.
    pub bound: Vec<(String, String)>,
    /// `CK_LOG` override; `None` reads the process environment.
    pub spec: Option<String>,
    /// Age window and oversize alarm for the segments.
    pub retention: SegmentRetention,
    /// Optional module redactor, applied after fleet credential redaction.
    pub redactor: Option<Arc<Redactor>>,
    /// Optional clock override for deterministic callers and tests.
    pub clock: Option<Arc<dyn Fn() -> SystemTime + Send + Sync>>,
}

impl Config {
    /// The fleet configuration for a supervised module's own process: segments
    /// under `<module data dir>/logs/`, `CK_LOG` from the environment (the
    /// daemon injects it at spawn), fleet default retention, nothing bound.
    pub fn for_module(module_id: &str) -> Self {
        let data_dir = PathBuf::from(cortexkit_store_types::module_data_dir(module_id));
        Self {
            module_id: module_id.to_owned(),
            logs_dir: data_dir.join("logs"),
            bound: Vec::new(),
            spec: None,
            retention: SegmentRetention::default(),
            redactor: None,
            clock: None,
        }
    }

    /// [`Config::for_module`] with `harness=<harness>` bound on every line, for
    /// a plugin running inside a harness process.
    pub fn for_plugin(module_id: &str, harness: &str) -> Self {
        let mut config = Self::for_module(module_id);
        config
            .bound
            .push(("harness".to_owned(), harness.to_owned()));
        config
    }

    /// The configuration a supervised module can build with no arguments,
    /// because the daemon already injects everything it needs at spawn:
    /// `SUBC_MODULE_ID` (for launch attestation), `CK_LOG`, and the retention
    /// knobs as `CK_LOG_MAX_AGE_DAYS` / `CK_LOG_ALARM_SEGMENT_MB`. This is the
    /// zero-argument path the r2 spec requires so that reaching for the crate
    /// costs no more than reaching for `eprintln!`.
    pub fn from_env() -> Result<Self, InitError> {
        let module_id = env::var("SUBC_MODULE_ID")
            .ok()
            .filter(|value| !value.is_empty())
            .ok_or(InitError::ModuleIdNotInEnvironment)?;
        let mut config = Self::for_module(&module_id);
        if let Some(days) = env_u32("CK_LOG_MAX_AGE_DAYS") {
            config.retention.max_age_days = days;
        }
        if let Some(mb) = env_u32("CK_LOG_ALARM_SEGMENT_MB") {
            config.retention.alarm_segment_mb = mb;
        }
        Ok(config)
    }
}

fn env_u32(name: &str) -> Option<u32> {
    env::var(name).ok()?.trim().parse().ok()
}

/// A live logger handle suitable for inclusion in module health reports.
#[derive(Clone)]
pub struct Handle {
    inner: Arc<LoggerInner>,
}

impl Handle {
    /// Returns the number of lines dropped after a write failure.
    pub fn swallowed_writes(&self) -> u64 {
        self.inner.swallowed_writes.load(Ordering::Relaxed)
    }

    /// Returns the segment path for the current instant.
    pub fn path(&self) -> PathBuf {
        self.inner.path_now()
    }

    /// The directory the segments live in.
    pub fn logs_dir(&self) -> &Path {
        &self.inner.logs_dir
    }

    /// Reports whether opening the directory failed and events are going to stderr.
    pub fn fallback_active(&self) -> bool {
        self.inner.fallback_active
    }
}

/// An error that prevents installation of the global logger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitError {
    /// [`Config::from_env`] found no `SUBC_MODULE_ID`; the process is not a
    /// daemon-spawned module and must name itself.
    ModuleIdNotInEnvironment,
    /// A process-global tracing subscriber was already installed.
    GlobalSubscriberAlreadySet,
}

impl fmt::Display for InitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleIdNotInEnvironment => formatter.write_str(
                "SUBC_MODULE_ID is not set; use Config::for_module(<id>) outside supervision",
            ),
            Self::GlobalSubscriberAlreadySet => {
                formatter.write_str("a global tracing subscriber is already installed")
            }
        }
    }
}

impl std::error::Error for InitError {}

/// Installs the fleet logger as the process-global `tracing` subscriber.
pub fn init(config: Config) -> Result<Handle, InitError> {
    let (layer, handle) = build_layer(config, Box::new(io::stderr()))?;
    let panic_inner = Arc::clone(&handle.inner);
    let subscriber = Registry::default().with(layer);
    tracing::subscriber::set_global_default(subscriber)
        .map_err(|_| InitError::GlobalSubscriberAlreadySet)?;
    install_panic_hook(panic_inner);
    Ok(handle)
}

/// [`init`] with [`Config::from_env`]: the whole setup for a supervised module.
pub fn init_from_env() -> Result<Handle, InitError> {
    init(Config::from_env()?)
}

/// Creates a span carrying canonical session lineage for nested events. Every
/// event inside it renders `session=<issuer>:<id>` in the bound bracket.
pub fn session_span(issuer: &str, id: &str) -> tracing::Span {
    // An empty half means "no session": the line carries no field rather
    // than a placeholder. Callers that used to log a synthetic id must pass
    // nothing here instead; the crate does not know their sentinels.
    if issuer.is_empty() || id.is_empty() {
        tracing::Span::none()
    } else {
        let session = format!("{issuer}:{id}");
        tracing::info_span!("cortexkit.session", session = %session)
    }
}

/// Parses the fixed columns used by merged and filtered fleet log views.
pub fn parse_line(line: &str) -> Result<ParsedLine<'_>, ParseError> {
    format::parse(line)
}

struct LoggerInner {
    module_id: String,
    logs_dir: PathBuf,
    process_bound: Vec<(String, String)>,
    destination: Mutex<Option<SegmentDestination>>,
    stderr: Mutex<Box<dyn Write + Send>>,
    swallowed_writes: AtomicU64,
    fallback_active: bool,
    redactor: Option<Arc<Redactor>>,
    clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
}

impl LoggerInner {
    fn path_now(&self) -> PathBuf {
        self.logs_dir
            .join(segment::segment_name(&self.module_id, (self.clock)()))
    }

    fn emit(
        &self,
        level: &tracing::Level,
        logger: &str,
        scoped_bound: &[(String, String)],
        message: &str,
        fields: &[(String, String)],
    ) {
        self.emit_at((self.clock)(), level, logger, scoped_bound, message, fields);
    }

    fn emit_at(
        &self,
        at: SystemTime,
        level: &tracing::Level,
        logger: &str,
        scoped_bound: &[(String, String)],
        message: &str,
        fields: &[(String, String)],
    ) {
        // Process-level context first, then scoped: the process-level part is
        // identical on every line the process writes, so putting it first keeps
        // the bracket's leading columns stable within a file.
        let mut bound: Vec<(String, String)> = self.process_bound.clone();
        for (key, value) in scoped_bound {
            match bound.iter_mut().find(|(existing, _)| existing == key) {
                Some(slot) => slot.1 = value.clone(),
                None => bound.push((key.clone(), value.clone())),
            }
        }
        let raw = format::render_line(at, level, logger, &bound, message, fields);
        let fleet_redacted = fleet_redact(&raw);
        let module_redacted = self.redactor.as_ref().map_or_else(
            || Cow::Borrowed(fleet_redacted.as_ref()),
            |redactor| redactor(&fleet_redacted),
        );
        // Redactors are extensibility points, so the final guard preserves the
        // one-line, no-ANSI contract even if a module redactor introduces such
        // bytes.
        let guarded = format::strip_ansi(&module_redacted)
            .replace('\r', "\\r")
            .replace('\n', "\\n");
        self.write_line(&guarded, at);
    }

    fn write_line(&self, line: &str, at: SystemTime) {
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');

        // The mutex keeps this process's lines whole against its own threads;
        // O_APPEND keeps them whole against other processes. No queue, no
        // background flusher: a caller waits only for the write in front of it.
        let result = {
            let mut destination = self
                .destination
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match destination.as_mut() {
                None => {
                    let mut stderr = self
                        .stderr
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    stderr.write_all(&bytes).map(|()| None)
                }
                Some(segment) => segment.write(&bytes, at),
            }
        };

        match result {
            Ok(None) => {}
            Ok(Some(notice)) => self.report_notice(notice),
            Err(error) => {
                self.swallowed_writes.fetch_add(1, Ordering::Relaxed);
                if !WRITE_FAILURE_REPORTED.swap(true, Ordering::Relaxed) {
                    self.report(&format!(
                        "cortexkit-log: log write failed; future failures will be swallowed: {error}\n"
                    ));
                }
            }
        }
    }

    // A feature that fires later must say that it fired: without these lines,
    // "retention never ran" and "ran and found nothing to prune" are the same
    // observation from outside. Oversize is reported once per process because
    // a runaway writer must not get a second flood from its own alarm.
    fn report_notice(&self, notice: SegmentNotice) {
        match notice {
            SegmentNotice::Pruned { removed, kept } => self.report(&format!(
                "cortexkit-log: {}.retention pruned={removed} kept={kept}\n",
                self.module_id
            )),
            SegmentNotice::Oversized { path, bytes } => self.report(&format!(
                "cortexkit-log: segment oversized, NOT truncated: path={} bytes={bytes}\n",
                path.display()
            )),
        }
    }

    fn report(&self, report: &str) {
        let mut stderr = self
            .stderr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = stderr.write_all(report.as_bytes());
    }

    fn write_panic(&self, information: &panic::PanicHookInfo<'_>) {
        let at = (self.clock)();
        let logger = format!("{}.panic", self.module_id);
        let panic_text = information.to_string();
        for line in panic_text.lines() {
            self.emit_at(at, &tracing::Level::ERROR, &logger, &[], line, &[]);
        }
        let backtrace = Backtrace::force_capture().to_string();
        for line in backtrace.lines() {
            self.emit_at(at, &tracing::Level::ERROR, &logger, &[], line, &[]);
        }
    }
}

struct LogLayer {
    inner: Arc<LoggerInner>,
    filter: LevelFilter,
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn enabled(&self, metadata: &Metadata<'_>, _context: Context<'_, S>) -> bool {
        if metadata.is_span() {
            return true;
        }
        let logger = format::logger_name(&self.inner.module_id, metadata.target());
        self.filter.enabled(&logger, metadata.level())
    }

    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        let mut visitor = BoundVisitor::default();
        attributes.record(&mut visitor);
        if let (false, Some(span)) = (visitor.bound.is_empty(), context.span(id)) {
            span.extensions_mut().insert(SpanBound(visitor.bound));
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, context: Context<'_, S>) {
        let mut visitor = BoundVisitor::default();
        values.record(&mut visitor);
        if let (false, Some(span)) = (visitor.bound.is_empty(), context.span(id)) {
            let mut extensions = span.extensions_mut();
            match extensions.get_mut::<SpanBound>() {
                Some(existing) => existing.0.extend(visitor.bound),
                None => extensions.insert(SpanBound(visitor.bound)),
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        // Every field on every span enclosing the event is bound context,
        // root-first so an inner span overrides an outer one with the same key.
        // This is MDC: the module decides what is in scope by what it puts on
        // its spans, and the crate renders whatever is there.
        let mut scoped: Vec<(String, String)> = Vec::new();
        if let Some(scope) = context.event_scope(event) {
            for span in scope.from_root() {
                if let Some(bound) = span.extensions().get::<SpanBound>() {
                    for (key, value) in &bound.0 {
                        match scoped.iter_mut().find(|(existing, _)| existing == key) {
                            Some(slot) => slot.1 = value.clone(),
                            None => scoped.push((key.clone(), value.clone())),
                        }
                    }
                }
            }
        }
        let logger = format::logger_name(&self.inner.module_id, event.metadata().target());
        self.inner.emit(
            event.metadata().level(),
            &logger,
            &scoped,
            visitor.message.as_deref().unwrap_or(""),
            &visitor.fields,
        );
    }
}

#[derive(Clone)]
struct SpanBound(Vec<(String, String)>);

#[derive(Default)]
struct BoundVisitor {
    bound: Vec<(String, String)>,
}

impl BoundVisitor {
    fn record(&mut self, field: &Field, value: String) {
        // An empty value binds nothing: `session_span("", "")` and a plugin
        // with no harness must produce a line with no bracket, never `key=`.
        if !value.is_empty() {
            self.bound.push((field.name().to_owned(), value));
        }
    }
}

impl Visit for BoundVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        let value = rendered
            .strip_prefix('"')
            .and_then(|unquoted| unquoted.strip_suffix('"'))
            .unwrap_or(&rendered);
        self.record(field, value.to_owned());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, value.to_string());
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl EventVisitor {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push((field.name().to_owned(), value));
        }
    }
}

impl Visit for EventVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record(field, value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, value.to_string());
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.record(field, value.to_string());
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.record(field, value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, value.to_string());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.record(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

fn build_layer(
    config: Config,
    stderr: Box<dyn Write + Send>,
) -> Result<(LogLayer, Handle), InitError> {
    let clock = config.clock.unwrap_or_else(|| Arc::new(SystemTime::now));
    let now = clock();
    let (destination, open_notice, open_error) = match SegmentDestination::open(
        &config.logs_dir,
        &config.module_id,
        config.retention,
        now,
        true,
    ) {
        Ok((destination, notice)) => (Some(destination), notice, None),
        Err(error) => (None, None, Some(error)),
    };
    let fallback_active = open_error.is_some();
    let inner = Arc::new(LoggerInner {
        module_id: config.module_id,
        logs_dir: config.logs_dir,
        process_bound: config.bound,
        destination: Mutex::new(destination),
        stderr: Mutex::new(stderr),
        swallowed_writes: AtomicU64::new(0),
        fallback_active,
        redactor: config.redactor,
        clock,
    });

    if let Some(notice) = open_notice {
        inner.report_notice(notice);
    }
    if let Some(error) = open_error {
        let logger = inner.module_id.clone();
        inner.emit(
            &tracing::Level::ERROR,
            &logger,
            &[],
            "log directory unavailable; falling back to stderr",
            &[
                ("dir".to_owned(), inner.logs_dir.display().to_string()),
                ("error".to_owned(), error.to_string()),
            ],
        );
    }

    let filter = make_filter(config.spec, &inner);
    let layer = LogLayer {
        inner: Arc::clone(&inner),
        filter,
    };
    Ok((layer, Handle { inner }))
}

fn make_filter(spec_override: Option<String>, inner: &LoggerInner) -> LevelFilter {
    let spec = spec_override.or_else(|| env::var("CK_LOG").ok());
    let spec = spec
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match spec {
        Some(spec) => LevelFilter::parse(spec).unwrap_or_else(|error| {
            if !FILTER_FAILURE_REPORTED.swap(true, Ordering::Relaxed) {
                inner.report(&format!(
                    "cortexkit-log: invalid CK_LOG value {spec:?}; using info: {error}\n"
                ));
            }
            LevelFilter::info()
        }),
        None => LevelFilter::info(),
    }
}

fn install_panic_hook(inner: Arc<LoggerInner>) {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |information| {
        inner.write_panic(information);
        previous(information);
    }));
}

#[cfg(test)]
mod tests;
