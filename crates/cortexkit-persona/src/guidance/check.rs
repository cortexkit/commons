use std::collections::BTreeMap;

use minijinja::{
    machinery::{
        ast::{CallArg, Expr, Stmt},
        parse, WhitespaceConfig,
    },
    syntax::SyntaxConfig,
};

use super::{error, shape::Shape, FactsShape};
use crate::{PersonaError, RenderLimits};

pub(super) fn check(id: &str, text: &str, shape: &FactsShape) -> Result<(), PersonaError> {
    let Shape::Object(roots) = &shape.0 else {
        return Err(error(id, "facts shape must describe an object"));
    };
    // MiniJinja's nested undeclared-name analysis does not follow item paths
    // or aliases, and omits some expression fields. Walk the pinned AST so
    // unexecuted expressions receive the same checks as executed ones.
    #[allow(clippy::default_constructed_unit_structs)]
    let ast = parse(
        text,
        id,
        SyntaxConfig::default(),
        WhitespaceConfig::default(),
    )
    .map_err(|e| error(id, e.to_string()))?;
    Checker {
        id,
        roots,
        names: roots.clone(),
    }
    .stmt(&ast)
}

#[derive(Clone)]
struct Checker<'a> {
    id: &'a str,
    roots: &'a BTreeMap<String, Shape>,
    names: BTreeMap<String, Shape>,
}

impl Checker<'_> {
    fn fail(&self, message: impl Into<String>) -> PersonaError {
        error(self.id, message)
    }

    fn all(&mut self, statements: &[Stmt<'_>]) -> Result<(), PersonaError> {
        statements.iter().try_for_each(|stmt| self.stmt(stmt))
    }

    fn bind(&mut self, target: &Expr<'_>, shape: Shape) -> Result<(), PersonaError> {
        match target {
            Expr::Var(var) => {
                self.bind_name(var.id, shape)?;
            }
            Expr::List(list) => {
                for item in &list.items {
                    self.bind(item, Shape::Scalar)?;
                }
            }
            _ => return Err(self.fail("only simple local bindings can be checked")),
        }
        Ok(())
    }

    fn bind_name(&mut self, name: &str, shape: Shape) -> Result<(), PersonaError> {
        // Every template is also checked independently against the facts roots.
        // A changed root shape could otherwise hide an invalid path in an
        // included template, even if the includer never reads that path itself.
        if self.roots.get(name).is_some_and(|root| root != &shape) {
            return Err(self.fail(format!(
                "binding `{name}` changes a declared facts root's shape"
            )));
        }
        self.names.insert(name.into(), shape);
        Ok(())
    }

    fn stmt(&mut self, stmt: &Stmt<'_>) -> Result<(), PersonaError> {
        match stmt {
            Stmt::Template(node) => self.all(&node.children),
            Stmt::EmitRaw(_) => Ok(()),
            Stmt::EmitExpr(node) => self.expr(&node.expr).map(|_| ()),
            Stmt::ForLoop(node) => {
                if self.expr(&node.iter)? != Shape::List {
                    return Err(self.fail("only scalar-list iteration can be checked"));
                }
                let mut body = self.clone();
                body.bind(&node.target, Shape::Scalar)?;
                body.bind_name(
                    "loop",
                    Shape::Object(
                        [
                            "index",
                            "index0",
                            "revindex",
                            "revindex0",
                            "first",
                            "last",
                            "length",
                            "depth",
                            "depth0",
                            "previtem",
                            "nextitem",
                        ]
                        .into_iter()
                        .map(|name| (name.into(), Shape::Scalar))
                        .collect(),
                    ),
                )?;
                if let Some(expr) = &node.filter_expr {
                    body.expr(expr)?;
                }
                body.all(&node.body)?;
                self.clone().all(&node.else_body)
            }
            Stmt::IfCond(node) => {
                self.expr(&node.expr)?;
                let mut yes = self.clone();
                let mut no = self.clone();
                yes.all(&node.true_body)?;
                no.all(&node.false_body)?;
                // Existing aliases survive an if at runtime. Keep their shape
                // only when both branches agree; otherwise make subsequent
                // member access fail closed. Newly conditional names remain
                // unavailable after the branch.
                for (name, shape) in &mut self.names {
                    *shape = if yes.names[name] == no.names[name] {
                        yes.names[name].clone()
                    } else {
                        Shape::Scalar
                    };
                }
                Ok(())
            }
            Stmt::WithBlock(node) => {
                let mut body = self.clone();
                for (target, expr) in &node.assignments {
                    let shape = body.expr(expr)?;
                    body.bind(target, shape)?;
                }
                body.all(&node.body)
            }
            Stmt::Set(node) => {
                let shape = self.expr(&node.expr)?;
                self.bind(&node.target, shape)
            }
            Stmt::SetBlock(node) => {
                self.all(&node.body)?;
                if let Some(filter) = &node.filter {
                    self.expr(filter)?;
                }
                self.bind(&node.target, Shape::Scalar)
            }
            Stmt::AutoEscape(node) => {
                self.expr(&node.enabled)?;
                self.all(&node.body)
            }
            Stmt::FilterBlock(node) => {
                self.all(&node.body)?;
                self.expr(&node.filter).map(|_| ())
            }
            Stmt::Block(node) => self.clone().all(&node.body),
            Stmt::Extends(node) => self.expr(&node.name).map(|_| ()),
            Stmt::Include(node) => self.expr(&node.name).map(|_| ()),
            Stmt::Import(node) => {
                self.expr(&node.expr)?;
                self.bind(&node.name, Shape::Scalar)
            }
            Stmt::FromImport(node) => {
                self.expr(&node.expr)?;
                for (name, alias) in &node.names {
                    self.bind(alias.as_ref().unwrap_or(name), Shape::Scalar)?;
                }
                Ok(())
            }
            Stmt::Do(node) => self.call(&node.call.expr, &node.call.args).map(|_| ()),
            // Another crate in the same build can enable MiniJinja features
            // (Cargo unifies them), adding statement kinds this checker does
            // not model. Refuse those rather than silently skip their reads.
            #[allow(unreachable_patterns)]
            _ => Err(self.fail("statement cannot be statically checked")),
        }
    }

    fn args(&self, args: &[CallArg<'_>]) -> Result<(), PersonaError> {
        for arg in args {
            match arg {
                CallArg::Pos(expr)
                | CallArg::Kwarg(_, expr)
                | CallArg::PosSplat(expr)
                | CallArg::KwargSplat(expr) => {
                    self.expr(expr)?;
                }
            }
        }
        Ok(())
    }

    fn call(&self, expr: &Expr<'_>, args: &[CallArg<'_>]) -> Result<Shape, PersonaError> {
        self.expr(expr)?;
        self.args(args)?;
        // Only range has a statically known scalar-list return shape. Other
        // results are opaque: reading a member of them is refused below.
        if matches!(expr, Expr::Var(var) if var.id == "range" && !self.names.contains_key("range"))
        {
            Ok(Shape::List)
        } else {
            Ok(Shape::Scalar)
        }
    }

    fn member(&self, base: &Expr<'_>, name: &str) -> Result<Shape, PersonaError> {
        let Shape::Object(members) = self.expr(base)? else {
            return Err(self.fail(format!(
                "cannot read `{}`: base is not a declared object",
                path(base, name)
            )));
        };
        members
            .get(name)
            .cloned()
            .ok_or_else(|| self.fail(format!("unknown facts path `{}`", path(base, name))))
    }

    fn expr(&self, expr: &Expr<'_>) -> Result<Shape, PersonaError> {
        match expr {
            Expr::Var(var) => {
                if let Some(shape) = self.names.get(var.id) {
                    return Ok(shape.clone());
                }
                if crate::template::environment(RenderLimits::default())
                    .empty_state()
                    .lookup(var.id)
                    .is_some()
                {
                    return Ok(Shape::Scalar);
                }
                Err(self.fail(format!("unknown facts path `{}`", var.id)))
            }
            Expr::Const(_) => Ok(Shape::Scalar),
            Expr::GetAttr(node) => self.member(&node.expr, node.name),
            Expr::GetItem(node) => {
                let base = self.expr(&node.expr)?;
                self.expr(&node.subscript_expr)?;
                match base {
                    Shape::Object(_) => {
                        let Expr::Const(key) = &node.subscript_expr else {
                            return Err(self.fail("dynamic object member access cannot be checked"));
                        };
                        let Some(key) = key.value.as_str() else {
                            return Err(
                                self.fail("object item access requires a literal string key")
                            );
                        };
                        self.member(&node.expr, key)
                    }
                    Shape::List => Ok(Shape::Scalar),
                    Shape::Scalar => {
                        Err(self.fail("item access on a scalar or opaque value cannot be checked"))
                    }
                }
            }
            Expr::Slice(node) => {
                let shape = self.expr(&node.expr)?;
                for bound in [&node.start, &node.stop, &node.step].into_iter().flatten() {
                    self.expr(bound)?;
                }
                Ok(shape)
            }
            Expr::UnaryOp(node) => {
                self.expr(&node.expr)?;
                Ok(Shape::Scalar)
            }
            Expr::BinOp(node) => {
                self.expr(&node.left)?;
                self.expr(&node.right)?;
                Ok(Shape::Scalar)
            }
            Expr::IfExpr(node) => {
                self.expr(&node.test_expr)?;
                self.expr(&node.true_expr)?;
                if let Some(expr) = &node.false_expr {
                    self.expr(expr)?;
                }
                Ok(Shape::Scalar)
            }
            Expr::Filter(node) => {
                if let Some(expr) = &node.expr {
                    self.expr(expr)?;
                }
                self.args(&node.args)?;
                if matches!(
                    node.name,
                    "attr" | "map" | "selectattr" | "rejectattr" | "groupby"
                ) || node.args.iter().any(|arg| {
                    matches!(arg, CallArg::Kwarg("attribute", _) | CallArg::KwargSplat(_))
                }) {
                    return Err(self.fail(format!(
                        "attribute-indirection filter `{}` cannot be checked",
                        node.name
                    )));
                }
                Ok(Shape::Scalar)
            }
            Expr::Test(node) => {
                self.expr(&node.expr)?;
                self.args(&node.args)?;
                Ok(Shape::Scalar)
            }
            Expr::Call(node) => self.call(&node.expr, &node.args),
            Expr::List(node) => {
                for item in &node.items {
                    self.expr(item)?;
                }
                Ok(Shape::List)
            }
            Expr::Map(node) => {
                for expr in node.keys.iter().chain(&node.values) {
                    self.expr(expr)?;
                }
                // Object literals are opaque instead of inventing a shape for
                // dynamic keys or heterogeneous values.
                Ok(Shape::Scalar)
            }
        }
    }
}

fn path(base: &Expr<'_>, member: &str) -> String {
    match base {
        Expr::Var(var) => format!("{}.{member}", var.id),
        Expr::GetAttr(node) => format!("{}.{member}", path(&node.expr, node.name)),
        _ => format!("<expression>.{member}"),
    }
}
