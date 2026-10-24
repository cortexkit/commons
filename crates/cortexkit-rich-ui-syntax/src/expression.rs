use crate::parser::Engine;
use crate::{
    reference_end, DiagnosticCode as C, Expression as E, Kind, Node, Span, EXPRESSION_DEPTH,
};

pub(crate) type Result<T> = std::result::Result<T, C>;

/// Words the grammar gives a meaning of its own, so they never name a value.
pub(crate) const KEYWORDS: [&str; 6] = ["true", "false", "null", "export", "let", "const"];

pub(crate) struct ExprParser<'a, 's> {
    pub engine: &'a mut Engine<'s>,
    pub pos: usize,
    pub end: usize,
    pub tag_depth: usize,
}

impl ExprParser<'_, '_> {
    fn rest(&self) -> &str {
        &self.engine.source[self.pos..self.end]
    }
    pub fn ws(&mut self) {
        while self.pos < self.end && self.engine.source.as_bytes()[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }
    pub fn eat(&mut self, text: &str) -> bool {
        self.ws();
        if self.rest().starts_with(text) {
            self.pos += text.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, text: &str) -> Result<()> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(C::Syntax)
        }
    }
    pub fn identifier(&mut self) -> Result<String> {
        let name = self.word()?;
        if KEYWORDS.contains(&name.as_str()) {
            return Err(C::Syntax);
        }
        Ok(name)
    }
    /// Reads an identifier-shaped word, keywords included.
    pub fn word(&mut self) -> Result<String> {
        self.ws();
        let start = self.pos;
        let bytes = self.engine.source.as_bytes();
        if self.pos >= self.end
            || !(bytes[self.pos].is_ascii_alphabetic() || bytes[self.pos] == b'_')
        {
            return Err(C::Syntax);
        }
        self.pos += 1;
        while self.pos < self.end
            && (bytes[self.pos].is_ascii_alphanumeric() || bytes[self.pos] == b'_')
        {
            self.pos += 1;
        }
        Ok(self.engine.source[start..self.pos].to_owned())
    }
    /// Reads a quoted string. A raw line break, or the end of the enclosing
    /// construct, before the closing quote is `E_UNTERMINATED_STRING`, spanning
    /// the opening quote up to that point; a bad escape is `E_BAD_ESCAPE`,
    /// spanning the escape as far as it was read.
    pub fn string(&mut self) -> Result<String> {
        self.ws();
        let open = self.pos;
        let quote = self.rest().chars().next().ok_or(C::Syntax)?;
        if quote != '\'' && quote != '"' {
            return Err(C::Syntax);
        }
        self.pos += 1;
        let mut value = String::new();
        while self.pos < self.end {
            self.engine.work(1)?;
            let at = self.pos;
            let c = self.rest().chars().next().ok_or(C::Syntax)?;
            if c == '\n' || c == '\r' {
                return Err(self.unterminated(open, at));
            }
            self.pos += c.len_utf8();
            if c == quote {
                return Ok(value);
            }
            if c != '\\' {
                value.push(if c == '\0' { '\u{fffd}' } else { c });
                continue;
            }
            let Some(c) = self.rest().chars().next() else {
                return Err(self.unterminated(open, self.pos));
            };
            if c == '\n' || c == '\r' {
                return Err(self.unterminated(open, self.pos));
            }
            self.pos += c.len_utf8();
            match c {
                '\'' | '"' | '\\' | '/' => value.push(c),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                'u' => {
                    if !self.rest().starts_with('{') {
                        return Err(self.bad_escape(at));
                    }
                    self.pos += 1;
                    let start = self.pos;
                    while self.pos < self.end
                        && self.engine.source.as_bytes()[self.pos].is_ascii_hexdigit()
                    {
                        self.pos += 1;
                    }
                    let digits = self.pos - start;
                    if !self.rest().starts_with('}') {
                        return Err(self.bad_escape(at));
                    }
                    self.pos += 1;
                    let scalar = (1..=6)
                        .contains(&digits)
                        .then(|| {
                            u32::from_str_radix(&self.engine.source[start..start + digits], 16)
                        })
                        .and_then(std::result::Result::ok)
                        .and_then(char::from_u32);
                    match scalar {
                        Some(scalar) => value.push(scalar),
                        None => return Err(self.bad_escape(at)),
                    }
                }
                _ => return Err(self.bad_escape(at)),
            }
        }
        Err(self.unterminated(open, self.pos))
    }
    fn unterminated(&mut self, open: usize, stop: usize) -> C {
        self.engine
            .fail(C::UnterminatedString, Some(Span::new(open, stop)), None)
    }
    fn bad_escape(&mut self, backslash: usize) -> C {
        self.engine
            .fail(C::BadEscape, Some(Span::new(backslash, self.pos)), None)
    }
    fn node(&mut self, start: usize, expression: E, children: Vec<Node>) -> Result<Node> {
        let node = self.engine.node(
            Span::new(start, self.pos),
            Kind::Expression(expression),
            children,
        )?;
        if height(&node) > EXPRESSION_DEPTH + 1 {
            return Err(C::Nesting);
        }
        Ok(node)
    }
    pub fn expression(&mut self, min: u8, depth: usize) -> Result<Node> {
        self.engine.expression_frames += 1;
        let result = if self.engine.expression_frames > EXPRESSION_DEPTH + 1 {
            Err(C::Nesting)
        } else {
            self.expression_inner(min, depth)
        };
        self.engine.expression_frames -= 1;
        result
    }
    fn expression_inner(&mut self, min: u8, depth: usize) -> Result<Node> {
        self.engine.work(1)?;
        if depth > EXPRESSION_DEPTH {
            return Err(C::Nesting);
        }
        self.ws();
        let start = self.pos;
        let mut left = if self.eat("!") {
            let child = self.expression(9, depth + 1)?;
            self.node(start, E::Unary("!".into()), vec![child])?
        } else if self.eat("-") {
            let child = self.expression(9, depth + 1)?;
            self.node(start, E::Unary("-".into()), vec![child])?
        } else {
            self.primary(depth)?
        };
        loop {
            self.ws();
            let op = [
                "??", "||", "&&", "==", "!=", "<=", ">=", "**", "<", ">", "+", "-", "*", "/", "%",
            ]
            .into_iter()
            .find(|op| self.rest().starts_with(op));
            let Some(op) = op else { break };
            let prec = match op {
                "??" => 1,
                "||" => 2,
                "&&" => 3,
                "==" | "!=" => 4,
                "<" | "<=" | ">" | ">=" => 5,
                "+" | "-" => 6,
                "*" | "/" | "%" => 7,
                "**" => 10,
                _ => unreachable!(),
            };
            if prec < min {
                break;
            }
            if let Kind::Expression(E::Binary(prev)) = &left.kind {
                if (prec == 4 && matches!(prev.as_str(), "==" | "!="))
                    || (prec == 5 && matches!(prev.as_str(), "<" | "<=" | ">" | ">="))
                {
                    return Err(C::NonAssoc);
                }
            }
            self.pos += op.len();
            let right = self.expression(if op == "**" { 9 } else { prec + 1 }, depth + 1)?;
            let mixes = |node: &Node| matches!(&node.kind, Kind::Expression(E::Binary(other)) if (op == "??" && matches!(other.as_str(), "||" | "&&")) || (matches!(op, "||" | "&&") && other == "??"));
            if mixes(&left) || mixes(&right) {
                return Err(C::MixedCoalesce);
            }
            left = self.node(start, E::Binary(op.to_owned()), vec![left, right])?;
        }
        if min == 0 && self.rest().starts_with('?') && !self.rest().starts_with("??") {
            self.pos += 1;
            let yes = self.expression(0, depth + 1)?;
            self.expect(":")?;
            let no = self.expression(0, depth + 1)?;
            left = self.node(start, E::Conditional, vec![left, yes, no])?;
        }
        Ok(left)
    }
    fn primary(&mut self, depth: usize) -> Result<Node> {
        if depth > EXPRESSION_DEPTH {
            return Err(C::Nesting);
        }
        self.ws();
        let start = self.pos;
        let first = self.rest().chars().next().ok_or(C::Syntax)?;
        let mut node = match first {
            '\'' | '"' => {
                let value = self.string()?;
                self.node(start, E::String(value), vec![])?
            }
            '0'..='9' => {
                let bytes = self.engine.source.as_bytes();
                while self.pos < self.end && bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                if self.pos < self.end && bytes[self.pos] == b'.' {
                    self.pos += 1;
                    let digits = self.pos;
                    while self.pos < self.end && bytes[self.pos].is_ascii_digit() {
                        self.pos += 1;
                    }
                    if self.pos == digits {
                        return Err(C::NumberLiteral);
                    }
                }
                if self.pos < self.end && matches!(bytes[self.pos], b'e' | b'E') {
                    self.pos += 1;
                    if self.pos < self.end && matches!(bytes[self.pos], b'+' | b'-') {
                        self.pos += 1;
                    }
                    let digits = self.pos;
                    while self.pos < self.end && bytes[self.pos].is_ascii_digit() {
                        self.pos += 1;
                    }
                    if self.pos == digits {
                        return Err(C::NumberLiteral);
                    }
                }
                let token = &self.engine.source[start..self.pos];
                if !valid_number(token) {
                    return Err(C::NumberLiteral);
                }
                self.node(start, E::Number(token.to_owned()), vec![])?
            }
            '$' => {
                if let Some(end) = reference_end(&self.engine.source[..self.end], start) {
                    self.pos = end;
                    self.engine.node(
                        Span::new(start, end),
                        Kind::Reference {
                            n: self.engine.source[start + 5..end - 1].to_owned(),
                        },
                        vec![],
                    )?
                } else {
                    self.pos += 1;
                    let name = self.identifier()?;
                    if self
                        .engine
                        .source
                        .as_bytes()
                        .get(start + 1)
                        .is_some_and(u8::is_ascii_whitespace)
                    {
                        return Err(C::Syntax);
                    }
                    self.node(start, E::State(name), vec![])?
                }
            }
            '(' => {
                self.pos += 1;
                let child = self.expression(0, depth + 1)?;
                self.expect(")")?;
                self.node(start, E::Group, vec![child])?
            }
            '[' => {
                self.pos += 1;
                let children = self.arguments("]", depth)?;
                self.node(start, E::List, children)?
            }
            '{' => {
                self.pos += 1;
                let mut keys = Vec::new();
                let mut values = Vec::new();
                let mut unique = std::collections::HashSet::new();
                if !self.eat("}") {
                    loop {
                        self.ws();
                        let key = if self.rest().starts_with(['\'', '"']) {
                            self.string()?
                        } else {
                            self.identifier()?
                        };
                        if !unique.insert(key.clone()) {
                            return Err(self.engine.fail(C::DuplicateKey, None, Some(key)));
                        }
                        keys.push(key);
                        self.expect(":")?;
                        values.push(self.expression(0, depth + 1)?);
                        if self.eat("}") {
                            break;
                        }
                        self.expect(",")?;
                        if self.eat("}") {
                            break;
                        }
                    }
                }
                self.node(start, E::Record(keys), values)?
            }
            '<' => {
                let (node, end, broken) =
                    self.engine
                        .tag(start, self.end, self.tag_depth + 1, false)?;
                if broken {
                    return Err(C::Syntax);
                }
                self.pos = end;
                node
            }
            _ => {
                let keyword = ["true", "false", "null"].into_iter().find(|k| {
                    self.rest().starts_with(k)
                        && self
                            .rest()
                            .as_bytes()
                            .get(k.len())
                            .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_')
                });
                if let Some(k) = keyword {
                    self.pos += k.len();
                    self.node(
                        start,
                        match k {
                            "true" => E::Boolean(true),
                            "false" => E::Boolean(false),
                            _ => E::Null,
                        },
                        vec![],
                    )?
                } else {
                    let name = self.identifier()?;
                    if self.eat("(") {
                        let children = self.arguments(")", depth)?;
                        if matches!(name.as_str(), "Artifact" | "File") {
                            let path = match children.as_slice() {
                                [Node {
                                    kind: Kind::Expression(E::String(path)),
                                    ..
                                }] => path,
                                _ => {
                                    return Err(self.engine.fail(
                                        C::LiteralRequired,
                                        None,
                                        Some(name),
                                    ))
                                }
                            };
                            self.engine.node(
                                Span::new(start, self.pos),
                                Kind::Handle {
                                    kind: name,
                                    path: path.clone(),
                                },
                                vec![],
                            )?
                        } else {
                            self.node(start, E::Call(name), children)?
                        }
                    } else {
                        self.node(start, E::Name(name), vec![])?
                    }
                }
            }
        };
        loop {
            if self.eat(".") {
                let member = self.identifier()?;
                node = self.node(start, E::Member(member), vec![node])?;
            } else if self.eat("[") {
                let index = self.expression(0, depth + 1)?;
                self.expect("]")?;
                node = self.node(start, E::Index, vec![node, index])?;
            } else {
                break;
            }
        }
        Ok(node)
    }
    fn arguments(&mut self, close: &str, depth: usize) -> Result<Vec<Node>> {
        let mut children = Vec::new();
        if !self.eat(close) {
            loop {
                children.push(self.expression(0, depth + 1)?);
                if self.eat(close) {
                    break;
                }
                self.expect(",")?;
                if self.eat(close) {
                    break;
                }
            }
        }
        Ok(children)
    }
}

fn height(node: &Node) -> usize {
    1 + node.children.iter().map(height).max().unwrap_or(0)
}

pub(crate) fn literal(node: &Node) -> bool {
    match &node.kind {
        Kind::Expression(E::Number(_) | E::String(_) | E::Boolean(_) | E::Null) => true,
        Kind::Expression(E::Unary(op)) if op == "-" => {
            matches!(node.children[0].kind, Kind::Expression(E::Number(_)))
        }
        Kind::Expression(E::List | E::Record(_)) => node.children.iter().all(literal),
        _ => false,
    }
}

pub(crate) fn reply_value(node: &Node) -> bool {
    if literal(node) {
        return true;
    }
    match &node.kind {
        Kind::Reference { .. } | Kind::Handle { .. } => true,
        Kind::Expression(E::List | E::Record(_)) => node.children.iter().all(reply_value),
        Kind::Expression(E::Call(name)) if name == "Parse" => {
            matches!(node.children.len(), 2 | 3)
                && matches!(node.children[0].kind, Kind::Reference { .. })
                && matches!(node.children[1].kind, Kind::Expression(E::String(_)))
                && (node.children.len() == 2
                    || (matches!(node.children[2].kind, Kind::Expression(E::Record(_)))
                        && parse_options(&node.children[2])))
        }
        _ => false,
    }
}

fn parse_options(node: &Node) -> bool {
    match node.kind {
        Kind::Expression(E::Record(_)) => node.children.iter().all(parse_options),
        Kind::Expression(E::List) => node
            .children
            .iter()
            .all(|n| literal(n) && !matches!(n.kind, Kind::Expression(E::List | E::Record(_)))),
        _ => literal(node),
    }
}

fn valid_number(token: &str) -> bool {
    let (mantissa, exponent) = token.split_once(['e', 'E']).unwrap_or((token, "0"));
    let Ok(exponent) = exponent.parse::<i32>() else {
        return false;
    };
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.len() > 1 && integer.starts_with('0') {
        return false;
    }
    let digits = format!("{integer}{fraction}");
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return true;
    }
    let trailing = significant.len() - significant.trim_end_matches('0').len();
    let scale = 18_i64 + i64::from(exponent) - fraction.len() as i64;
    scale + trailing as i64 >= 0 && significant.len() as i64 + scale <= 38
}
