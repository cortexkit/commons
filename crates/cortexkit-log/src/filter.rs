//! `CK_LOG` level filter over hierarchical logger names.
//!
//! Grammar is `RUST_LOG`'s: comma-separated directives, each `<logger>=<level>`
//! or a bare `<level>` that sets the root default. A directive names a dotted
//! logger PREFIX and applies to it and every logger beneath it; the most
//! specific matching directive decides. This is Logback's `<logger name=...>`
//! and Python's `getLogger("a.b").setLevel(...)` — never a separate "tag"
//! dimension.
//!
//! `tracing_subscriber::EnvFilter` is not used because it matches targets on
//! `::` boundaries and our hierarchy is `.`-separated: `engram.gc=trace` must
//! cover `engram.gc.walk`, and `aft=debug` must NOT cover `aftershock`.

use std::fmt;

use tracing::Level;

/// A parsed `CK_LOG` value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LevelFilter {
    /// The level applied when no directive matches. `None` means off.
    root: Option<Level>,
    /// `(dotted logger prefix, level)`, most specific last so a linear scan
    /// that keeps the last hit is "most specific wins".
    directives: Vec<(String, Option<Level>)>,
}

/// Why a `CK_LOG` value could not be parsed. Carried in the once-only stderr
/// report; the filter itself falls back to `info`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FilterParseError {
    directive: String,
    reason: &'static str,
}

impl fmt::Display for FilterParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {:?}", self.reason, self.directive)
    }
}

impl LevelFilter {
    /// The fleet default: `info` at the root, nothing else.
    pub(crate) fn info() -> Self {
        Self {
            root: Some(Level::INFO),
            directives: Vec::new(),
        }
    }

    pub(crate) fn parse(spec: &str) -> Result<Self, FilterParseError> {
        let mut root = Some(Level::INFO);
        let mut directives = Vec::new();
        for raw in spec.split(',') {
            let directive = raw.trim();
            if directive.is_empty() {
                continue;
            }
            match directive.split_once('=') {
                None => {
                    root = parse_level(directive).ok_or(FilterParseError {
                        directive: directive.to_owned(),
                        reason: "unknown level",
                    })?;
                }
                Some((logger, level)) => {
                    let logger = logger.trim();
                    let level = level.trim();
                    if logger.is_empty() || !logger.split('.').all(crate::format::is_segment) {
                        return Err(FilterParseError {
                            directive: directive.to_owned(),
                            reason: "logger name is not <segment>(.<segment>)*",
                        });
                    }
                    if level.is_empty() {
                        return Err(FilterParseError {
                            directive: directive.to_owned(),
                            reason: "a directive has no level after '='",
                        });
                    }
                    let level = parse_level(level).ok_or(FilterParseError {
                        directive: directive.to_owned(),
                        reason: "unknown level",
                    })?;
                    directives.push((logger.to_owned(), level));
                }
            }
        }
        // Sort by segment count so the scan's last hit is the deepest prefix.
        directives.sort_by_key(|(logger, _)| logger.split('.').count());
        Ok(Self { root, directives })
    }

    /// Whether an event at `level` on `logger` is emitted.
    pub(crate) fn enabled(&self, logger: &str, level: &Level) -> bool {
        let mut threshold = self.root;
        for (prefix, directive_level) in &self.directives {
            if is_prefix(prefix, logger) {
                threshold = *directive_level;
            }
        }
        match threshold {
            None => false,
            Some(threshold) => *level <= threshold,
        }
    }
}

// Prefix on DOTTED SEGMENTS, not characters: `aft` covers `aft` and
// `aft.index`, and does not cover `aftershock`.
fn is_prefix(prefix: &str, logger: &str) -> bool {
    match logger.strip_prefix(prefix) {
        Some("") => true,
        Some(rest) => rest.starts_with('.'),
        None => false,
    }
}

// `Some(None)` is a parsed "off"; `None` is not a level at all.
fn parse_level(text: &str) -> Option<Option<Level>> {
    match text.to_ascii_lowercase().as_str() {
        "off" => Some(None),
        "error" => Some(Some(Level::ERROR)),
        "warn" => Some(Some(Level::WARN)),
        "info" => Some(Some(Level::INFO)),
        "debug" => Some(Some(Level::DEBUG)),
        "trace" => Some(Some(Level::TRACE)),
        _ => None,
    }
}
