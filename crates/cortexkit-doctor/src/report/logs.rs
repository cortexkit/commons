use std::{borrow::Cow, collections::HashMap, time::UNIX_EPOCH};

use cortexkit_log::{parse_line, ParsedLevel, ParsedLine};
use serde::Serialize;

use crate::{redact_secrets, EvidenceKind, Manifest, Template};

use super::{project::ProjectedValue, Projection, ReportError};

#[derive(Serialize)]
pub(super) struct Log<'a> {
    age: String,
    level: &'static str,
    text: Cow<'a, str>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    matched: Option<Matched<'a>>,
}

#[derive(Serialize)]
struct Matched<'a> {
    template: &'a str,
    fields: HashMap<&'a str, ProjectedValue>,
}

pub(super) fn sanitize<'a>(
    manifest: &'a Manifest,
    tails: &[&'a str],
    projection: &mut Projection<'_>,
) -> Result<(Vec<Log<'a>>, usize), ReportError> {
    let mut lines = Vec::new();
    let mut omitted = 0;
    for tail in tails {
        for line in tail.split_terminator('\n') {
            // parse_line validates all logger segments and exposes the first.
            match parse_line(line) {
                Ok(line) if line.module_id == manifest.module_id().as_str() => lines.push(line),
                _ => omitted += 1,
            }
        }
    }
    // Stable sorting preserves the tail order, then line order of a tie.
    lines.sort_by_key(|line| line.timestamp);
    let index = TemplateIndex::new(manifest.templates());
    let logs = lines
        .into_iter()
        .map(|line| index.sanitize(line, projection))
        .collect::<Result<_, _>>()?;
    Ok((logs, omitted))
}

struct TemplateIndex<'a> {
    by_text: HashMap<Cow<'a, str>, Group<'a>>,
    max_len: usize,
}

struct Group<'a> {
    first: &'a Template,
    fields: HashMap<&'a str, EvidenceKind>,
}

impl<'a> TemplateIndex<'a> {
    fn new(templates: &'a [Template]) -> Self {
        let mut index = Self {
            by_text: HashMap::new(),
            max_len: 0,
        };
        // A validated manifest already sorts templates by id.
        for template in templates {
            let text = template.text();
            let rendered = if text.contains('\\') {
                Cow::Owned(text.replace('\\', "\\\\"))
            } else {
                Cow::Borrowed(text)
            };
            index.max_len = index.max_len.max(rendered.len());
            index
                .by_text
                .entry(rendered)
                .and_modify(|group| {
                    let fields = template.fields();
                    group.fields.retain(|&name, kind| {
                        fields
                            .binary_search_by_key(&name, |field| field.name())
                            .is_ok_and(|i| fields[i].kind() == *kind)
                    });
                })
                .or_insert_with(|| Group {
                    first: template,
                    fields: template
                        .fields()
                        .iter()
                        .map(|field| (field.name(), field.kind()))
                        .collect(),
                });
        }
        index
    }

    fn find(&self, body: &'a str) -> Option<(&Group<'a>, Vec<Field<'a>>)> {
        if let Some(group) = self.by_text.get(body) {
            return Some((group, Vec::new()));
        }
        // Only a prefix ending just before a field can match. Look longest
        // first and stop looking for boundaries beyond any template's length.
        let end = body.floor_char_boundary((self.max_len + 1).min(body.len()));
        body[..end].rmatch_indices(' ').find_map(|(i, _)| {
            let group = self.by_text.get(&body[..i])?;
            Some((group, lex_fields(&body[i..])?))
        })
    }

    fn sanitize(
        &self,
        line: ParsedLine<'a>,
        projection: &mut Projection<'_>,
    ) -> Result<Log<'a>, ReportError> {
        let millis = match line.timestamp.duration_since(UNIX_EPOCH) {
            Ok(delta) => delta.as_millis() as i128,
            Err(error) => -(error.duration().as_millis() as i128),
        };
        let mut log = Log {
            age: projection.age(millis),
            level: match line.level {
                ParsedLevel::Trace => "trace",
                ParsedLevel::Debug => "debug",
                ParsedLevel::Info => "info",
                ParsedLevel::Warn => "warn",
                ParsedLevel::Error => "error",
            },
            text: Cow::Borrowed("text omitted"),
            matched: None,
        };
        if let Some((group, fields)) = self.find(line.body) {
            let mut projected = HashMap::new();
            for (name, text) in fields {
                if let Some(&kind) = group.fields.get(name) {
                    projection.log_field(&mut projected, name, kind, &text)?;
                }
            }
            log.text = redact_secrets(group.first.text());
            log.matched = Some(Matched {
                template: group.first.id(),
                fields: projected
                    .into_iter()
                    .filter_map(|(name, value)| Some((name, value?)))
                    .collect(),
            });
        }
        Ok(log)
    }
}

type Field<'a> = (&'a str, Cow<'a, str>);

fn lex_fields(mut rest: &str) -> Option<Vec<Field<'_>>> {
    let mut fields = Vec::new();
    while !rest.is_empty() {
        rest = rest.strip_prefix(' ')?;
        let (key, value) = rest.split_at(rest.find([' ', '=', '"'])?);
        if key.is_empty() {
            return None;
        }
        let value = value.strip_prefix('=')?;
        let (value, tail) = if let Some(quoted) = value.strip_prefix('"') {
            lex_quoted(quoted)?
        } else {
            let (word, tail) = value.split_at(value.find([' ', '"']).unwrap_or(value.len()));
            if word.is_empty() {
                return None;
            }
            (Cow::Borrowed(word), tail)
        };
        fields.push((key, value));
        rest = tail;
    }
    Some(fields)
}

fn lex_quoted(mut rest: &str) -> Option<(Cow<'_, str>, &str)> {
    let mut decoded = None::<String>;
    loop {
        let (plain, special) = rest.split_at(rest.find(['"', '\\'])?);
        if let Some(tail) = special.strip_prefix('"') {
            return Some((
                match decoded {
                    None => Cow::Borrowed(plain),
                    Some(mut decoded) => {
                        decoded.push_str(plain);
                        Cow::Owned(decoded)
                    }
                },
                tail,
            ));
        }
        let decoded = decoded.get_or_insert_with(String::new);
        decoded.push_str(plain);
        let escape = special.as_bytes().get(1)?;
        rest = special.get(2..)?;
        let ch = match escape {
            b'\\' => '\\',
            b'"' => '"',
            b'r' => '\r',
            b'n' => '\n',
            b'u' => {
                let digits = rest.get(..4)?;
                if !crate::wire::is_lower_hex(digits, 4) {
                    return None;
                }
                rest = &rest[4..];
                char::from_u32(u32::from_str_radix(digits, 16).ok()?)?
            }
            _ => return None,
        };
        decoded.push(ch);
    }
}
