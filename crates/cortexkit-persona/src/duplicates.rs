//! Repeated mapping keys in front matter.
//!
//! A YAML mapping that repeats a key is ambiguous: a reader of the file sees
//! the first value, while a parser that keeps the last one applies something
//! else. In a repository file that is a way to hide what a layer does, so the
//! engine refuses a repeated key at any depth and names its path.
//! `yaml_serde` happens to refuse repeats as well, but only with an untyped
//! message; this walk names the key and does not depend on that behaviour.

use std::{collections::BTreeSet, fmt};

use serde::de::{
    Deserialize, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor,
};

/// A YAML node with every mapping entry kept, repeats included.
enum Node {
    Scalar(Scalar),
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
    Tagged(String, Box<Node>),
}

/// A scalar, typed so that `1`, `"1"` and `true` stay distinct keys, as they
/// are distinct keys in YAML.
enum Scalar {
    Null,
    Bool(bool),
    Int(i128),
    Float(u64),
    Str(String),
}

impl Node {
    /// A canonical text that is equal for equal keys and distinct otherwise.
    fn canonical(&self) -> String {
        match self {
            Node::Scalar(Scalar::Null) => "n:".into(),
            Node::Scalar(Scalar::Bool(b)) => format!("b:{b}"),
            Node::Scalar(Scalar::Int(i)) => format!("i:{i}"),
            Node::Scalar(Scalar::Float(bits)) => format!("f:{bits}"),
            Node::Scalar(Scalar::Str(s)) => format!("s:{s:?}"),
            Node::Seq(items) => {
                let inner: Vec<String> = items.iter().map(Node::canonical).collect();
                format!("[{}]", inner.join(","))
            }
            Node::Map(entries) => {
                let inner: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}={}", k.canonical(), v.canonical()))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
            Node::Tagged(tag, inner) => format!("!{tag:?}:{}", inner.canonical()),
        }
    }

    /// How a key is shown in a path: a scalar as written, anything else in
    /// its canonical form.
    fn shown(&self) -> String {
        match self {
            Node::Scalar(Scalar::Str(s)) => s.clone(),
            Node::Scalar(Scalar::Int(i)) => i.to_string(),
            Node::Scalar(Scalar::Bool(b)) => b.to_string(),
            Node::Scalar(Scalar::Float(bits)) => f64::from_bits(*bits).to_string(),
            Node::Scalar(Scalar::Null) => "null".into(),
            other => other.canonical(),
        }
    }
}

/// The path of the first repeated key in `yaml`, such as `version`,
/// `limits.max_words` or `flows[0].package`; `Err` with the parser's message
/// when the text is not YAML.
pub(crate) fn first_repeated_key(yaml: &str) -> Result<Option<String>, String> {
    let node =
        Node::deserialize(yaml_serde::Deserializer::from_str(yaml)).map_err(|e| e.to_string())?;
    Ok(find(&node, ""))
}

fn find(node: &Node, path: &str) -> Option<String> {
    match node {
        Node::Scalar(_) => None,
        Node::Tagged(_, inner) => find(inner, path),
        Node::Seq(items) => items
            .iter()
            .enumerate()
            .find_map(|(i, item)| find(item, &format!("{path}[{i}]"))),
        Node::Map(entries) => {
            let mut seen = BTreeSet::new();
            for (key, value) in entries {
                let here = if path.is_empty() {
                    key.shown()
                } else {
                    format!("{path}.{}", key.shown())
                };
                if !seen.insert(key.canonical()) {
                    return Some(here);
                }
                if let Some(found) = find(value, &here) {
                    return Some(found);
                }
            }
            None
        }
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any YAML value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Bool(v)))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Int(v.into())))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Int(v.into())))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Float(v.to_bits())))
    }

    fn visit_str<E>(self, v: &str) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Str(v.to_string())))
    }

    fn visit_string<E>(self, v: String) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Str(v)))
    }

    fn visit_unit<E>(self) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Null))
    }

    fn visit_none<E>(self) -> Result<Node, E> {
        Ok(Node::Scalar(Scalar::Null))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        Node::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Node::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let mut entries = Vec::new();
        while let Some(key) = map.next_key()? {
            entries.push((key, map.next_value()?));
        }
        Ok(Node::Map(entries))
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Node, A::Error> {
        // `yaml_serde` presents a tagged value (`!tag value`) as an enum.
        let (tag, variant) = data.variant::<String>()?;
        Ok(Node::Tagged(tag, Box::new(variant.newtype_variant()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_keys_are_found_at_any_depth() {
        let cases = [
            ("a: 1\nb: 2\n", None),
            ("version: 1\nversion: 2\n", Some("version")),
            ("limits:\n  x: 1\n  x: 2\n", Some("limits.x")),
            (
                "flows:\n  - {package: a, package: b}\n",
                Some("flows[0].package"),
            ),
            ("a: {1: x, \"1\": y, true: z}\n", None),
            ("a: {1: x, 1: y}\n", Some("a.1")),
            ("x: !t {k: 1, k: 2}\n", Some("x.k")),
        ];
        for (yaml, expected) in cases {
            assert_eq!(
                first_repeated_key(yaml).unwrap().as_deref(),
                expected,
                "{yaml:?}"
            );
        }
        assert!(first_repeated_key("a: [").is_err());
    }
}
