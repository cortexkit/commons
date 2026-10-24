//! `PERSONA.md`: closed YAML front matter, then a MiniJinja body.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use serde::Deserialize;

use crate::{
    duplicates::first_repeated_key,
    extension::{AnyBox, ErasedExtension},
    ids::{check_bare, check_block, check_role},
    FragmentId, PersonaError, SourceRef,
};

/// The core fields every role may carry.
pub(crate) const CORE_FIELDS: [&str; 6] = [
    "name",
    "version",
    "display_name",
    "extends",
    "fragments",
    "disabled_blocks",
];

/// The core fields the first applied layer must supply.
pub(crate) const REQUIRED_FIELDS: [&str; 3] = ["name", "version", "display_name"];

const MAX_DISPLAY_NAME: usize = 128;

/// A layer's explicit parent template: the body of `role`'s layer at `tier`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExtendsRef {
    pub(crate) role: String,
    pub(crate) tier: String,
}

/// One layer's front matter, every field optional.
#[derive(Default)]
pub(crate) struct LayerManifest {
    pub(crate) present: BTreeSet<String>,
    pub(crate) name: Option<String>,
    pub(crate) version: Option<u64>,
    pub(crate) display_name: Option<String>,
    pub(crate) extends: Option<ExtendsRef>,
    pub(crate) fragments: Option<Vec<FragmentId>>,
    pub(crate) disabled_blocks: Option<Vec<String>>,
    pub(crate) extensions: BTreeMap<String, AnyBox>,
}

pub(crate) struct ParsedFile {
    pub(crate) manifest: LayerManifest,
    pub(crate) body: String,
}

pub(crate) type Registry = BTreeMap<String, Arc<dyn ErasedExtension>>;

/// Split and parse a normalised `PERSONA.md`. `role` is the role the file's
/// directory names; a present `name` must equal it.
pub(crate) fn parse_persona(
    text: &str,
    file: &SourceRef,
    role: &str,
    registry: &Registry,
) -> Result<ParsedFile, PersonaError> {
    let (yaml, body) = split_front_matter(text, file)?;
    if !yaml.trim().is_empty() {
        match first_repeated_key(yaml) {
            Ok(None) => {}
            Ok(Some(key)) => {
                return Err(PersonaError::DuplicateKey {
                    file: file.clone(),
                    key,
                })
            }
            Err(message) => {
                return Err(PersonaError::FrontMatter {
                    file: file.clone(),
                    message,
                })
            }
        }
    }
    let mapping = if yaml.trim().is_empty() {
        yaml_serde::Mapping::new()
    } else {
        match yaml_serde::from_str::<yaml_serde::Value>(yaml) {
            Ok(yaml_serde::Value::Mapping(m)) => m,
            Ok(yaml_serde::Value::Null) => yaml_serde::Mapping::new(),
            Ok(_) => {
                return Err(PersonaError::FrontMatter {
                    file: file.clone(),
                    message: "must be a mapping of field names to values".into(),
                })
            }
            Err(e) => {
                return Err(PersonaError::FrontMatter {
                    file: file.clone(),
                    message: e.to_string(),
                })
            }
        }
    };

    let mut m = LayerManifest::default();
    for (key, value) in mapping {
        let Some(key) = key.as_str().map(str::to_string) else {
            return Err(PersonaError::FrontMatter {
                file: file.clone(),
                message: "every field name must be a string".into(),
            });
        };
        let invalid = |reason: String| PersonaError::InvalidField {
            file: file.clone(),
            field: key.clone(),
            reason,
        };
        match key.as_str() {
            "name" => {
                let name = value
                    .as_str()
                    .ok_or_else(|| invalid("must be a string".into()))?;
                check_role(name).map_err(|e| invalid(e.to_string()))?;
                if name != role {
                    return Err(invalid(format!(
                        "is `{name}` but the bundle directory is `{role}`"
                    )));
                }
                m.name = Some(name.to_string());
            }
            "version" => {
                let version = value
                    .as_u64()
                    .filter(|v| *v > 0)
                    .ok_or_else(|| invalid("must be a positive integer".into()))?;
                m.version = Some(version);
            }
            "display_name" => {
                let display = value
                    .as_str()
                    .ok_or_else(|| invalid("must be a string".into()))?;
                if display.trim().is_empty() {
                    return Err(invalid("must not be blank".into()));
                }
                if display.len() > MAX_DISPLAY_NAME {
                    return Err(invalid("must be at most 128 UTF-8 bytes".into()));
                }
                m.display_name = Some(display.to_string());
            }
            "extends" => {
                let parent: ExtendsRef = yaml_serde::from_value(value)
                    .map_err(|e| invalid(format!("must be {{role, tier}}: {e}")))?;
                check_role(&parent.role).map_err(|e| invalid(e.to_string()))?;
                check_bare("tier name", &parent.tier).map_err(|e| invalid(e.to_string()))?;
                m.extends = Some(parent);
            }
            "fragments" => {
                let ids = string_list(&value)
                    .ok_or_else(|| invalid("must be a list of fragment ids".into()))?;
                let mut seen = BTreeSet::new();
                let mut out = Vec::with_capacity(ids.len());
                for id in ids {
                    let id = FragmentId::parse(&id).map_err(|e| invalid(e.to_string()))?;
                    if !seen.insert(id.clone()) {
                        return Err(invalid(format!("lists `{id}` twice")));
                    }
                    out.push(id);
                }
                m.fragments = Some(out);
            }
            "disabled_blocks" => {
                let ids = string_list(&value)
                    .ok_or_else(|| invalid("must be a list of block ids".into()))?;
                let mut seen = BTreeSet::new();
                for id in &ids {
                    check_block(id).map_err(|e| invalid(e.to_string()))?;
                    if !seen.insert(id.clone()) {
                        return Err(invalid(format!("lists `{id}` twice")));
                    }
                }
                m.disabled_blocks = Some(ids);
            }
            other => {
                let Some(ext) = registry.get(other) else {
                    return Err(PersonaError::UnknownField {
                        file: file.clone(),
                        field: key,
                    });
                };
                let parsed = ext.parse(value).map_err(invalid)?;
                m.extensions.insert(key.clone(), parsed);
            }
        }
        m.present.insert(key);
    }
    Ok(ParsedFile {
        manifest: m,
        body: body.to_string(),
    })
}

fn string_list(value: &yaml_serde::Value) -> Option<Vec<String>> {
    value
        .as_sequence()?
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// `---\n<yaml>---\n<body>`. The closing line is exactly `---`.
fn split_front_matter<'t>(
    text: &'t str,
    file: &SourceRef,
) -> Result<(&'t str, &'t str), PersonaError> {
    let Some(rest) = text.strip_prefix("---\n") else {
        return Err(PersonaError::MissingFrontMatter { file: file.clone() });
    };
    let mut offset = 0;
    loop {
        let line_end = rest[offset..].find('\n').map(|i| offset + i);
        let line = &rest[offset..line_end.unwrap_or(rest.len())];
        if line == "---" {
            let body = line_end.map(|end| &rest[end + 1..]).unwrap_or("");
            return Ok((&rest[..offset], body));
        }
        match line_end {
            Some(end) => offset = end + 1,
            None => return Err(PersonaError::UnterminatedFrontMatter { file: file.clone() }),
        }
    }
}

/// The composed manifest of a loaded role.
pub struct Manifest {
    pub(crate) name: String,
    pub(crate) version: u64,
    pub(crate) display_name: String,
    pub(crate) fragments: Vec<FragmentId>,
    pub(crate) disabled_blocks: BTreeSet<String>,
    pub(crate) extensions: BTreeMap<String, (Arc<dyn ErasedExtension>, AnyBox)>,
}

impl Manifest {
    /// The role id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version from the most specific layer that set it.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The display name from the most specific layer that set it.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The fragment ids, from the most specific layer that listed them.
    pub fn fragments(&self) -> &[FragmentId] {
        &self.fragments
    }

    /// The union of every applied layer's `disabled_blocks`, sorted.
    pub fn disabled_blocks(&self) -> &BTreeSet<String> {
        &self.disabled_blocks
    }

    /// The composed value of extension `key`, if it is registered with type `T`
    /// and some layer supplied it.
    pub fn extension<T: 'static>(&self, key: &str) -> Option<&T> {
        self.extensions
            .get(key)
            .and_then(|(_, v)| v.downcast_ref::<T>())
    }

    /// The manifest as JSON, keyed as in the file. It is the `manifest`
    /// member of the source revision's canonical JSON.
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("name".into(), self.name.clone().into());
        map.insert("version".into(), self.version.into());
        map.insert("display_name".into(), self.display_name.clone().into());
        map.insert(
            "fragments".into(),
            self.fragments
                .iter()
                .map(|f| serde_json::Value::from(f.as_str()))
                .collect(),
        );
        map.insert(
            "disabled_blocks".into(),
            self.disabled_blocks
                .iter()
                .map(|b| serde_json::Value::from(b.as_str()))
                .collect(),
        );
        for (key, (ext, value)) in &self.extensions {
            // Serialisation is checked when the role is loaded; if it still
            // fails here the key is written as `null` rather than failing a
            // render.
            let json = ext.to_json(value).unwrap_or(serde_json::Value::Null);
            map.insert(key.clone(), json);
        }
        serde_json::Value::Object(map)
    }
}

impl fmt::Debug for Manifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Manifest({})", self.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> SourceRef {
        SourceRef::File {
            tier: "t".into(),
            path: "roles/r/PERSONA.md".into(),
        }
    }

    #[test]
    fn front_matter_split() {
        let (y, b) = split_front_matter("---\na: 1\n---\nbody\n", &file()).unwrap();
        assert_eq!((y, b), ("a: 1\n", "body\n"));
        let (y, b) = split_front_matter("---\n---", &file()).unwrap();
        assert_eq!((y, b), ("", ""));
        assert!(matches!(
            split_front_matter("body", &file()),
            Err(PersonaError::MissingFrontMatter { .. })
        ));
        assert!(matches!(
            split_front_matter("---\na: 1\n", &file()),
            Err(PersonaError::UnterminatedFrontMatter { .. })
        ));
    }

    #[test]
    fn core_field_rules() {
        let reg = Registry::new();
        let parse = |yaml: &str| parse_persona(&format!("---\n{yaml}---\n"), &file(), "r", &reg);
        assert!(parse("name: r\nversion: 1\ndisplay_name: R\n").is_ok());
        for bad in [
            "name: other\n",
            "version: 0\n",
            "version: -1\n",
            "version: 1.5\n",
            "version: \"1\"\n",
            "display_name: \"  \"\n",
            "fragments: [a, a]\n",
            "fragments: [Bad]\n",
            "disabled_blocks: [has-hyphen]\n",
            "extends: {role: r}\n",
            "extends: {role: r, tier: base, extra: 1}\n",
            "name: r\nname: r\n",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
        let long = format!("display_name: {}\n", "x".repeat(129));
        assert!(parse(&long).is_err());
    }
}
