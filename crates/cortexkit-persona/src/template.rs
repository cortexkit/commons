//! The sealed template environment and the static checks every template
//! passes before it is rendered.

use std::collections::BTreeSet;

use minijinja::{
    machinery::{
        ast::{Expr, Stmt},
        parse, WhitespaceConfig,
    },
    syntax::SyntaxConfig,
    AutoEscape, Environment, UndefinedBehavior,
};

use crate::{PersonaError, SourceRef};

/// The most template instructions one render may execute.
pub const FUEL_LIMIT: u64 = 1_000_000;

/// MiniJinja's recursion budget for one render. An `include` costs ten; each
/// nested loop, `with` block or block call costs one; other statements are
/// free.
pub const RECURSION_LIMIT: usize = 100;

/// A MiniJinja environment with the built-in filters and tests only: strict
/// undefined, fuel and recursion limits, no auto-escaping, and no loader, so
/// only templates the engine adds can be reached.
pub(crate) fn environment<'s>() -> Environment<'s> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_fuel(Some(FUEL_LIMIT));
    env.set_recursion_limit(RECURSION_LIMIT);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env
}

/// What a template refers to and defines.
#[derive(Debug, Default)]
pub(crate) struct Analysis {
    /// Every `include`, `import`, `from ... import` and `extends` target.
    pub(crate) references: BTreeSet<String>,
    /// The template's own `extends` target, if any.
    pub(crate) extends: Option<String>,
    /// Every `{% block %}` the template defines.
    pub(crate) blocks: BTreeSet<String>,
}

/// Parse `text` and check it: every reference is a literal id in `declared`,
/// `ignore missing` is refused, and when `inherits` is set (the body extends a
/// lower body) the top level holds only blocks, whitespace, comments and the
/// two statements that neither print nor run another template (`set`, `do`),
/// and no `extends` of its own.
pub(crate) fn analyse(
    text: &str,
    origin: &SourceRef,
    declared: &BTreeSet<String>,
    inherits: bool,
) -> Result<Analysis, PersonaError> {
    // `SyntaxConfig` is a unit struct only while the `custom_syntax` feature is
    // off; `default()` keeps this compiling if another crate turns it on.
    #[allow(clippy::default_constructed_unit_structs)]
    let ast = parse(
        text,
        "<persona>",
        SyntaxConfig::default(),
        WhitespaceConfig::default(),
    )
    .map_err(|e| PersonaError::TemplateSyntax {
        origin: origin.clone(),
        line: e.line().unwrap_or(0),
        message: e.detail().unwrap_or("invalid syntax").to_string(),
    })?;
    let mut walker = Walker {
        origin,
        declared,
        inherits,
        out: Analysis::default(),
    };
    walker.stmt(&ast, true)?;
    Ok(walker.out)
}

struct Walker<'a> {
    origin: &'a SourceRef,
    declared: &'a BTreeSet<String>,
    inherits: bool,
    out: Analysis,
}

impl Walker<'_> {
    fn reference(
        &mut self,
        expr: &Expr<'_>,
        line: usize,
        statement: &'static str,
    ) -> Result<String, PersonaError> {
        let target = match expr {
            Expr::Const(c) => c.value.as_str().map(str::to_string),
            _ => None,
        };
        let Some(target) = target else {
            return Err(PersonaError::NonLiteralReference {
                origin: self.origin.clone(),
                line,
                statement,
            });
        };
        if !self.declared.contains(&target) {
            return Err(PersonaError::UndeclaredReference {
                origin: self.origin.clone(),
                line,
                statement,
                target,
            });
        }
        self.out.references.insert(target.clone());
        Ok(target)
    }

    fn outside_blocks(&self, top: bool, line: usize) -> Result<(), PersonaError> {
        if top && self.inherits {
            return Err(PersonaError::ContentOutsideBlocks {
                origin: self.origin.clone(),
                line,
            });
        }
        Ok(())
    }

    fn all(&mut self, stmts: &[Stmt<'_>], top: bool) -> Result<(), PersonaError> {
        stmts.iter().try_for_each(|s| self.stmt(s, top))
    }

    fn stmt(&mut self, stmt: &Stmt<'_>, top: bool) -> Result<(), PersonaError> {
        match stmt {
            Stmt::Template(t) => self.all(&t.children, true),
            Stmt::EmitRaw(r) => {
                if !r.raw.trim().is_empty() {
                    self.outside_blocks(top, line(r.span()))?;
                }
                Ok(())
            }
            Stmt::EmitExpr(e) => self.outside_blocks(top, line(e.span())),
            Stmt::ForLoop(f) => {
                self.outside_blocks(top, line(f.span()))?;
                self.all(&f.body, false)?;
                self.all(&f.else_body, false)
            }
            Stmt::IfCond(i) => {
                self.outside_blocks(top, line(i.span()))?;
                self.all(&i.true_body, false)?;
                self.all(&i.false_body, false)
            }
            Stmt::WithBlock(w) => {
                self.outside_blocks(top, line(w.span()))?;
                self.all(&w.body, false)
            }
            Stmt::SetBlock(s) => {
                self.outside_blocks(top, line(s.span()))?;
                self.all(&s.body, false)
            }
            Stmt::AutoEscape(a) => {
                self.outside_blocks(top, line(a.span()))?;
                self.all(&a.body, false)
            }
            Stmt::FilterBlock(f) => {
                self.outside_blocks(top, line(f.span()))?;
                self.all(&f.body, false)
            }
            Stmt::Set(_) | Stmt::Do(_) => Ok(()),
            Stmt::Block(b) => {
                self.out.blocks.insert(b.name.to_string());
                self.all(&b.body, false)
            }
            Stmt::Extends(e) => {
                let at = line(e.span());
                if self.inherits {
                    return Err(PersonaError::ExtendsInInheritingBody {
                        origin: self.origin.clone(),
                        line: at,
                    });
                }
                let target = self.reference(&e.name, at, "extends")?;
                self.out.extends = Some(target);
                Ok(())
            }
            Stmt::Include(i) => {
                let at = line(i.span());
                // An include prints, so in an inheriting body it belongs in a block.
                self.outside_blocks(top, at)?;
                if i.ignore_missing {
                    return Err(PersonaError::IgnoreMissing {
                        origin: self.origin.clone(),
                        line: at,
                    });
                }
                self.reference(&i.name, at, "include").map(|_| ())
            }
            // An import runs another template; in an inheriting body that
            // belongs inside a block, like every other statement that does
            // more than bind a name.
            Stmt::Import(i) => {
                let at = line(i.span());
                self.outside_blocks(top, at)?;
                self.reference(&i.expr, at, "import").map(|_| ())
            }
            Stmt::FromImport(f) => {
                let at = line(f.span());
                self.outside_blocks(top, at)?;
                self.reference(&f.expr, at, "import").map(|_| ())
            }
            // Statements that only exist when another crate in the build turns
            // on more MiniJinja features (macros, loop controls). They could
            // hide references this walker does not inspect, so they are refused.
            #[allow(unreachable_patterns)]
            _ => Err(PersonaError::TemplateSyntax {
                origin: self.origin.clone(),
                line: 0,
                message: "statement not supported by persona/v1".into(),
            }),
        }
    }
}

fn line(span: minijinja::machinery::Span) -> usize {
    span.start_line as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> SourceRef {
        SourceRef::Supplied {
            fragment: "t/t@1".into(),
        }
    }

    fn declared() -> BTreeSet<String> {
        ["frame", "note"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn references_are_collected() {
        let a = analyse(
            "{% extends \"frame\" %}{% block a %}{% include \"note\" %}{% endblock %}",
            &origin(),
            &declared(),
            false,
        )
        .unwrap();
        assert_eq!(a.extends.as_deref(), Some("frame"));
        assert!(a.references.contains("note"));
        assert!(a.blocks.contains("a"));
    }

    #[test]
    fn non_literal_and_list_targets_are_refused() {
        for text in [
            "{% include name %}",
            "{% include [\"note\"] %}",
            "{% include \"no\" ~ \"te\" %}",
            "{% import name as x %}",
        ] {
            assert!(
                matches!(
                    analyse(text, &origin(), &declared(), false),
                    Err(PersonaError::NonLiteralReference { .. })
                ),
                "{text}"
            );
        }
    }

    #[test]
    fn inheriting_body_allows_only_blocks_at_top_level() {
        assert!(analyse(
            "\n{% set x = 1 %}\n{% block a %}text{% endblock %}\n",
            &origin(),
            &declared(),
            true
        )
        .is_ok());
        for text in [
            "loose text",
            "{{ x }}",
            "{% if x %}{% endif %}",
            "{% include \"note\" %}",
            "{% import \"note\" as n %}",
            "{% from \"note\" import x %}",
        ] {
            assert!(
                matches!(
                    analyse(text, &origin(), &declared(), true),
                    Err(PersonaError::ContentOutsideBlocks { .. })
                ),
                "{text}"
            );
        }
    }
}
