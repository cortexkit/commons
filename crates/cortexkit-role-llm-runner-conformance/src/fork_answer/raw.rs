//! Lexical JSON comparison: parsing locates the conversation and cache markers,
//! but never reserializes the frozen bytes the model actually received.
use serde_json::Value;
use std::ops::Range;

type Check<T> = Result<T, String>;

struct Node {
    span: Range<usize>,
    kind: Kind,
}
enum Kind {
    Object(Vec<(String, usize, Node)>),
    Array(Vec<Node>),
    Scalar,
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Parser<'_> {
    fn whitespace(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn string(&mut self) -> Range<usize> {
        let start = self.at;
        self.at += 1;
        while self.bytes[self.at] != b'"' {
            if self.bytes[self.at] == b'\\' {
                self.at += 1;
            }
            self.at += 1;
        }
        self.at += 1;
        start..self.at
    }
    fn node(&mut self) -> Node {
        self.whitespace();
        let start = self.at;
        let kind = match self.bytes[self.at] {
            b'{' => {
                self.at += 1;
                self.whitespace();
                let mut fields = Vec::new();
                while self.bytes[self.at] != b'}' {
                    let key_start = self.at;
                    let range = self.string();
                    let key = serde_json::from_slice(&self.bytes[range]).unwrap();
                    self.whitespace();
                    self.at += 1;
                    fields.push((key, key_start, self.node()));
                    self.whitespace();
                    if self.bytes[self.at] != b',' {
                        break;
                    }
                    self.at += 1;
                    self.whitespace();
                }
                self.at += 1;
                Kind::Object(fields)
            }
            b'[' => {
                self.at += 1;
                self.whitespace();
                let mut values = Vec::new();
                while self.bytes[self.at] != b']' {
                    values.push(self.node());
                    self.whitespace();
                    if self.bytes[self.at] != b',' {
                        break;
                    }
                    self.at += 1;
                    self.whitespace();
                }
                self.at += 1;
                Kind::Array(values)
            }
            b'"' => {
                self.string();
                Kind::Scalar
            }
            _ => {
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|b| !b.is_ascii_whitespace() && !b",]}".contains(b))
                {
                    self.at += 1;
                }
                Kind::Scalar
            }
        };
        Node {
            span: start..self.at,
            kind,
        }
    }
}

fn parse(bytes: &[u8]) -> Check<Node> {
    serde_json::from_slice::<Value>(bytes).map_err(|e| format!("raw request is not JSON: {e}"))?;
    let node = Parser { bytes, at: 0 }.node();
    unique_members(&node)?;
    Ok(node)
}

fn unique_members(node: &Node) -> Check<()> {
    match &node.kind {
        Kind::Object(fields) => {
            let mut names = std::collections::BTreeSet::new();
            for (name, _, value) in fields {
                if !names.insert(name) {
                    return Err(format!("raw request has duplicate member {name}"));
                }
                unique_members(value)?;
            }
        }
        Kind::Array(values) => {
            for value in values {
                unique_members(value)?;
            }
        }
        Kind::Scalar => {}
    }
    Ok(())
}

fn member<'a>(node: &'a Node, key: &str) -> Option<&'a Node> {
    let Kind::Object(fields) = &node.kind else {
        return None;
    };
    fields
        .iter()
        .find(|(name, _, _)| name == key)
        .map(|(_, _, node)| node)
}

fn content_blocks(root: &Node) -> Vec<&Node> {
    let mut blocks = Vec::new();
    if let Some(Node {
        kind: Kind::Array(system),
        ..
    }) = member(root, "system")
    {
        blocks.extend(system);
    }
    if let Some(Node {
        kind: Kind::Array(messages),
        ..
    }) = member(root, "messages")
    {
        for message in messages {
            if let Some(Node {
                kind: Kind::Array(content),
                ..
            }) = member(message, "content")
            {
                blocks.extend(content);
            }
        }
    }
    blocks
}

fn cache_removals(root: &Node) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    for block in content_blocks(root) {
        let Kind::Object(fields) = &block.kind else {
            continue;
        };
        // The explicit-breakpoint shape is an Anthropic content block. A
        // similarly named member in arbitrary metadata is not an exception.
        if !fields.iter().any(|(name, _, _)| name == "type") {
            continue;
        }
        for (i, (key, start, value)) in fields.iter().enumerate() {
            if key != "cache_control" {
                continue;
            }
            let range = if i > 0 {
                fields[i - 1].2.span.end..value.span.end
            } else if fields.len() > 1 {
                *start..fields[1].1
            } else {
                *start..value.span.end
            };
            ranges.push(range);
        }
    }
    ranges.sort_by_key(|r| r.start);
    ranges
}

fn breakpoint_count(node: &Node) -> usize {
    match &node.kind {
        Kind::Object(fields) => fields
            .iter()
            .map(|(name, _, value)| usize::from(name == "cache_control") + breakpoint_count(value))
            .sum(),
        Kind::Array(values) => values.iter().map(breakpoint_count).sum(),
        Kind::Scalar => 0,
    }
}

fn normalize(bytes: &[u8], ranges: &[Range<usize>]) -> Vec<u8> {
    let mut normalized = Vec::new();
    let mut at = 0;
    for range in ranges {
        normalized.extend_from_slice(&bytes[at..range.start]);
        at = range.end;
    }
    normalized.extend_from_slice(&bytes[at..]);
    normalized
}

fn difference(label: &str, a: &[u8], b: &[u8]) -> String {
    let at = a
        .iter()
        .zip(b)
        .position(|(a, b)| a != b)
        .unwrap_or(a.len().min(b.len()));
    let excerpt = |bytes: &[u8]| {
        String::from_utf8_lossy(
            &bytes[at.saturating_sub(16).min(bytes.len())..(at + 24).min(bytes.len())],
        )
        .into_owned()
    };
    format!(
        "{label}: first differing byte offset {at}; head {:?}; fork {:?}",
        excerpt(a),
        excerpt(b)
    )
}

fn conversation(root: &Node) -> Check<&Node> {
    for field in ["messages", "input", "contents"] {
        if let Some(node) = member(root, field) {
            if matches!(node.kind, Kind::Array(_)) {
                return Ok(node);
            }
        }
    }
    Err("raw model request has no conversation array (messages, input or contents)".into())
}

fn mask(bytes: &[u8], span: &Range<usize>) -> Vec<u8> {
    let mut result = bytes[..span.start].to_vec();
    result.extend_from_slice(b"[]");
    result.extend_from_slice(&bytes[span.end..]);
    result
}

/// Only append to the conversation. Explicit content-block cache breakpoints
/// may relocate, but cannot increase in count and can never exceed four.
pub(super) fn extends_base(base: &[u8], fork: &[u8]) -> Check<()> {
    let root = parse(base)?;
    let next = parse(fork)?;
    let removals = cache_removals(&root);
    let next_removals = cache_removals(&next);
    if breakpoint_count(&next) > 4 || breakpoint_count(&next) > breakpoint_count(&root) {
        return Err("fork cache_control breakpoint count increased or exceeds four".into());
    }
    let base = normalize(base, &removals);
    let fork = normalize(fork, &next_removals);
    let root = parse(&base)?;
    let next = parse(&fork)?;
    let a = conversation(&root)?;
    let b = conversation(&next)?;
    let settings = mask(&base, &a.span);
    let next_settings = mask(&fork, &b.span);
    if settings != next_settings {
        return Err(difference(
            "frozen request settings changed",
            &settings,
            &next_settings,
        ));
    }
    let frozen = &base[a.span.start + 1..a.span.end - 1];
    let appended = &fork[b.span.start + 1..b.span.end - 1];
    if !appended.starts_with(frozen) {
        return Err(difference(
            "frozen conversation prefix changed",
            frozen,
            appended,
        ));
    }
    let Kind::Array(before) = &a.kind else {
        unreachable!()
    };
    let Kind::Array(after) = &b.kind else {
        unreachable!()
    };
    if after.len() <= before.len() {
        return Err("fork did not append a question to its base".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fork_answer_raw_prefix_preserves_non_marker_bytes() {
        let head=br#"{"model":"a", "messages":[{"role":"user","content":[{"type":"text","text":"frozen","cache_control":{"type":"ephemeral"}}]}],"tools":[]}"#;
        let relocated=br#"{"model":"a", "messages":[{"role":"user","content":[{"type":"text","text":"frozen"}]},{"role":"user","content":[{"type":"text","text":"question","cache_control":{"type":"ephemeral"}}]}],"tools":[]}"#;
        extends_base(head, relocated).unwrap();
        for wrong in [
            String::from_utf8(relocated.to_vec())
                .unwrap()
                .replace("frozen", "frozen!"),
            String::from_utf8(relocated.to_vec())
                .unwrap()
                .replace("\"model\":\"a\", ", "\"model\":\"a\","),
        ] {
            let error = extends_base(head, wrong.as_bytes()).unwrap_err();
            assert!(error.contains("first differing byte offset"), "{error}");
            assert!(error.contains("head") && error.contains("fork"), "{error}");
        }
        let extra = String::from_utf8(relocated.to_vec()).unwrap().replace(
            "\"text\":\"frozen\"",
            "\"text\":\"frozen\",\"cache_control\":{\"type\":\"ephemeral\"}",
        );
        assert!(extends_base(head, extra.as_bytes())
            .unwrap_err()
            .contains("count"));
        let metadata = relocated.to_vec();
        let metadata = String::from_utf8(metadata)
            .unwrap()
            .replace("\"tools\":[]", "\"tools\":[],\"cache_control\":{}");
        assert!(extends_base(head, metadata.as_bytes()).is_err());
    }
}
