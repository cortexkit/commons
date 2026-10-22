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

use std::collections::HashMap;
use std::fmt;

use tracing::Level;

use crate::format::LoggerPath;

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
    #[cfg(test)]
    pub(crate) fn enabled(&self, logger: &str, level: &Level) -> bool {
        Self::allows(self.threshold(logger), level)
    }

    pub(crate) fn enabled_path(
        &self,
        module_id: &str,
        path: LoggerPath<'_>,
        level: &Level,
    ) -> bool {
        Self::allows(self.threshold_for_path(module_id, path), level)
    }

    /// Applies the dependency WARN default unless CK_LOG names this dependency.
    pub(crate) fn dependency_enabled_path(
        &self,
        module_id: &str,
        path: LoggerPath<'_>,
        level: &Level,
        ceiling: Option<Option<Level>>,
    ) -> bool {
        let has_dependency_directive = self.directives.iter().any(|(prefix, _)| {
            is_dependency_directive(prefix, module_id) && path.matches_prefix(module_id, prefix)
        });
        let mut threshold = if has_dependency_directive {
            self.threshold_for_path(module_id, path)
        } else {
            self.threshold(module_id)
                .map(|own_level| own_level.min(Level::WARN))
        };

        // A configured ceiling is applied after the CK_LOG directives. `None`
        // inside the outer option is an explicit OFF ceiling.
        if let Some(ceiling) = ceiling {
            threshold = threshold.zip(ceiling).map(|(level, cap)| level.min(cap));
        }

        Self::allows(threshold, level)
    }

    pub(crate) fn max_level_hint(
        &self,
        module_id: &str,
        own_crates_possible: bool,
        ceilings: Option<&HashMap<String, Option<Level>>>,
    ) -> Option<Level> {
        if own_crates_possible {
            // A dependency-looking prefix can also be an own-crate component
            // target, where dependency ceilings do not apply.
            let mut maximum = self.root;
            for (index, (prefix, level)) in self.directives.iter().enumerate() {
                if is_last_directive(&self.directives, index, prefix) {
                    maximum = max_level(maximum, *level);
                }
            }
            return maximum;
        }

        let mut dependency_root_set = false;
        let mut dependency_root = None;
        for (prefix, level) in &self.directives {
            if dependency_directive_crate(prefix, module_id) == Some(None) {
                dependency_root_set = true;
                dependency_root = *level;
            }
        }

        let mut maximum = if dependency_root_set {
            dependency_root
        } else {
            self.threshold(module_id)
                .map(|level| level.min(Level::WARN))
        };
        for (index, (prefix, level)) in self.directives.iter().enumerate() {
            let Some(Some(crate_segment)) = dependency_directive_crate(prefix, module_id) else {
                continue;
            };
            if !is_last_directive(&self.directives, index, prefix) {
                continue;
            }
            let crate_name = crate_segment.replace('-', "_");
            let ceiling = ceilings
                .and_then(|ceilings| ceilings.get(&crate_name))
                .copied();
            let effective = match ceiling {
                Some(None) => None,
                Some(Some(ceiling)) => level.map(|level| level.min(ceiling)),
                None => *level,
            };
            maximum = max_level(maximum, effective);
        }
        maximum
    }

    fn threshold(&self, logger: &str) -> Option<Level> {
        let mut threshold = self.root;
        for (prefix, directive_level) in &self.directives {
            if is_prefix(prefix, logger) {
                threshold = *directive_level;
            }
        }
        threshold
    }

    fn threshold_for_path(&self, module_id: &str, path: LoggerPath<'_>) -> Option<Level> {
        let mut threshold = self.root;
        for (prefix, directive_level) in &self.directives {
            if path.matches_prefix(module_id, prefix) {
                threshold = *directive_level;
            }
        }
        threshold
    }

    fn allows(threshold: Option<Level>, level: &Level) -> bool {
        threshold.is_some_and(|threshold| *level <= threshold)
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

fn is_dependency_directive(prefix: &str, module_id: &str) -> bool {
    let mut directive_segments = prefix.split('.');
    for module_segment in module_id.split('.') {
        if directive_segments.next() != Some(module_segment) {
            return false;
        }
    }
    directive_segments.next() == Some("dep")
}

fn dependency_directive_crate<'a>(prefix: &'a str, module_id: &str) -> Option<Option<&'a str>> {
    let mut segments = prefix.split('.');
    for module_segment in module_id.split('.') {
        if segments.next() != Some(module_segment) {
            return None;
        }
    }
    if segments.next() != Some("dep") {
        return None;
    }
    match (segments.next(), segments.next()) {
        (None, None) => Some(None),
        (Some(crate_name), None) => Some(Some(crate_name)),
        _ => None,
    }
}

fn max_level(current: Option<Level>, candidate: Option<Level>) -> Option<Level> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.max(candidate)),
        (Some(level), None) | (None, Some(level)) => Some(level),
        (None, None) => None,
    }
}

fn is_last_directive(directives: &[(String, Option<Level>)], index: usize, prefix: &str) -> bool {
    !directives[index + 1..]
        .iter()
        .any(|(later_prefix, _)| later_prefix == prefix)
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
