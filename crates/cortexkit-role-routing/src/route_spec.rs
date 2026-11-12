//! Shared provider/model exclusion specs. These are not pin expressions:
//! colons are literal model characters and there are no globs or ranges.

/// Validate a producer's spec. Consumers trim and drop malformed entries;
/// producers must never emit an empty entry or an empty slash segment.
pub fn is_route_spec(value: &str) -> bool {
    let value = value.trim().strip_prefix('-').unwrap_or(value.trim());
    !value.is_empty() && value.split('/').all(|segment| !segment.is_empty())
}

/// Does a valid spec cover this route key? Matching respects slash boundaries.
pub fn matches(spec: &str, key: &str) -> bool {
    if !is_route_spec(spec) {
        return false;
    }
    let spec = spec.trim().strip_prefix('-').unwrap_or(spec.trim());
    key == spec
        || key
            .strip_prefix(spec)
            .is_some_and(|tail| tail.starts_with('/'))
}

/// Resolve a list against a base value. Model-level matches beat provider
/// matches regardless of order; the last match at the winning level wins.
/// A positive entry uses the base, and a negative entry flips that base.
pub fn resolve<'a>(
    specs: impl IntoIterator<Item = &'a str>,
    key: &str,
    base: bool,
) -> Option<bool> {
    let mut provider = None;
    let mut model = None;
    for spec in specs {
        if matches(spec, key) {
            let spec = spec.trim();
            let negative = spec.starts_with('-');
            let value = if negative { !base } else { base };
            if spec.contains('/') {
                model = Some(value);
            } else {
                provider = Some(value);
            }
        }
    }
    model.or(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_spec_grammar_and_boundaries() {
        for valid in [
            "provider",
            " provider/model ",
            "-provider",
            "-provider/model",
            "openrouter/anthropic/claude",
            "p/m:variant",
        ] {
            assert!(is_route_spec(valid), "{valid}");
        }
        for invalid in ["", " ", "-", "/model", "p/", "p//m", "-p//m"] {
            assert!(!is_route_spec(invalid), "{invalid}");
        }
        for (spec, key, expected) in [
            ("p", "p/m", true),
            ("p", "prefix/m", false),
            ("p/m", "p/m", true),
            ("p/m", "p/m/v", true),
            ("p/m", "p/model", false),
            ("p/m", "p/m:variant", false),
            (
                "openrouter/anthropic/claude",
                "openrouter/anthropic/claude/v",
                true,
            ),
            ("p/*", "p/m", false),
            ("p/m:variant", "p/m:variant", true),
        ] {
            assert_eq!(matches(spec, key), expected, "{spec}: {key}");
        }
    }

    #[test]
    fn route_spec_resolution_levels_order_and_base() {
        for base in [false, true] {
            assert_eq!(resolve(["p/m", "-p"], "p/m", base), Some(base));
            assert_eq!(resolve(["-p", "p/m"], "p/m", base), Some(base));
            assert_eq!(resolve(["p/m", "-p/m"], "p/m/v", base), Some(!base));
            assert_eq!(resolve(["-p", "p"], "p/other", base), Some(base));
            assert_eq!(resolve(["-", "p/", "other"], "p/m", base), None);
        }
    }
}
