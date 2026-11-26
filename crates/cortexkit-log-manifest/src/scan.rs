use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use proc_macro2::{Span, TokenStream, TokenTree};
use serde::Deserialize;
use syn::ext::IdentExt;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Item, ItemMod, Lit, Macro, Meta, Stmt};

use crate::event::{self, Event};
use crate::Diagnostic;

#[derive(Default)]
pub(crate) struct Scan {
    pub(crate) sites: Vec<Site>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

pub(crate) struct Site {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) target: String,
    pub(crate) text: String,
    pub(crate) fields: BTreeSet<String>,
    _level: event::Level,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    manifest_path: PathBuf,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

pub(crate) fn package(crate_dir: &Path, scan: &mut Scan) -> Result<(), String> {
    let manifest = crate_dir.join("Cargo.toml");
    let manifest = manifest
        .canonicalize()
        .map_err(|err| format!("{}: {err}", manifest.display()))?;
    let root = parent(&manifest);
    let output = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(root)
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .map_err(|err| format!("cargo metadata: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    let metadata: Metadata = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("invalid cargo metadata: {err}"))?;
    let package = metadata
        .packages
        .into_iter()
        .find(|package| package.manifest_path.canonicalize().as_ref().ok() == Some(&manifest))
        .ok_or_else(|| format!("{} does not describe a Cargo package", manifest.display()))?;
    let mut targets: Vec<_> = package
        .targets
        .into_iter()
        .filter(|target| {
            target.kind.iter().any(|kind| {
                matches!(
                    kind.as_str(),
                    "bin" | "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro"
                )
            })
        })
        .collect();
    targets.sort_by(|a, b| (&a.src_path, &a.name).cmp(&(&b.src_path, &b.name)));
    let mut walker = Walker {
        root,
        scan,
        ancestry: Vec::new(),
    };
    for target in targets {
        let module = target.name.replace('-', "_");
        walker.file(&target.src_path, parent(&target.src_path), &module)?;
    }
    Ok(())
}

struct Walker<'a> {
    root: &'a Path,
    scan: &'a mut Scan,
    // The same file can be part of multiple targets, or used by multiple mods.
    // Only reject a file if recurring in its own ancestry would loop forever.
    ancestry: Vec<PathBuf>,
}

impl Walker<'_> {
    fn file(&mut self, path: &Path, child_dir: &Path, module: &str) -> Result<(), String> {
        if excluded(self.root, path) {
            return Ok(());
        }
        let canonical = path
            .canonicalize()
            .map_err(|err| format!("{}: {err}", path.display()))?;
        if excluded(self.root, &canonical) {
            return Ok(());
        }
        let loc = path
            .strip_prefix(self.root)
            .or_else(|_| canonical.strip_prefix(self.root))
            .unwrap_or(path)
            .to_owned();
        if self.ancestry.contains(&canonical) {
            return Err(format!("{}: recursive module source", loc.display()));
        }
        let src = fs::read_to_string(path).map_err(|err| format!("{}: {err}", loc.display()))?;
        let syntax = syn::parse_file(&src)
            .map_err(|err| format!("{}:{}: {err}", loc.display(), err.span().start().line))?;
        if test_only(&syntax.attrs) {
            return Ok(());
        }
        self.ancestry.push(canonical);
        let mut reader = Reader {
            walker: self,
            path: &loc,
            child_dir,
            attr_dir: parent(path),
            module,
            error: None,
        };
        reader.visit_file(&syntax);
        let result = reader.error.map_or(Ok(()), Err);
        self.ancestry.pop();
        result
    }
}

struct Reader<'a, 'b> {
    walker: &'a mut Walker<'b>,
    path: &'a Path,
    child_dir: &'a Path,
    attr_dir: &'a Path,
    module: &'a str,
    error: Option<String>,
}

impl Reader<'_, '_> {
    fn read_mod(&mut self, item: &ItemMod) -> Result<(), String> {
        let name = item.ident.unraw().to_string();
        let module = format!("{}::{name}", self.module);
        let path = path_attr(&item.attrs);
        if let Some((_, items)) = &item.content {
            // An explicit path starts at the containing source directory, not
            // the implicit child directory of a non-mod.rs source file.
            let dir = match path {
                Some(path) => self.attr_dir.join(path),
                None => self.child_dir.join(&name),
            };
            let mut reader = Reader {
                walker: self.walker,
                path: self.path,
                child_dir: &dir,
                attr_dir: &dir,
                module: &module,
                error: None,
            };
            for item in items {
                reader.visit_item(item);
            }
            return reader.error.map_or(Ok(()), Err);
        }
        if let Some(path) = path {
            let path = self.attr_dir.join(path);
            return self.walker.file(&path, parent(&path), &module);
        }
        let dir = self.child_dir.join(&name);
        let flat = self.child_dir.join(format!("{name}.rs"));
        let nested = dir.join("mod.rs");
        match (flat.is_file(), nested.is_file()) {
            (true, false) => self.walker.file(&flat, &dir, &module),
            (false, true) => self.walker.file(&nested, &dir, &module),
            (flat, _) => {
                let reason = if flat {
                    "has two source files"
                } else {
                    "has no source file"
                };
                Err(format!(
                    "{}:{}: module {name} {reason}",
                    self.path.display(),
                    item.ident.span().start().line
                ))
            }
        }
    }

    fn skip(&mut self, span: Span, reason: impl Into<String>) {
        let loc = span.start();
        self.walker.scan.diagnostics.push(Diagnostic {
            path: self.path.to_owned(),
            line: loc.line,
            reason: reason.into(),
            column: loc.column,
        });
    }

    fn skip_nested(&mut self, tokens: TokenStream, reason: &'static str) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (i, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Ident(name)
                    if event::Macro::named(name).is_some()
                        && punct(&tokens, i + 1, '!')
                        && recognized_qualifier(&tokens[..i]) =>
                {
                    self.skip(name.span(), reason);
                }
                TokenTree::Group(group) => self.skip_nested(group.stream(), reason),
                _ => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for Reader<'_, '_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if self.error.is_none() && !test_only(item_attrs(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        self.error = self.read_mod(item).err();
    }

    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        let attrs = match stmt {
            Stmt::Macro(mac) => mac.attrs.as_slice(),
            Stmt::Local(local) => local.attrs.as_slice(),
            _ => &[],
        };
        if self.error.is_none() && !test_only(attrs) {
            visit::visit_stmt(self, stmt);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        use syn::ImplItem;
        let attrs = match item {
            ImplItem::Const(item) => item.attrs.as_slice(),
            ImplItem::Fn(item) => item.attrs.as_slice(),
            ImplItem::Macro(item) => item.attrs.as_slice(),
            ImplItem::Type(item) => item.attrs.as_slice(),
            _ => &[],
        };
        if !test_only(attrs) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        use syn::TraitItem;
        let attrs = match item {
            TraitItem::Const(item) => item.attrs.as_slice(),
            TraitItem::Fn(item) => item.attrs.as_slice(),
            TraitItem::Macro(item) => item.attrs.as_slice(),
            TraitItem::Type(item) => item.attrs.as_slice(),
            _ => &[],
        };
        if !test_only(attrs) {
            visit::visit_trait_item(self, item);
        }
    }

    fn visit_expr_block(&mut self, expr: &'ast syn::ExprBlock) {
        if !test_only(&expr.attrs) {
            visit::visit_expr_block(self, expr);
        }
    }

    fn visit_expr_macro(&mut self, expr: &'ast syn::ExprMacro) {
        if !test_only(&expr.attrs) {
            visit::visit_expr_macro(self, expr);
        }
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        let mut segments = mac.path.segments.iter();
        let Some(first) = segments.next() else { return };
        if mac.path.is_ident("macro_rules") {
            self.skip_nested(mac.tokens.clone(), "inside a macro_rules! body");
            return;
        }
        let name = match segments.next() {
            None if mac.path.leading_colon.is_none() => Some(&first.ident),
            Some(second) if first.ident == "tracing" && segments.next().is_none() => {
                Some(&second.ident)
            }
            _ => None,
        };
        if let Some((name, kind)) = name.and_then(|name| Some((name, event::Macro::named(name)?))) {
            match kind.parse(mac.tokens.clone(), self.module) {
                Ok(Event {
                    target,
                    text,
                    fields,
                    level,
                }) => self.walker.scan.sites.push(Site {
                    path: self.path.to_owned(),
                    line: name.span().start().line,
                    target,
                    text,
                    fields,
                    _level: level,
                }),
                Err(err) => self.skip(name.span(), err.to_string()),
            }
        }
        self.skip_nested(mac.tokens.clone(), "inside another macro invocation");
    }
}

fn punct(tokens: &[TokenTree], i: usize, ch: char) -> bool {
    matches!(tokens.get(i), Some(TokenTree::Punct(mark)) if mark.as_char() == ch)
}

fn recognized_qualifier(prefix: &[TokenTree]) -> bool {
    match prefix {
        [rest @ .., TokenTree::Ident(name), TokenTree::Punct(a), TokenTree::Punct(b)]
            if a.as_char() == ':' && b.as_char() == ':' =>
        {
            name == "tracing"
                && !matches!(rest, [.., TokenTree::Ident(_), TokenTree::Punct(a), TokenTree::Punct(b)] if a.as_char() == ':' && b.as_char() == ':')
        }
        [.., TokenTree::Punct(a), TokenTree::Punct(b)]
            if a.as_char() == ':' && b.as_char() == ':' =>
        {
            false
        }
        _ => true,
    }
}

fn parent(path: &Path) -> &Path {
    path.parent().expect("Cargo returns absolute source paths")
}

fn excluded(root: &Path, path: &Path) -> bool {
    matches!(path.strip_prefix(root).ok().and_then(|path| path.components().next()),
        Some(Component::Normal(dir)) if ["tests", "benches", "examples"].iter().any(|name| dir == *name))
}

fn path_attr(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| match &attr.meta {
        Meta::NameValue(meta) if meta.path.is_ident("path") => match &meta.value {
            Expr::Lit(expr) => match &expr.lit {
                Lit::Str(path) => Some(path.value()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    })
}

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args_with(
                    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                )
                .is_ok_and(|args| {
                    args.len() == 1 && matches!(&args[0], Meta::Path(path) if path.is_ident("test"))
                })
    })
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}
