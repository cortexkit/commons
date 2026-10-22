use crate::expression::{literal, reply_value, ExprParser, Result};
use crate::*;
use std::collections::{HashMap, HashSet};

pub(crate) struct Engine<'s> {
    pub source: &'s str,
    pub mode: Mode,
    nodes: usize,
    pub blocks: usize,
    tags: usize,
    declarations: usize,
    names: HashSet<String>,
    remaining_work: usize,
    diagnostics: Vec<Diagnostic>,
    suppressed: usize,
    pub fatal: bool,
    body_depth: usize,
    pub expression_frames: usize,
    structural_failures: usize,
}

impl<'s> Engine<'s> {
    pub fn work(&mut self, count: usize) -> Result<()> {
        if count > self.remaining_work {
            self.fatal = true;
            return Err("E_PARSE_WORK");
        }
        self.remaining_work -= count;
        Ok(())
    }
    pub fn report(&mut self, code: &'static str, span: Span) {
        if self.diagnostics.len() < DIAGNOSTICS {
            self.diagnostics.push(Diagnostic { code, span });
        } else {
            self.suppressed += 1;
        }
    }
    pub fn node(&mut self, span: Span, kind: Kind, children: Vec<Node>) -> Result<Node> {
        self.nodes += 1;
        if self.nodes > AST_NODES {
            self.fatal = true;
            return Err("E_LIMIT_AST");
        }
        Ok(Node {
            span,
            kind,
            children,
        })
    }
    fn opaque(&mut self, start: usize, end: usize) -> Result<Node> {
        self.node(
            Span::new(start, end),
            Kind::Literal(self.source[start..end].replace('\0', "\u{fffd}")),
            vec![],
        )
    }
    fn expr(&mut self, start: usize, end: usize, tag_depth: usize) -> Result<(Node, usize)> {
        let mut p = ExprParser {
            engine: self,
            pos: start,
            end,
            tag_depth,
        };
        let node = p.expression(0, 0)?;
        p.ws();
        Ok((node, p.pos))
    }
    /// Opening-tag errors have no component span. Keep just the candidate prefix
    /// opaque, stopping at the first blank line so later paragraphs remain live.
    fn candidate_end(&mut self, start: usize, end: usize) -> usize {
        let mut pos = start;
        let mut quote = None;
        let mut escaped = false;
        let mut braces = 0;
        while pos < end {
            if self.work(1).is_err() {
                return pos;
            }
            if at_line_start(self.source, pos)
                && self.source[pos..line_content_end(self.source, pos, end)]
                    .trim()
                    .is_empty()
            {
                return pos;
            }
            let c = self.source.as_bytes()[pos];
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if matches!(c, b'\'' | b'"') {
                quote = Some(c);
            } else if c == b'{' {
                braces += 1;
            } else if c == b'}' {
                braces = (braces - 1).max(0);
            } else if c == b'>' && braces == 0 {
                return pos + 1;
            }
            pos += self.source[pos..end]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
        }
        end
    }
    pub fn tag(
        &mut self,
        start: usize,
        end: usize,
        depth: usize,
        block_candidate: bool,
    ) -> Result<(Node, usize, bool)> {
        let structural_before = self.structural_failures;
        let opening = self.open_tag(start, end, depth);
        let (name, props, open_end, self_closing) = match opening {
            Ok(value) => value,
            Err(code) => {
                if code == "E_NESTING" {
                    self.structural_failures += 1;
                }
                let stop = self.candidate_end(start, end).max(start + 1);
                self.report(code, Span::new(start, stop));
                return Ok((self.opaque(start, stop)?, stop, true));
            }
        };
        self.tags += 1;
        let over_tags = self.mode == Mode::Reply && self.tags > 64;
        if self.tags == 65 && self.mode == Mode::Reply {
            self.report("E_LIMIT_TAGS", Span::new(start, open_end));
        }
        let block = block_candidate
            && self.source[open_end..line_content_end(self.source, open_end, end)]
                .trim()
                .is_empty();
        let mut stop = open_end;
        let mut children = vec![];
        let mut broken = false;
        if !self_closing {
            let (content, content_end, close) =
                self.body(open_end, end, depth, !block, true, true)?;
            children = content;
            stop = content_end;
            match close {
                Some((close_name, close_end)) => {
                    stop = close_end;
                    if close_name != name {
                        broken = true;
                        self.report("E_MISMATCHED_CLOSE", Span::new(start, stop));
                    }
                    if block {
                        let tail = line_content_end(self.source, close_end, end);
                        if !block_position(self.source, content_end)
                            || !self.source[close_end..tail].trim().is_empty()
                        {
                            broken = true;
                            self.report("E_MISMATCHED_CLOSE", Span::new(start, stop));
                        }
                    }
                }
                None => {
                    broken = true;
                    self.report("E_UNCLOSED_TAG", Span::new(start, stop));
                }
            }
        }
        if depth > TAG_DEPTH {
            broken = true;
            self.report("E_NESTING", Span::new(start, stop));
        }
        if broken {
            self.structural_failures += 1;
        }
        if !children.is_empty() && props.iter().any(|p| p.name == "children") {
            broken = true;
            self.report("E_DUPLICATE_PROP", Span::new(start, stop));
        }
        if self.mode == Mode::Reply && props.iter().any(|p| !reply_value(&p.value)) {
            broken = true;
            self.report("E_REPLY_FORBIDDEN", Span::new(start, stop));
        }
        // A crossed or missing close invalidates the outer component span. A
        // child's forbidden prop, by contrast, is a local literal-text failure.
        if self.structural_failures > structural_before {
            broken = true;
        }
        if broken || over_tags {
            return Ok((self.opaque(start, stop)?, stop, true));
        }
        let node = self.node(
            Span::new(start, stop),
            Kind::Tag { name, block, props },
            children,
        )?;
        Ok((node, stop, false))
    }
    fn open_tag(
        &mut self,
        start: usize,
        end: usize,
        depth: usize,
    ) -> Result<(String, Vec<Prop>, usize, bool)> {
        if depth > TAG_DEPTH {
            return Err("E_NESTING");
        }
        let bytes = self.source.as_bytes();
        if bytes.get(start) != Some(&b'<')
            || !bytes.get(start + 1).is_some_and(u8::is_ascii_uppercase)
        {
            return Err("E_SYNTAX");
        }
        let mut pos = start + 2;
        while pos < end && bytes[pos].is_ascii_alphanumeric() {
            pos += 1;
        }
        let name = self.source[start + 1..pos].to_owned();
        let mut props = Vec::new();
        let mut unique = HashSet::new();
        loop {
            self.work(1)?;
            let before = pos;
            while pos < end && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if has_blank_line(&self.source[before..pos]) {
                return Err("E_SYNTAX");
            }
            if self.source[pos..end].starts_with("/>") {
                return Ok((name, props, pos + 2, true));
            }
            if self.source[pos..end].starts_with('>') {
                return Ok((name, props, pos + 1, false));
            }
            if before == pos {
                return Err("E_SYNTAX");
            }
            let prop_start = pos;
            let mut p = ExprParser {
                engine: self,
                pos,
                end,
                tag_depth: depth,
            };
            let prop_name = p.identifier()?;
            if !unique.insert(prop_name.clone()) {
                return Err("E_DUPLICATE_PROP");
            }
            let name_end = p.pos;
            p.ws();
            let value = if p.eat("=") {
                p.ws();
                if p.rest_is_string() {
                    let start = p.pos;
                    let text = p.string()?;
                    p.engine.node(
                        Span::new(start, p.pos),
                        Kind::Expression(Expression::String(text)),
                        vec![],
                    )?
                } else {
                    if !p.eat("{") {
                        return Err("E_SYNTAX");
                    }
                    let node = p.expression(0, 0)?;
                    if !p.eat("}") {
                        return Err("E_SYNTAX");
                    }
                    node
                }
            } else {
                p.pos = name_end;
                p.engine.node(
                    Span::new(prop_start, p.pos),
                    Kind::Expression(Expression::Boolean(true)),
                    vec![],
                )?
            };
            pos = p.pos;
            if has_blank_line(&self.source[prop_start..pos]) {
                return Err("E_SYNTAX");
            }
            props.push(Prop {
                name: prop_name,
                span: Span::new(prop_start, pos),
                value,
            });
        }
    }
    fn declaration_end(&mut self, start: usize, end: usize) -> Result<usize> {
        let mut stop = line_content_end(self.source, start, end);
        while stop < end {
            self.work(stop - start + 1)?;
            let next_start = line_end(self.source, stop, end);
            let next_end = line_content_end(self.source, next_start, end);
            if self.source[next_start..next_end].trim().is_empty() {
                break;
            }
            let text = self.source[start..stop].trim_end();
            let next = self.source[next_start..next_end].trim_start();
            let continuation = declaration_continues(text)
                || ["?", ":", "&&", "||", "??", "."]
                    .iter()
                    .any(|x| next.starts_with(x));
            if !continuation {
                break;
            }
            stop = next_end;
        }
        Ok(stop)
    }
    fn declaration(
        &mut self,
        start: usize,
        stop: usize,
        tag_depth: usize,
    ) -> Result<(Node, usize)> {
        self.declarations += 1;
        if self.declarations > DECLARATIONS {
            return Err("E_LIMIT_DECLARATIONS");
        }
        let mut p = ExprParser {
            engine: self,
            pos: start + 6,
            end: stop,
            tag_depth,
        };
        let state = if p.eat("let") {
            true
        } else if p.eat("const") {
            false
        } else {
            return Err("E_SYNTAX");
        };
        let before = p.pos;
        p.ws();
        if p.pos == before {
            return Err("E_SYNTAX");
        }
        if state && !p.eat("$") {
            return Err("E_SYNTAX");
        }
        if state
            && p.engine
                .source
                .as_bytes()
                .get(p.pos)
                .is_some_and(u8::is_ascii_whitespace)
        {
            return Err("E_SYNTAX");
        }
        let name = p.identifier()?;
        if name.as_bytes()[0].is_ascii_uppercase()
            || matches!(name.as_str(), "data" | "input" | "view" | "now" | "zone")
        {
            return Err("E_RESERVED_NAME");
        }
        if !p.engine.names.insert(name.clone()) {
            return Err("E_DUPLICATE_NAME");
        }
        if !p.eat("=") {
            return Err("E_SYNTAX");
        }
        let child = p.expression(0, 0).map_err(|code| {
            if code == "E_SYNTAX" {
                "E_DECLARATION_INCOMPLETE"
            } else {
                code
            }
        })?;
        p.ws();
        if p.pos != stop {
            return Err("E_SYNTAX");
        }
        if state && !literal(&child) {
            return Err("E_LITERAL_REQUIRED");
        }
        Ok((
            p.engine.node(
                Span::new(start, stop),
                Kind::Declaration { name, state },
                vec![child],
            )?,
            stop,
        ))
    }
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn body(
        &mut self,
        start: usize,
        end: usize,
        depth: usize,
        inline: bool,
        allow_tags: bool,
        stop_at_close: bool,
    ) -> Result<(Vec<Node>, usize, Option<(String, usize)>)> {
        self.body_depth += 1;
        if self.body_depth > 64 {
            self.fatal = true;
            return Err("E_NESTING");
        }
        self.work(end - start)?;
        let contexts = markdown_contexts(self.source, start, end);
        let code_matches = &contexts.code;
        let mut specials = Vec::new();
        let mut pos = start;
        let mut close = None;
        while pos < end {
            self.work(1)?;
            let rest = &self.source[pos..end];
            if inline
                && at_line_start(self.source, pos)
                && self.source[pos..line_content_end(self.source, pos, end)]
                    .trim()
                    .is_empty()
            {
                break;
            }
            if let Some(stop) = thematic(self.source, pos, end) {
                specials.push(self.node(
                    Span::new(pos, stop),
                    Kind::Markdown(Markdown::ThematicBreak),
                    vec![],
                )?);
                pos = stop;
                continue;
            }
            if let Some((language, fence_end, text)) =
                contexts.fences.get(&pos).cloned().or_else(|| {
                    fence(self.source, pos, end).map(|(language, stop)| {
                        (language, stop, fence_content(self.source, pos, stop))
                    })
                })
            {
                let node = self.node(
                    Span::new(pos, fence_end),
                    Kind::Markdown(Markdown::CodeBlock { language, text }),
                    vec![],
                )?;
                specials.push(node);
                pos = fence_end;
                continue;
            }
            if let Some(tail) = rest.strip_prefix('\\') {
                if let Some(c) = tail.chars().next() {
                    if c.is_ascii_punctuation() || c == '\n' || c == '\r' {
                        pos += 1 + c.len_utf8();
                        continue;
                    }
                }
            }
            if rest.starts_with('`') {
                if let Some(&stop) = code_matches.get(&pos) {
                    specials.push(self.node(
                        Span::new(pos, stop),
                        Kind::Markdown(Markdown::Code(code_text(&self.source[pos..stop]))),
                        vec![],
                    )?);
                    pos = stop;
                    continue;
                }
                pos += rest.bytes().take_while(|b| *b == b'`').count();
                continue;
            }
            if stop_at_close && rest.starts_with("</") {
                if let Some((name, stop)) = close_tag(self.source, pos, end) {
                    close = Some((name, stop));
                    break;
                }
            }
            let link_start = if rest.starts_with("![") {
                Some(pos + 1)
            } else if rest.starts_with('[') {
                Some(pos)
            } else {
                None
            };
            if let Some(bracket) = link_start.filter(|_| contexts.links.contains_key(&pos)) {
                if let Some((label_end, stop, _, _)) = self.link(bracket, end, code_matches)? {
                    let (children, _, _) =
                        self.body(bracket + 1, label_end, depth, true, false, false)?;
                    specials.push(self.node(
                        Span::new(pos, stop),
                        Kind::Markdown(Markdown::Link {
                            destination: contexts.links[&pos].0.clone(),
                            title: contexts.links[&pos].1.clone(),
                            image: bracket != pos,
                        }),
                        children,
                    )?);
                    pos = stop;
                    continue;
                }
            }
            if rest.starts_with("<http://") || rest.starts_with("<https://") {
                let found = rest.find('>');
                self.work(found.unwrap_or(rest.len()))?;
                if let Some(offset) = found {
                    let stop = pos + offset + 1;
                    specials.push(self.node(
                        Span::new(pos, stop),
                        Kind::Markdown(Markdown::Link {
                            destination: rest[1..offset].to_owned(),
                            title: String::new(),
                            image: false,
                        }),
                        vec![],
                    )?);
                    pos = stop;
                    continue;
                }
            }
            if rest.starts_with("https://") || rest.starts_with("http://") {
                let len = rest
                    .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>'))
                    .unwrap_or(rest.len());
                let stop = pos + len;
                specials.push(self.node(
                    Span::new(pos, stop),
                    Kind::Markdown(Markdown::Link {
                        destination: self.source[pos..stop].to_owned(),
                        title: String::new(),
                        image: false,
                    }),
                    vec![],
                )?);
                pos = stop;
                continue;
            }
            if allow_tags
                && rest.starts_with('<')
                && rest.as_bytes().get(1).is_some_and(u8::is_ascii_uppercase)
            {
                let line_start = self.source[..pos].rfind(['\r', '\n']).map_or(0, |i| i + 1);
                self.work(pos - line_start)?;
                let block = !inline && block_position(self.source, pos);
                let (node, stop, _) = self.tag(pos, end, depth + 1, block)?;
                specials.push(node);
                pos = stop;
                continue;
            }
            if at_line_start(self.source, pos) && declaration_start(rest) {
                let stop = self.declaration_end(pos, end)?;
                if self.mode == Mode::Reply {
                    specials.push(self.opaque(pos, stop)?);
                    pos = stop;
                    continue;
                }
                match self.declaration(pos, stop, depth) {
                    Ok((node, stop)) => {
                        specials.push(node);
                        pos = stop;
                        continue;
                    }
                    Err(code) => {
                        self.report(code, Span::new(pos, stop));
                        specials.push(self.opaque(pos, stop)?);
                        pos = stop;
                        continue;
                    }
                }
            }
            if self.mode == Mode::Reply && rest.starts_with('{') {
                let stop = self.literal_braces(pos, end)?;
                specials.push(self.opaque(pos, stop)?);
                pos = stop;
                continue;
            }
            if self.mode == Mode::Program && rest.starts_with('{') {
                match self.expr(pos + 1, end, depth) {
                    Ok((node, next))
                        if self.source[next..end].starts_with('}')
                            && !has_blank_line(&self.source[pos..next]) =>
                    {
                        let stop = next + 1;
                        let block = !inline
                            && block_position(self.source, pos)
                            && self.source[stop..line_content_end(self.source, stop, end)]
                                .trim()
                                .is_empty();
                        specials.push(self.node(
                            Span::new(pos, stop),
                            Kind::Interpolation { block },
                            vec![node],
                        )?);
                        pos = stop;
                        continue;
                    }
                    Err(code) => self.report(code, Span::new(pos, pos + 1)),
                    _ => self.report("E_SYNTAX", Span::new(pos, pos + 1)),
                }
                let stop = self.literal_braces(pos, end)?;
                specials.push(self.opaque(pos, stop)?);
                pos = stop;
                continue;
            }
            if let Some(stop) = reference_end(&self.source[..end], pos) {
                specials.push(self.node(
                    Span::new(pos, stop),
                    Kind::Reference {
                        n: self.source[pos + 5..stop - 1].to_owned(),
                    },
                    vec![],
                )?);
                pos = stop;
                continue;
            }
            pos += rest.chars().next().map_or(1, char::len_utf8);
        }
        let nodes = crate::markdown::assemble(self, start, pos, specials, inline)?;
        self.body_depth -= 1;
        Ok((nodes, pos, close))
    }
    fn literal_braces(&mut self, start: usize, end: usize) -> Result<usize> {
        let mut depth = 1;
        let mut pos = start + 1;
        while pos < end {
            self.work(1)?;
            if at_line_start(self.source, pos)
                && self.source[pos..line_content_end(self.source, pos, end)]
                    .trim()
                    .is_empty()
            {
                break;
            }
            match self.source.as_bytes()[pos] {
                b'\\' => {
                    pos = next_char(self.source, pos + 1, end);
                    continue;
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(pos + 1);
                    }
                }
                _ => {}
            }
            pos = next_char(self.source, pos, end);
        }
        Ok(pos)
    }
    fn link(
        &mut self,
        start: usize,
        end: usize,
        code: &HashMap<usize, usize>,
    ) -> Result<Option<(usize, usize, String, String)>> {
        let mut pos = start + 1;
        let mut depth = 1;
        while pos < end {
            self.work(1)?;
            if let Some(stop) = code.get(&pos) {
                pos = *stop;
                continue;
            }
            let c = self.source.as_bytes()[pos];
            if c == b'\\' {
                pos = next_char(self.source, pos + 1, end);
                continue;
            }
            if c == b'[' {
                depth += 1;
            }
            if c == b']' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            pos = next_char(self.source, pos, end);
        }
        if pos >= end || !self.source[pos..end].starts_with("](") {
            return Ok(None);
        }
        let label_end = pos;
        pos += 2;
        let dest_start = pos;
        let mut nesting = 1;
        let mut quote = None;
        while pos < end {
            self.work(1)?;
            let c = self.source.as_bytes()[pos];
            if c == b'\\' {
                pos = next_char(self.source, pos + 1, end);
                continue;
            }
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if matches!(c, b'\'' | b'"') {
                quote = Some(c);
            } else if c == b'(' {
                nesting += 1;
            } else if c == b')' {
                nesting -= 1;
                if nesting == 0 {
                    break;
                }
            }
            pos = next_char(self.source, pos, end);
        }
        if pos == end || has_blank_line(&self.source[dest_start..pos]) {
            return Ok(None);
        }
        let raw = self.source[dest_start..pos].trim();
        let split = raw.find(char::is_whitespace).unwrap_or(raw.len());
        Ok(Some((
            label_end,
            pos + 1,
            raw[..split].trim_matches(['<', '>']).to_owned(),
            raw[split..].trim().trim_matches(['\'', '"']).to_owned(),
        )))
    }
}

impl ExprParser<'_, '_> {
    fn rest_is_string(&self) -> bool {
        self.engine.source[self.pos..self.end].starts_with(['\'', '"'])
    }
}

fn declaration_start(rest: &str) -> bool {
    rest.strip_prefix("export")
        .filter(|s| s.starts_with([' ', '\t']))
        .is_some_and(|s| {
            let s = s.trim_start_matches([' ', '\t']);
            ["let", "const"].iter().any(|k| {
                s.strip_prefix(k)
                    .is_some_and(|tail| tail.starts_with([' ', '\t']))
            })
        })
}
fn declaration_continues(text: &str) -> bool {
    let mut depth = 0_i32;
    let mut tags = 0_i32;
    let mut header = false;
    let mut closing = false;
    let mut header_braces = 0_i32;
    let mut quote = None;
    let mut escaped = false;
    let mut operand = true;
    let mut previous = '\0';
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
                operand = false;
            }
            continue;
        }
        if (header || tags == 0) && matches!(c, '\'' | '"') {
            quote = Some(c);
            continue;
        }
        if c == '<'
            && !header
            && (tags > 0 || operand)
            && chars
                .peek()
                .is_some_and(|c| c.is_ascii_uppercase() || *c == '/')
        {
            closing = chars.peek() == Some(&'/');
            tags += if closing { -1 } else { 1 };
            header = true;
            header_braces = 0;
        } else if header {
            if c == '{' {
                header_braces += 1;
            }
            if c == '}' {
                header_braces -= 1;
            }
            if c == '>' && header_braces == 0 {
                if !closing && previous == '/' {
                    tags -= 1;
                }
                header = false;
                operand = false;
            }
        } else if tags == 0 {
            match c {
                '(' | '[' | '{' => {
                    depth += 1;
                    operand = true;
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    operand = false;
                }
                '+' | '-' | '*' | '/' | '%' | '?' | ':' | ',' | '=' | '!' | '<' | '>' | '&'
                | '|' => operand = true,
                c if !c.is_whitespace() => operand = false,
                _ => {}
            }
        }
        if !c.is_whitespace() {
            previous = c;
        }
    }
    depth > 0 || tags > 0 || header || operand
}
pub(crate) fn line_content_end(source: &str, pos: usize, end: usize) -> usize {
    source[pos..end].find(['\r', '\n']).map_or(end, |i| pos + i)
}
fn line_end(source: &str, pos: usize, end: usize) -> usize {
    let stop = line_content_end(source, pos, end);
    if stop == end {
        end
    } else if source[stop..end].starts_with("\r\n") {
        stop + 2
    } else {
        stop + 1
    }
}
fn at_line_start(source: &str, pos: usize) -> bool {
    pos == 0
        || (pos == 3 && source.starts_with('\u{feff}'))
        || source
            .as_bytes()
            .get(pos - 1)
            .is_some_and(|c| matches!(c, b'\n' | b'\r'))
}
fn block_position(source: &str, pos: usize) -> bool {
    let start = source[..pos]
        .rfind(['\r', '\n'])
        .map_or(if source.starts_with('\u{feff}') { 3 } else { 0 }, |p| {
            p + 1
        });
    let mut prefix = &source[start..pos];
    for _ in 0..=MARKDOWN_DEPTH {
        let trimmed = prefix.trim_start_matches(' ');
        if prefix.len() - trimmed.len() > 3 {
            return false;
        }
        prefix = trimmed;
        if prefix.is_empty() {
            return true;
        }
        if let Some(tail) = prefix.strip_prefix('>') {
            prefix = tail.strip_prefix(' ').unwrap_or(tail);
            continue;
        }
        let marker = if prefix.starts_with(['-', '+', '*']) {
            1
        } else {
            let digits = prefix.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || digits > 9 || !prefix[digits..].starts_with(['.', ')']) {
                return false;
            }
            digits + 1
        };
        if !prefix[marker..].starts_with([' ', '\t']) {
            return false;
        }
        prefix = prefix[marker..].trim_start_matches([' ', '\t']);
    }
    false
}
pub(crate) fn has_blank_line(text: &str) -> bool {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .split('\n')
        .skip(1)
        .take(normalized.split('\n').count().saturating_sub(2))
        .any(|line| line.trim().is_empty())
}
fn next_char(source: &str, pos: usize, end: usize) -> usize {
    pos + source[pos..end].chars().next().map_or(0, char::len_utf8)
}
fn close_tag(source: &str, start: usize, end: usize) -> Option<(String, usize)> {
    let tail = source.get(start + 2..end)?;
    if !tail.as_bytes().first()?.is_ascii_uppercase() {
        return None;
    }
    let len = tail.bytes().take_while(u8::is_ascii_alphanumeric).count();
    let rest = tail[len..].trim_start_matches(char::is_whitespace);
    if !rest.starts_with('>') {
        return None;
    }
    Some((tail[..len].to_owned(), end - rest.len() + 1))
}
fn fence(source: &str, pos: usize, end: usize) -> Option<(String, usize)> {
    if !at_line_start(source, pos) {
        return None;
    }
    let line_end_pos = line_content_end(source, pos, end);
    let line = &source[pos..line_end_pos];
    let trim = line.trim_start_matches(' ');
    if line.len() - trim.len() > 3 {
        return None;
    }
    let ch = *trim.as_bytes().first()?;
    if !matches!(ch, b'`' | b'~') {
        return None;
    }
    let count = trim.bytes().take_while(|b| *b == ch).count();
    if count < 3 || (ch == b'`' && trim[count..].contains('`')) {
        return None;
    }
    let language = trim[count..]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned();
    let mut next = line_end(source, pos, end);
    while next < end {
        let stop = line_content_end(source, next, end);
        let line = &source[next..stop];
        let trim = line.trim_start_matches(' ');
        if line.len() - trim.len() <= 3 {
            let run = trim.bytes().take_while(|b| *b == ch).count();
            if run >= count && trim[run..].trim().is_empty() {
                return Some((language, line_end(source, next, end)));
            }
        }
        next = line_end(source, next, end);
    }
    Some((language, end))
}
fn code_text(source: &str) -> String {
    let run = source.bytes().take_while(|b| *b == b'`').count();
    let text = source[run..source.len() - run]
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ");
    if text.starts_with(' ') && text.ends_with(' ') && text.chars().any(|c| c != ' ') {
        text[1..text.len() - 1].to_owned()
    } else {
        text
    }
}
fn thematic(source: &str, pos: usize, end: usize) -> Option<usize> {
    if !at_line_start(source, pos) {
        return None;
    }
    let stop = line_content_end(source, pos, end);
    let line = &source[pos..stop];
    let trim = line.trim_start_matches(' ');
    if line.len() - trim.len() > 3 {
        return None;
    }
    let mark = *trim.as_bytes().first()?;
    if !matches!(mark, b'-' | b'*' | b'_')
        || trim.bytes().filter(|b| *b == mark).count() < 3
        || !trim.bytes().all(|b| b == mark || matches!(b, b' ' | b'\t'))
    {
        return None;
    }
    Some(stop)
}
#[derive(Default)]
struct Contexts {
    code: HashMap<usize, usize>,
    fences: HashMap<usize, (String, usize, String)>,
    links: HashMap<usize, (String, String)>,
}
fn markdown_contexts(source: &str, start: usize, end: usize) -> Contexts {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
    let masked = crate::markdown::context_source(&source[start..end]);
    let mut contexts = Contexts::default();
    let mut current: Option<(usize, String, usize, String)> = None;
    for (event, range) in Parser::new_ext(
        &masked,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    )
    .into_offset_iter()
    {
        match event {
            Event::Code(_) => {
                contexts.code.insert(start + range.start, start + range.end);
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                current = Some((
                    start + range.start,
                    info.split_whitespace().next().unwrap_or("").to_owned(),
                    start + range.end,
                    String::new(),
                ));
            }
            Event::Text(text) if current.is_some() => {
                if let Some((_, _, _, content)) = &mut current {
                    content.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((pos, language, stop, text)) = current.take() {
                    contexts.fences.insert(
                        pos,
                        (
                            language,
                            stop,
                            text.replace('\u{1}', "<")
                                .replace('\u{2}', " ")
                                .replace('\u{3}', "=")
                                .replace('\u{4}', "["),
                        ),
                    );
                }
            }
            Event::Start(
                Tag::Link {
                    dest_url, title, ..
                }
                | Tag::Image {
                    dest_url, title, ..
                },
            ) => {
                contexts.links.insert(
                    start + range.start,
                    (dest_url.into_string(), title.into_string()),
                );
            }
            _ => {}
        }
    }
    contexts
}
fn fence_content(source: &str, start: usize, end: usize) -> String {
    let first = line_end(source, start, end);
    let tail = source[first..end].trim_end_matches(['\r', '\n']);
    let last = tail.rfind(['\r', '\n']).map_or(first, |i| first + i + 1);
    let candidate = source[last..end].trim();
    let content_end = if candidate.bytes().all(|b| matches!(b, b'`' | b'~')) {
        last
    } else {
        end
    };
    source[first..content_end]
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

pub fn parse(source: &str, mode: Mode) -> Document {
    let cap = if mode == Mode::Reply {
        REPLY_BYTES
    } else {
        PROGRAM_BYTES
    };
    let mut end = source.len().min(cap);
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let mut engine = Engine {
        source,
        mode,
        nodes: 0,
        blocks: 0,
        tags: 0,
        declarations: 0,
        names: HashSet::new(),
        remaining_work: end.saturating_mul(128).max(1024),
        diagnostics: vec![],
        suppressed: 0,
        fatal: false,
        body_depth: 0,
        expression_frames: 0,
        structural_failures: 0,
    };
    let mut fallback = mode == Mode::Program && source.len() > cap;
    if fallback {
        engine.report("E_SOURCE_TOO_LARGE", Span::new(0, end));
    }
    let start = if source.starts_with('\u{feff}') {
        3.min(end)
    } else {
        0
    };
    let mut nodes = if fallback {
        vec![]
    } else {
        match engine.body(start, end, 0, false, true, false) {
            Ok((nodes, _, _)) => nodes,
            Err(code) => {
                engine.report(code, Span::new(start, end));
                fallback = true;
                vec![]
            }
        }
    };
    if engine.fatal {
        fallback = true;
    }
    if mode == Mode::Reply && text_scalars(&nodes) > OUTPUT_SCALARS {
        engine.report("E_LIMIT_TEXT", Span::new(start, end));
        fallback = true;
    }
    if fallback {
        nodes = vec![Node {
            span: Span::new(start, end),
            kind: Kind::Literal(
                source[start..end]
                    .chars()
                    .take(OUTPUT_SCALARS)
                    .map(|c| if c == '\0' { '\u{fffd}' } else { c })
                    .collect(),
            ),
            children: vec![],
        }];
    }
    let mut doc = Document {
        mode,
        nodes,
        diagnostics: engine.diagnostics,
        suppressed_diagnostics: engine.suppressed,
        parsed_bytes: end,
        shortened: source.len() > cap,
        fallback,
    };
    if doc.references().len() > REFERENCES {
        add_diagnostic(&mut doc, "E_REFERENCE_LIMIT", Span::new(start, end));
    }
    if count_handles(&doc.nodes) > HANDLES {
        add_diagnostic(&mut doc, "E_REFERENCE_LIMIT", Span::new(start, end));
    }
    doc
}
fn add_diagnostic(doc: &mut Document, code: &'static str, span: Span) {
    if doc.diagnostics.len() < DIAGNOSTICS {
        doc.diagnostics.push(Diagnostic { code, span });
    } else {
        doc.suppressed_diagnostics += 1;
    }
}
fn count_handles(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| {
            usize::from(matches!(n.kind, Kind::Handle { .. }))
                + count_handles(&n.children)
                + match &n.kind {
                    Kind::Tag { props, .. } => props
                        .iter()
                        .map(|p| count_handles(std::slice::from_ref(&p.value)))
                        .sum(),
                    _ => 0,
                }
        })
        .sum()
}
fn text_scalars(nodes: &[Node]) -> usize {
    nodes.iter().map(|n| match &n.kind { Kind::Text(t) | Kind::Literal(t) => t.chars().count(), Kind::Markdown(Markdown::Code(t) | Markdown::CodeBlock { text: t, .. }) => t.chars().count(), _ => 0 } + text_scalars(&n.children)).sum()
}

/// Invalid UTF-8 is replaced before parsing; offsets refer to the repaired text.
pub fn parse_bytes(source: &[u8], mode: Mode) -> Document {
    let cap = if mode == Mode::Reply {
        REPLY_BYTES
    } else {
        PROGRAM_BYTES
    };
    // A scalar can straddle the source cap. Retain at most three lookahead
    // bytes so truncation never manufactures an encoding error for valid UTF-8.
    let prefix = &source[..source.len().min(cap + 3)];
    let repaired = String::from_utf8_lossy(prefix);
    let mut doc = parse(&repaired, mode);
    if matches!(repaired, std::borrow::Cow::Owned(_)) {
        let end = doc.parsed_bytes;
        add_diagnostic(&mut doc, "W_ENCODING", Span::new(0, end));
    }
    if source.len() > cap {
        doc.shortened = true;
        if mode == Mode::Program && !doc.fallback {
            doc.fallback = true;
            doc.nodes.clear();
            let end = doc.parsed_bytes;
            add_diagnostic(&mut doc, "E_SOURCE_TOO_LARGE", Span::new(0, end));
        }
    }
    doc
}

/// Collects a streamed source and parses it once, when `finish` is called.
/// Bytes beyond the mode's size limit (plus a three-byte margin) are counted
/// but not kept, and `finish` then marks the document shortened. No partial
/// parse is offered while bytes are still arriving.
pub struct StreamParser {
    mode: Mode,
    bytes: Vec<u8>,
    total: usize,
}
impl StreamParser {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            bytes: vec![],
            total: 0,
        }
    }
    pub fn push(&mut self, chunk: &[u8]) {
        let cap = if self.mode == Mode::Reply {
            REPLY_BYTES
        } else {
            PROGRAM_BYTES
        };
        self.total = self.total.saturating_add(chunk.len());
        let count = chunk.len().min(cap + 3 - self.bytes.len());
        self.bytes.extend_from_slice(&chunk[..count]);
    }
    pub fn finish(self) -> Document {
        let mut doc = parse_bytes(&self.bytes, self.mode);
        if self.total > self.bytes.len() {
            doc.shortened = true;
            if self.mode == Mode::Program && !doc.fallback {
                doc.fallback = true;
                doc.nodes.clear();
                let end = doc.parsed_bytes;
                add_diagnostic(&mut doc, "E_SOURCE_TOO_LARGE", Span::new(0, end));
            }
        }
        doc
    }
}
