use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    path::Path,
};

use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;

use crate::{redact_secrets, Evidence, EvidenceKind, EvidenceValue, PathCategory};

use super::{ReportContext, ReportError};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum AliasKind {
    Path,
    Ref,
}

#[derive(Default)]
struct Aliases {
    paths: HashMap<String, u32>,
    refs: HashMap<String, u32>,
    used: HashSet<u32>,
}

impl Aliases {
    fn assign(&mut self, key: &[u8; 32], kind: AliasKind, value: &str) -> Result<u32, ReportError> {
        let (cache, domain) = match kind {
            AliasKind::Path => (&mut self.paths, &b"path\0"[..]),
            AliasKind::Ref => (&mut self.refs, &b"ref\0"[..]),
        };
        if let Some(&window) = cache.get(value) {
            return Ok(window);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a 32-byte key");
        mac.update(domain);
        mac.update(value.as_bytes());
        for &chunk in mac.finalize().into_bytes().as_slice().as_chunks::<4>().0 {
            let window = u32::from_be_bytes(chunk);
            if self.used.insert(window) {
                cache.insert(value.to_owned(), window);
                return Ok(window);
            }
        }
        Err(ReportError::AliasExhausted)
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum ProjectedValue {
    Integer(u64),
    Number(f64),
    Bool(bool),
    Text(String),
}

#[derive(Serialize)]
pub(super) struct ProjectedEvidence<'a> {
    name: &'a str,
    kind: EvidenceKind,
    value: ProjectedValue,
}

pub(super) struct Projection<'a> {
    ctx: &'a ReportContext,
    aliases: Aliases,
    pub(super) dropped_values: usize,
}

impl<'a> Projection<'a> {
    pub(super) fn new(ctx: &'a ReportContext) -> Self {
        Self {
            ctx,
            aliases: Aliases::default(),
            dropped_values: 0,
        }
    }

    pub(super) fn age(&self, millis: i128) -> String {
        let delta = i128::from(self.ctx.report_time_ms) - millis;
        let side = if delta >= 0 { "before" } else { "after" };
        let seconds = delta.unsigned_abs() / 1_000;
        let (n, unit) = match seconds {
            0..60 => (seconds, "s"),
            60..3_600 => (seconds / 60, "min"),
            3_600..172_800 => (seconds / 3_600, "h"),
            _ => (seconds / 86_400, "d"),
        };
        format!("{n} {unit} {side}")
    }

    pub(super) fn evidence<'b>(
        &mut self,
        evidence: &'b [Evidence],
    ) -> Result<Vec<ProjectedEvidence<'b>>, ReportError> {
        evidence
            .iter()
            .map(|item| {
                Ok(ProjectedEvidence {
                    name: item.name(),
                    kind: item.kind(),
                    value: self.value(item.name(), item.value())?,
                })
            })
            .collect()
    }

    pub(super) fn log_field<'b>(
        &mut self,
        projected: &mut HashMap<&'b str, Option<ProjectedValue>>,
        name: &'b str,
        kind: EvidenceKind,
        text: &str,
    ) -> Result<(), ReportError> {
        // Remember an invalid first occurrence as well as a valid one.
        if let Entry::Vacant(slot) = projected.entry(name) {
            let category = if kind == EvidenceKind::Path {
                self.path_category(Path::new(text))
            } else {
                PathCategory::Other
            };
            if let Some(value) = EvidenceValue::from_log(kind, text, category) {
                slot.insert(Some(self.value(name, &value)?));
            } else {
                slot.insert(None);
                self.dropped_values += 1;
            }
        }
        Ok(())
    }

    fn path_category(&self, path: &Path) -> PathCategory {
        if path.starts_with(&self.ctx.data_home) {
            PathCategory::Data
        } else if path.starts_with(&self.ctx.config_home) {
            PathCategory::Config
        } else if path.starts_with(&self.ctx.temp_dir) {
            PathCategory::Temp
        } else {
            PathCategory::Other
        }
    }

    fn value(&mut self, name: &str, value: &EvidenceValue) -> Result<ProjectedValue, ReportError> {
        use ProjectedValue::{Bool, Integer, Number, Text};
        Ok(match value {
            EvidenceValue::Count(n) | EvidenceValue::Bytes(n) => Integer(*n),
            EvidenceValue::DurationMs(n) | EvidenceValue::Ratio(n) => Number(*n),
            EvidenceValue::Bool(b) => Bool(*b),
            EvidenceValue::Code(s) | EvidenceValue::Version(s) => {
                Text(redact_secrets(s).into_owned())
            }
            EvidenceValue::TimestampMs(t) => Text(self.age(i128::from(*t))),
            EvidenceValue::Path { category, path } => {
                let category = match category {
                    PathCategory::Data => "data",
                    PathCategory::Config => "config",
                    PathCategory::Project => "project",
                    PathCategory::Temp => "temp",
                    PathCategory::Other => "other",
                };
                let part = self
                    .aliases
                    .assign(&self.ctx.report_key, AliasKind::Path, path)?;
                Text(format!("{category}/#{part:08x}"))
            }
            EvidenceValue::Ref(value) => {
                let prefix = name
                    .strip_suffix("_id")
                    .or_else(|| name.strip_suffix("_ref"))
                    .unwrap_or(name);
                let prefix = if prefix.is_empty() { "ref" } else { prefix };
                let part = self
                    .aliases
                    .assign(&self.ctx.report_key, AliasKind::Ref, value)?;
                Text(format!("{prefix}#{part:08x}"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_vectors_alias_exhaustion_refuses_without_reusing_a_window() {
        // HMAC for ref\0collision-16873 under [7; 32].
        let mut aliases = Aliases {
            used: HashSet::from([
                0x73855194, 0x2816906c, 0x62f44829, 0x8aad94a3, 0x12c6a426, 0x9cf1cecf, 0xc8ebcc59,
                0xf2d58139,
            ]),
            ..Aliases::default()
        };
        assert_eq!(
            aliases.assign(&[7; 32], AliasKind::Ref, "collision-16873"),
            Err(ReportError::AliasExhausted)
        );
        assert!(aliases.refs.is_empty());
    }
}
