use std::collections::BTreeSet;

use cortexkit_doctor::{field_kind, Template};
use proc_macro2::{Ident, TokenStream};
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Parser};
use syn::{braced, Expr, Lit, LitStr, Token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn from_constant(expr: &Expr) -> Option<Self> {
        let Expr::Path(expr) = expr else {
            return None;
        };
        let mut segments = expr.path.segments.iter().rev();
        let level = segments.next()?;
        if segments.next()?.ident != "Level" {
            return None;
        }
        Some(match level.ident.to_string().as_str() {
            "TRACE" => Self::Trace,
            "DEBUG" => Self::Debug,
            "INFO" => Self::Info,
            "WARN" => Self::Warn,
            "ERROR" => Self::Error,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Macro {
    At(Level),
    Event,
}

impl Macro {
    pub(crate) fn named(ident: &Ident) -> Option<Self> {
        Some(match ident.to_string().as_str() {
            "trace" => Self::At(Level::Trace),
            "debug" => Self::At(Level::Debug),
            "info" => Self::At(Level::Info),
            "warn" => Self::At(Level::Warn),
            "error" => Self::At(Level::Error),
            "event" => Self::Event,
            _ => return None,
        })
    }

    pub(crate) fn parse(self, tokens: TokenStream, module: &str) -> syn::Result<Event> {
        (|input: ParseStream| parse(input, self, module)).parse2(tokens)
    }
}

pub(crate) struct Event {
    pub(crate) target: String,
    pub(crate) text: String,
    pub(crate) fields: BTreeSet<String>,
    // Levels are recognized per call; they are not part of a template's identity.
    pub(crate) level: Level,
}

fn parse(input: ParseStream, kind: Macro, module: &str) -> syn::Result<Event> {
    let mut target = None;
    while input.peek(Ident::peek_any) && input.peek2(Token![:]) && !input.peek2(Token![::]) {
        let key = input.call(Ident::parse_any)?;
        if key != "target" && key != "name" && key != "parent" {
            return Err(syn::Error::new(key.span(), "unsupported event prefix"));
        }
        input.parse::<Token![:]>()?;
        let value: Expr = input.parse()?;
        input.parse::<Token![,]>()?;
        if key == "target" {
            let error = || syn::Error::new(key.span(), "non-literal target");
            let Expr::Lit(value) = value else {
                return Err(error());
            };
            let Lit::Str(value) = value.lit else {
                return Err(error());
            };
            target = Some(value.value());
        }
    }
    let target = target.unwrap_or_else(|| module.to_owned());
    if !Template::valid_target(&target) {
        return Err(input.error("invalid target"));
    }

    let level = match kind {
        Macro::At(level) => level,
        Macro::Event => {
            let span = input.span();
            let expr: Expr = input.parse()?;
            let level = Level::from_constant(&expr)
                .ok_or_else(|| syn::Error::new(span, "non-constant event level"))?;
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
            level
        }
    };

    let mut fields = BTreeSet::new();
    if input.peek(syn::token::Brace) {
        let group;
        braced!(group in input);
        parse_fields(&group, &mut fields)?;
        if !group.is_empty() {
            return Err(group.error("expected event fields"));
        }
        if !input.is_empty() {
            input.parse::<Token![,]>()?;
        }
    } else {
        parse_fields(input, &mut fields)?;
    }

    if !is_message(input) {
        return Err(syn::Error::new(input.span(), "no literal message"));
    }
    let message: LitStr = input.parse()?;
    if !input.is_empty() {
        input.parse::<Token![,]>()?;
        if !input.is_empty() {
            return Err(input.error("message has format arguments"));
        }
    }
    let text = literal_text(&message.value()).ok_or_else(|| {
        syn::Error::new(message.span(), "message has format arguments or captures")
    })?;
    if text.starts_with('[') {
        return Err(syn::Error::new(message.span(), "message starts with ["));
    }
    if !Template::valid_text(&text) {
        return Err(syn::Error::new(message.span(), "invalid message"));
    }

    Ok(Event {
        target,
        text,
        fields,
        level,
    })
}

fn is_message(input: ParseStream) -> bool {
    input.peek(LitStr) && !input.peek2(Token![=])
}

fn parse_fields(input: ParseStream, fields: &mut BTreeSet<String>) -> syn::Result<()> {
    while !input.is_empty() && !is_message(input) {
        sigil(input)?;
        let name = if input.peek(LitStr) {
            input.parse::<LitStr>()?.value()
        } else {
            let mut name = input.call(Ident::parse_any)?.to_string();
            while input.peek(Token![.]) && !input.peek(Token![..]) {
                input.parse::<Token![.]>()?;
                name.push('.');
                name.push_str(&input.call(Ident::parse_any)?.to_string());
            }
            name
        };

        if name == "message" {
            return Err(input.error("explicit message field"));
        }
        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            sigil(input)?;
            input.parse::<Expr>()?;
        }
        if field_kind(&name).is_some() {
            fields.insert(name);
        }
        if !input.is_empty() {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(())
}

fn sigil(input: ParseStream) -> syn::Result<()> {
    if input.peek(Token![?]) {
        input.parse::<Token![?]>()?;
    } else if input.peek(Token![%]) {
        input.parse::<Token![%]>()?;
    }
    Ok(())
}

fn literal_text(value: &str) -> Option<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if matches!(ch, '{' | '}') && chars.next() != Some(ch) {
            return None;
        }
        decoded.push(ch);
    }
    Some(decoded)
}
