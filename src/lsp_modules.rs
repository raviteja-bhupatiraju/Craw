//! Cross-file intelligence for the language server: resolving `import`s to
//! other `.craw` files (open buffers win over disk), following any name —
//! `from` imports, `a.b.name` chains, `*` imports, re-exports — to its
//! definition, diagnostics for broken imports, module completions, and
//! workspace-wide references and rename.
//!
//! Resolution is [`crate::modules`]'s, so the editor and the compiler
//! agree on what an import means. The one difference is the search roots:
//! the compiler knows the program's entry file, the editor doesn't, so it
//! searches the workspace folders and the file's own (non-package)
//! directory (see [`modules::editor_roots`]).

use crate::ast::Stmt;
use crate::ide::CrossFile;
use crate::lexer::Token;
use crate::lsp_analyzer::LspAnalyzer;
use crate::modules::{self, SourceLoader};
use crate::semantic::{RefKind, Resolution, SemanticModel, SymbolId, SymbolKind};
use crate::text::Span;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Diagnostic, DiagnosticSeverity, Location, NumberOrString, Range,
    TextEdit, Url, WorkspaceEdit,
};

/// How far a rename reaches.
#[derive(Debug)]
pub enum RenameScope {
    /// Everywhere the target is used, across the project.
    Project(Target),
    /// Just this file (a local, or an import alias).
    Local,
    /// Not renameable (a module name: rename its file instead).
    Refuse,
}

/// Workspace roots and a cache of on-disk module models.
#[derive(Debug, Default)]
pub struct Project {
    pub(crate) roots: Vec<PathBuf>,
    disk: RefCell<HashMap<PathBuf, Arc<SemanticModel>>>,
}

/// A symbol somewhere in the project (`symbol: None` = a whole module).
#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    pub model: Arc<SemanticModel>,
    pub symbol: Option<SymbolId>,
}

impl Target {
    pub fn uri(&self) -> Option<Url> {
        // A namespace package has no file: point at its directory.
        if modules::is_package(&self.path) && !self.path.exists() {
            return Url::from_directory_path(self.path.parent()?).ok();
        }
        Url::from_file_path(&self.path).ok()
    }

    pub fn location(&self) -> Option<Location> {
        let range = match self.symbol {
            Some(id) => self
                .model
                .line_index()
                .range(self.model.symbol(id).name_span),
            None => Range::default(),
        };
        Some(Location {
            uri: self.uri()?,
            range,
        })
    }

    fn same(&self, other: &Target) -> bool {
        self.path == other.path && self.symbol == other.symbol
    }
}

/// The file path an LSP URI names (canonical when it exists).
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let url = Url::parse(uri).ok()?;
    url.to_file_path().ok().map(|p| modules::normalize(&p))
}

/// Reads open buffers first, then disk.
struct ProjectLoader<'a>(&'a LspAnalyzer);

impl SourceLoader for ProjectLoader<'_> {
    fn load(&self, path: &Path) -> Option<String> {
        if let Some(m) = self.0.open_model_at(path) {
            return Some(m.text().to_string());
        }
        std::fs::read_to_string(path).ok()
    }
    fn exists(&self, path: &Path) -> bool {
        self.0.open_model_at(path).is_some() || path.is_file()
    }
}

/// Top-level symbol of `model` named `name`.
pub fn top_level(model: &SemanticModel, name: &str) -> Option<SymbolId> {
    model.scopes()[model.module_scope().0 as usize]
        .bindings
        .get(name)
        .copied()
}

/// The symbol an identifier token refers to: its resolved reference, or
/// (for tokens error recovery blanked out mid-edit, which have none) a
/// plain scope lookup of its name. Never a field access.
pub fn symbol_of_token(model: &SemanticModel, tok: usize) -> Option<SymbolId> {
    if let Some(r) = model.references().iter().find(|r| r.token == tok) {
        return match r.target {
            Resolution::Symbol(id) => Some(id),
            _ => None,
        };
    }
    let Token::Ident(name) = model.tokens().get(tok)? else {
        return None;
    };
    if tok > 0 && matches!(model.tokens()[tok - 1], Token::Dot) {
        return None;
    }
    model.lookup(name, model.scope_at(model.spans()[tok].start))
}

/// The identifier token covering byte `offset`.
pub fn ident_token_at(model: &SemanticModel, offset: usize) -> Option<usize> {
    model
        .tokens()
        .iter()
        .zip(model.spans())
        .position(|(t, s)| matches!(t, Token::Ident(_)) && s.start <= offset && offset < s.end)
}

/// Every import statement in `model` (any depth): `(first token, one past
/// its last token)`.
fn import_statements(model: &SemanticModel) -> Vec<(usize, usize)> {
    let tokens = model.tokens();
    let mut out = Vec::new();
    crate::ide::walk_stmts(model.ast(), &mut |stmt| {
        if let Stmt::FromImport(_, _, pos) | Stmt::Import(_, pos) = stmt {
            let end = (*pos..tokens.len())
                .find(|&i| matches!(tokens[i], Token::Newline | Token::Semicolon | Token::Eof))
                .unwrap_or(tokens.len());
            out.push((*pos, end));
        }
    });
    out
}

/// What an identifier token inside an import statement names.
#[derive(Debug, Clone)]
pub enum ImportPart {
    /// A module path segment: the module path up to and including it.
    Path(Vec<String>),
    /// The original name of a `from` item (`area` in `area as a`, or the
    /// binding itself in `from m import area`).
    Item { module: Vec<String>, name: String },
}

/// The module path written at tokens `from..to` of a `from` statement,
/// leading dots included (`..pkg.mod` → `[".", ".", "pkg", "mod"]`), up
/// to token `upto` (inclusive).
fn from_path(tokens: &[Token], from: usize, to: usize, upto: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen_ident = false;
    for tok in tokens.iter().take(to.min(upto + 1)).skip(from) {
        match tok {
            Token::Dot if !seen_ident => out.push(".".to_string()),
            Token::DotDot if !seen_ident => out.extend([".".to_string(), ".".to_string()]),
            Token::Ident(n) => {
                seen_ident = true;
                out.push(n.clone());
            }
            _ => {}
        }
    }
    out
}

/// Classifies the identifier token `tok` if it lies in an import statement.
pub fn import_part(model: &SemanticModel, tok: usize) -> Option<ImportPart> {
    let tokens = model.tokens();
    let (start, end) = import_statements(model)
        .into_iter()
        .find(|&(s, e)| s <= tok && tok < e)?;
    let Token::Ident(word) = &tokens[tok] else {
        return None;
    };
    let after_as = tok > 0 && matches!(tokens[tok - 1], Token::As);
    if after_as {
        return None;
    }
    if matches!(tokens[start], Token::From) {
        let import_kw = (start..end).find(|&i| matches!(tokens[i], Token::Import))?;
        if tok < import_kw {
            return Some(ImportPart::Path(from_path(
                tokens,
                start + 1,
                import_kw,
                tok,
            )));
        }
        return Some(ImportPart::Item {
            module: from_path(tokens, start + 1, import_kw, import_kw),
            name: word.clone(),
        });
    }
    // `import a.b, c as d`: the segment this token is in.
    let seg_start = (start + 1..=tok)
        .rev()
        .find(|&i| matches!(tokens[i - 1], Token::Comma | Token::Import))?;
    Some(ImportPart::Path(from_path(tokens, seg_start, end, tok)))
}

fn diag(
    model: &SemanticModel,
    span: Span,
    severity: DiagnosticSeverity,
    code: &str,
    message: String,
) -> Diagnostic {
    Diagnostic {
        range: model.line_index().range(span),
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_string())),
        source: Some("craw".to_string()),
        message,
        ..Default::default()
    }
}

fn error(model: &SemanticModel, span: Span, code: &str, message: String) -> Diagnostic {
    diag(model, span, DiagnosticSeverity::ERROR, code, message)
}

impl LspAnalyzer {
    pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
        self.project.roots = roots.iter().map(|r| modules::normalize(r)).collect();
    }

    /// A file changed (or was created/deleted) on disk.
    pub fn file_changed(&mut self, path: &Path) {
        self.project
            .disk
            .borrow_mut()
            .remove(&modules::normalize(path));
    }

    /// The model of an open document at `path`.
    pub(crate) fn open_model_at(&self, path: &Path) -> Option<Arc<SemanticModel>> {
        let want = modules::normalize(path);
        self.documents.iter().find_map(|(uri, d)| {
            (path_of(uri).as_deref() == Some(want.as_path())).then(|| d.model.clone())
        })
    }

    /// The model of `path`: its open buffer, else the file on disk (an
    /// empty model for a package directory without `__init__.craw`).
    pub(crate) fn model_at(&self, path: &Path) -> Option<Arc<SemanticModel>> {
        if let Some(m) = self.open_model_at(path) {
            return Some(m);
        }
        let key = modules::normalize(path);
        if let Some(m) = self.project.disk.borrow().get(&key) {
            return Some(m.clone());
        }
        let text = modules::module_source(&ProjectLoader(self), &key)?;
        let model = Arc::new(SemanticModel::build(&text));
        self.project.disk.borrow_mut().insert(key, model.clone());
        Some(model)
    }

    fn roots_for(&self, importer: &Path) -> Vec<PathBuf> {
        modules::editor_roots(importer, &self.project.roots, &ProjectLoader(self))
    }

    /// The module `module` names, as imported from the file `importer`.
    fn resolve_at(&self, importer: &Path, module: &[String]) -> Option<PathBuf> {
        modules::resolve_module(
            module,
            importer,
            &self.roots_for(importer),
            &ProjectLoader(self),
        )
    }

    /// The module `module` names, as imported from `importer_uri`.
    pub(crate) fn resolve_from(&self, importer_uri: &str, module: &[String]) -> Option<PathBuf> {
        self.resolve_at(&path_of(importer_uri)?, module)
    }

    fn module_target(&self, path: PathBuf) -> Option<Target> {
        Some(Target {
            model: self.model_at(&path)?,
            path,
            symbol: None,
        })
    }

    /// `name` in the module at `path`, followed through re-exports, `*`
    /// imports and package submodules to where it is actually defined.
    pub(crate) fn export_of(&self, path: &Path, name: &str, depth: u8) -> Option<Target> {
        if depth > 16 {
            return None;
        }
        let model = self.model_at(path)?;
        if let Some(id) = top_level(&model, name) {
            let sym = model.symbol(id);
            if sym.kind == SymbolKind::Import
                && let Some(src) = &sym.import
            {
                let next = self.resolve_at(path, &src.module);
                return match (&src.name, next) {
                    (Some(item), Some(next)) => {
                        self.export_of(&next, item, depth + 1).or_else(|| {
                            let sub = modules::submodule(&next, item, &ProjectLoader(self))?;
                            self.module_target(sub)
                        })
                    }
                    (None, Some(next)) => self.module_target(next),
                    // A native Rust import: the binding itself is all
                    // there is to point at.
                    (_, None) => Some(Target {
                        path: path.to_path_buf(),
                        model: model.clone(),
                        symbol: Some(id),
                    }),
                };
            }
            return Some(Target {
                path: path.to_path_buf(),
                model,
                symbol: Some(id),
            });
        }
        for star in self.star_modules(path, &model) {
            if let Some(m) = self.model_at(&star)
                && modules::star_names(&m).iter().any(|n| n == name)
            {
                return self.export_of(&star, name, depth + 1);
            }
        }
        let sub = modules::submodule(path, name, &ProjectLoader(self))?;
        self.module_target(sub)
    }

    /// The modules `model` (the file at `path`) imports with `*`.
    fn star_modules(&self, path: &Path, model: &SemanticModel) -> Vec<PathBuf> {
        model
            .ast()
            .iter()
            .filter_map(|s| match s {
                Stmt::FromImport(p, items, _) if items.iter().any(|(n, _)| n == "*") => {
                    self.resolve_at(path, p)
                }
                _ => None,
            })
            .collect()
    }

    /// Names this document gets from `from m import *`.
    pub(crate) fn star_provided(&self, uri: &str, model: &SemanticModel) -> HashSet<String> {
        let Some(path) = path_of(uri) else {
            return HashSet::new();
        };
        self.star_modules(&path, model)
            .into_iter()
            .filter_map(|m| self.model_at(&m))
            .flat_map(|m| modules::star_names(&m))
            .collect()
    }

    /// What an `Import` binding in `uri` refers to.
    pub(crate) fn import_target(
        &self,
        uri: &str,
        model: &SemanticModel,
        id: SymbolId,
    ) -> Option<Target> {
        let src = model.symbol(id).import.as_ref()?;
        let path = self.resolve_from(uri, &src.module)?;
        match &src.name {
            Some(name) => self.export_of(&path, name, 0).or_else(|| {
                let sub = modules::submodule(&path, name, &ProjectLoader(self))?;
                self.module_target(sub)
            }),
            None => self.module_target(path),
        }
    }

    /// If token `tok` names a module (an import binding, or a module
    /// reached through `a.b`), that module.
    pub(crate) fn module_alias_at(
        &self,
        uri: &str,
        model: &SemanticModel,
        tok: usize,
    ) -> Option<Target> {
        self.cross_target(uri, model, tok)
            .filter(|t| t.symbol.is_none())
    }

    /// For the identifier token `tok` in `x.name` where `x` is a module:
    /// `name` in that module.
    pub(crate) fn member_target(
        &self,
        uri: &str,
        model: &SemanticModel,
        tok: usize,
    ) -> Option<Target> {
        let tokens = model.tokens();
        let Token::Ident(name) = tokens.get(tok)? else {
            return None;
        };
        if tok < 2 || !matches!(tokens[tok - 1], Token::Dot) {
            return None;
        }
        let module = self.module_alias_at(uri, model, tok - 2)?;
        self.export_of(&module.path, name, 0)
    }

    /// What an identifier token inside an import statement names.
    pub(crate) fn import_part_target(
        &self,
        uri: &str,
        model: &SemanticModel,
        tok: usize,
    ) -> Option<Target> {
        match import_part(model, tok)? {
            ImportPart::Path(path) => self.module_target(self.resolve_from(uri, &path)?),
            ImportPart::Item { module, name } => {
                let file = self.resolve_from(uri, &module)?;
                self.export_of(&file, &name, 0)
            }
        }
    }

    /// The cross-file target of identifier token `tok`: a name in an import
    /// statement, `x.name` on a module, an import binding (or its use), or
    /// a name provided by `import *`.
    pub(crate) fn cross_target(
        &self,
        uri: &str,
        model: &SemanticModel,
        tok: usize,
    ) -> Option<Target> {
        if let Some(t) = self.import_part_target(uri, model, tok) {
            return Some(t);
        }
        let tokens = model.tokens();
        if tok >= 2 && matches!(tokens[tok - 1], Token::Dot) {
            return self.member_target(uri, model, tok);
        }
        match symbol_of_token(model, tok) {
            Some(id) if model.symbol(id).kind == SymbolKind::Import => {
                self.import_target(uri, model, id)
            }
            Some(_) => None,
            None => {
                let Token::Ident(name) = tokens.get(tok)? else {
                    return None;
                };
                let path = path_of(uri)?;
                for star in self.star_modules(&path, model) {
                    if let Some(t) = self.export_of(&star, name, 0) {
                        return Some(t);
                    }
                }
                None
            }
        }
    }

    /// Every module file `model`'s `import` statements load (each prefix of
    /// each path), for the "submodule isn't imported" check.
    fn imported_files(&self, path: &Path, model: &SemanticModel) -> HashSet<PathBuf> {
        let mut out = HashSet::new();
        for stmt in model.ast() {
            match stmt {
                Stmt::Import(list, _) => {
                    for (p, _) in list {
                        for n in 1..=p.len() {
                            if let Some(f) = self.resolve_at(path, &p[..n]) {
                                out.insert(f);
                            }
                        }
                    }
                }
                Stmt::FromImport(p, items, _) => {
                    if let Some(f) = self.resolve_at(path, p) {
                        for (item, _) in items {
                            if let Some(sub) = modules::submodule(&f, item, &ProjectLoader(self)) {
                                out.insert(sub);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Diagnostics for this document's imports:
    /// - `E0106` a module that can't be found,
    /// - `E0107` a name a module doesn't have (or an unimported submodule),
    /// - `E0108` a circular import,
    /// - `E0109` an import below the top level,
    /// - `E0110` a module used as a plain value,
    /// - `W0201` a native Rust import written as `from std... import`.
    pub(crate) fn import_diagnostics(&self, uri: &str, model: &SemanticModel) -> Vec<Diagnostic> {
        let Some(me) = path_of(uri) else {
            return Vec::new();
        };
        let tokens = model.tokens();
        let spans = model.spans();
        let mut out = Vec::new();

        let top: HashSet<usize> = model
            .ast()
            .iter()
            .filter_map(|s| match s {
                Stmt::FromImport(_, _, p) | Stmt::Import(_, p) => Some(*p),
                _ => None,
            })
            .collect();
        let statements = import_statements(model);
        let stmt_end = |pos: usize| {
            statements
                .iter()
                .find(|(s, _)| *s == pos)
                .map_or(pos + 1, |(_, e)| *e)
        };
        // The span of the tokens naming a path in `pos..end`.
        let path_span = |from: usize, to: usize| -> Span {
            let toks: Vec<usize> = (from..to)
                .filter(|&i| matches!(tokens[i], Token::Ident(_) | Token::Dot | Token::DotDot))
                .collect();
            match (toks.first(), toks.last()) {
                (Some(&a), Some(&b)) => Span::new(spans[a].start, spans[b].end),
                _ => spans[from.min(spans.len() - 1)],
            }
        };

        let mut stmts = Vec::new();
        crate::ide::walk_stmts(model.ast(), &mut |s| {
            if matches!(s, Stmt::FromImport(..) | Stmt::Import(..)) {
                stmts.push(s.clone());
            }
        });
        for stmt in &stmts {
            let pos = match stmt {
                Stmt::FromImport(_, _, p) | Stmt::Import(_, p) => *p,
                _ => continue,
            };
            let end = stmt_end(pos);
            if !top.contains(&pos) {
                out.push(error(
                    model,
                    spans[pos],
                    "E0109",
                    "imports must be at the top level of a file".to_string(),
                ));
            }
            match stmt {
                Stmt::FromImport(path, items, _) => {
                    let import_kw = (pos..end)
                        .find(|&i| matches!(tokens[i], Token::Import))
                        .unwrap_or(end);
                    let pspan = path_span(pos + 1, import_kw);
                    let Some(target) = self.resolve_at(&me, path) else {
                        if modules::is_native_path(path) {
                            out.push(diag(
                                model,
                                Span::new(
                                    spans[pos].start,
                                    spans[end.saturating_sub(1).max(pos)].end,
                                ),
                                DiagnosticSeverity::WARNING,
                                "W0201",
                                format!(
                                    "native Rust imports use Rust syntax: `{}`",
                                    rust_use(path, items)
                                ),
                            ));
                        } else {
                            out.push(error(
                                model,
                                pspan,
                                "E0106",
                                format!(
                                    "cannot find module `{}` (looked for `{}`)",
                                    modules::display_module(path),
                                    modules::module_relative_path(path).display()
                                ),
                            ));
                        }
                        continue;
                    };
                    if self.imports_reach(&target, &me, &mut Vec::new()) || target == me {
                        out.push(error(
                            model,
                            pspan,
                            "E0108",
                            format!(
                                "circular import: `{}` imports this file",
                                modules::display_module(path)
                            ),
                        ));
                    }
                    for (name, _) in items {
                        if name == "*" || self.export_of(&target, name, 0).is_some() {
                            continue;
                        }
                        let at = (import_kw..end)
                            .find(|&i| matches!(&tokens[i], Token::Ident(n) if n == name))
                            .map_or(pspan, |i| spans[i]);
                        out.push(error(
                            model,
                            at,
                            "E0107",
                            format!(
                                "module `{}` has no top-level `{name}`",
                                modules::module_name(&target)
                            ),
                        ));
                    }
                }
                Stmt::Import(list, _) => {
                    // One comma-separated segment per module.
                    let mut segs = Vec::new();
                    let mut a = pos + 1;
                    for (i, t) in tokens.iter().enumerate().take(end).skip(pos + 1) {
                        if matches!(t, Token::Comma) {
                            segs.push((a, i));
                            a = i + 1;
                        }
                    }
                    segs.push((a, end));
                    for ((path, _), (a, b)) in list.iter().zip(segs) {
                        let as_tok = (a..b)
                            .find(|&i| matches!(tokens[i], Token::As))
                            .unwrap_or(b);
                        let idents: Vec<usize> = (a..as_tok)
                            .filter(|&i| matches!(tokens[i], Token::Ident(_)))
                            .collect();
                        for n in 1..=path.len() {
                            if self.resolve_at(&me, &path[..n]).is_none() {
                                let last = idents.get(n - 1).copied().unwrap_or(a);
                                out.push(error(
                                    model,
                                    Span::new(spans[a].start, spans[last].end),
                                    "E0106",
                                    format!(
                                        "cannot find module `{}` (looked for `{}`)",
                                        path[..n].join("."),
                                        modules::module_relative_path(&path[..n]).display()
                                    ),
                                ));
                                break;
                            }
                        }
                        if let Some(t) = self.resolve_at(&me, path)
                            && (t == me || self.imports_reach(&t, &me, &mut Vec::new()))
                        {
                            out.push(error(
                                model,
                                path_span(a, as_tok),
                                "E0108",
                                format!("circular import: `{}` imports this file", path.join(".")),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }

        // `a.b.name` chains on module bindings, and a module used as a value.
        let imported = self.imported_files(&me, model);
        for r in model.references() {
            if r.kind == RefKind::Definition {
                continue;
            }
            let Resolution::Symbol(id) = r.target else {
                continue;
            };
            let sym = model.symbol(id);
            if sym.kind != SymbolKind::Import {
                continue;
            }
            let Some(mut current) = self
                .import_target(uri, model, id)
                .filter(|t| t.symbol.is_none())
            else {
                continue;
            };
            let mut tok = r.token;
            let mut dotted = r.name.clone();
            loop {
                let (Some(Token::Dot), Some(Token::Ident(n))) =
                    (tokens.get(tok + 1), tokens.get(tok + 2))
                else {
                    out.push(error(
                        model,
                        Span::new(r.span.start, spans[tok].end),
                        "E0110",
                        format!("module `{dotted}` can only be used as `{dotted}.name`"),
                    ));
                    break;
                };
                match self.export_of(&current.path, n, 0) {
                    Some(next) if next.symbol.is_none() => {
                        // A submodule reached by attribute must have been
                        // imported, as the compiler requires.
                        let via_name = current
                            .model
                            .scopes()
                            .get(current.model.module_scope().0 as usize)
                            .is_some_and(|s| s.bindings.contains_key(n));
                        if !via_name && !imported.contains(&next.path) {
                            out.push(error(
                                model,
                                spans[tok + 2],
                                "E0107",
                                format!("`{dotted}.{n}` is a module that isn't imported; add `import {dotted}.{n}`"),
                            ));
                            break;
                        }
                        current = next;
                        tok += 2;
                        dotted = format!("{dotted}.{n}");
                    }
                    Some(_) => break,
                    None => {
                        out.push(error(
                            model,
                            spans[tok + 2],
                            "E0107",
                            format!("module `{dotted}` has no top-level `{n}`"),
                        ));
                        break;
                    }
                }
            }
        }
        out
    }

    /// Code actions for `W0201`: rewrite `from std.x import a, b as c` as
    /// `use std::x::{a, b as c};`.
    pub(crate) fn native_import_actions(
        &self,
        uri: &str,
        model: &SemanticModel,
        diagnostics: &[Diagnostic],
    ) -> Vec<CodeAction> {
        let Ok(url) = Url::parse(uri) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for d in diagnostics {
            if d.code != Some(NumberOrString::String("W0201".into())) {
                continue;
            }
            let Some(offset) = model.line_index().offset(d.range.start.into()) else {
                continue;
            };
            let Some(Stmt::FromImport(path, items, _)) = model.ast().iter().find(
                |s| matches!(s, Stmt::FromImport(_, _, p) if model.spans()[*p].start == offset),
            ) else {
                continue;
            };
            let mut changes = HashMap::new();
            changes.insert(
                url.clone(),
                vec![TextEdit {
                    range: d.range,
                    new_text: rust_use(path, items),
                }],
            );
            out.push(CodeAction {
                title: "Use Rust `use` syntax".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![d.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            });
        }
        out
    }

    /// Whether `from`'s imports (transitively) reach `to`.
    fn imports_reach(&self, from: &Path, to: &Path, seen: &mut Vec<PathBuf>) -> bool {
        if seen.iter().any(|p| p == from) || seen.len() > 64 {
            return false;
        }
        seen.push(from.to_path_buf());
        let Some(model) = self.model_at(from) else {
            return false;
        };
        let mut targets = Vec::new();
        for stmt in model.ast() {
            match stmt {
                Stmt::FromImport(p, _, _) => targets.extend(self.resolve_at(from, p)),
                Stmt::Import(list, _) => {
                    for (p, _) in list {
                        targets.extend(self.resolve_at(from, p));
                    }
                }
                _ => {}
            }
        }
        targets
            .into_iter()
            .any(|next| next == to || self.imports_reach(&next, to, seen))
    }

    /// Every `.craw` file in the workspace roots plus every open document.
    pub(crate) fn project_files(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
            if depth > 16 {
                return;
            }
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name();
                if p.is_dir() {
                    if !matches!(
                        name.to_string_lossy().as_ref(),
                        "target" | ".craw" | "node_modules" | ".git"
                    ) {
                        walk(&p, out, depth + 1);
                    }
                } else if p.extension().is_some_and(|x| x == "craw") {
                    out.push(modules::normalize(&p));
                }
            }
        }
        let mut out = Vec::new();
        for root in &self.project.roots {
            walk(root, &mut out, 0);
        }
        for uri in self.documents.keys() {
            if let Some(p) = path_of(uri) {
                out.push(p);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Every occurrence, across the project, of the top-level symbol
    /// `target`: its own references, plus every token in any file that
    /// resolves to it by name (`from` items, uses of unaliased imports,
    /// `a.b.name` chains, `*`-imported uses). `for_rename` leaves out the
    /// uses of an alias (`a` in `from m import area as a`), which a rename
    /// must not touch; references include them.
    pub(crate) fn project_occurrences(&self, target: &Target, for_rename: bool) -> Vec<Location> {
        let Some(tid) = target.symbol else {
            return Vec::new();
        };
        let tname = target.model.symbol(tid).name.clone();
        let mut out: Vec<Location> = Vec::new();
        let push = |out: &mut Vec<Location>, path: &Path, model: &SemanticModel, span: Span| {
            if let Ok(uri) = Url::from_file_path(path) {
                out.push(Location {
                    uri,
                    range: model.line_index().range(span),
                });
            }
        };
        for r in target.model.references_to(tid) {
            push(&mut out, &target.path, &target.model, r.span);
        }
        for file in self.project_files() {
            let Some(model) = self.model_at(&file) else {
                continue;
            };
            let Ok(uri) = Url::from_file_path(&file) else {
                continue;
            };
            let uri = uri.to_string();
            for (i, t) in model.tokens().iter().enumerate() {
                if !matches!(t, Token::Ident(n) if *n == tname) {
                    continue;
                }
                if file == target.path && symbol_of_token(&model, i) == Some(tid) {
                    continue; // Already counted as a reference.
                }
                if self
                    .cross_target(&uri, &model, i)
                    .is_some_and(|t| t.same(target))
                {
                    push(&mut out, &file, &model, model.spans()[i]);
                }
            }
            if !for_rename {
                // Uses of aliases of the target.
                for sym in model.symbols() {
                    if sym.kind == SymbolKind::Import
                        && sym.name != tname
                        && self
                            .import_target(&uri, &model, sym.id)
                            .is_some_and(|t| t.same(target))
                    {
                        for r in model.references_to(sym.id) {
                            push(&mut out, &file, &model, r.span);
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            (a.uri.as_str(), a.range.start.line, a.range.start.character).cmp(&(
                b.uri.as_str(),
                b.range.start.line,
                b.range.start.character,
            ))
        });
        out.dedup();
        out
    }

    /// Rename edits for a project-wide rename, grouped by file.
    pub(crate) fn project_rename(
        &self,
        target: &Target,
        new_name: &str,
    ) -> HashMap<Url, Vec<TextEdit>> {
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        for loc in self.project_occurrences(target, true) {
            changes.entry(loc.uri).or_default().push(TextEdit {
                range: loc.range,
                new_text: new_name.to_string(),
            });
        }
        changes
    }

    /// Dotted module paths importable from `uri` by absolute import: every
    /// `.craw` file and package directory under its search roots.
    pub(crate) fn importable_modules(&self, uri: &str) -> Vec<String> {
        let Some(me) = path_of(uri) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for root in self.roots_for(&me) {
            for file in self.project_files() {
                if file == me {
                    continue;
                }
                let Ok(rel) = file.strip_prefix(&root) else {
                    continue;
                };
                let mut parts: Vec<String> = rel
                    .with_extension("")
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect();
                if parts.last().is_some_and(|p| p == "__init__") {
                    parts.pop();
                }
                // Every enclosing package is importable too.
                for n in 1..=parts.len() {
                    out.push(parts[..n].join("."));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Hover markdown for a cross-file target: the definition's own hover,
    /// plus which module it comes from.
    pub(crate) fn cross_hover(&self, t: &Target) -> String {
        let module = modules::module_name(&t.path);
        match t.symbol {
            Some(id) => {
                let sym = t.model.symbol(id);
                format!(
                    "{}\n\n*from `{module}`*",
                    crate::lsp_analyzer::render_symbol_hover(&t.model, sym, sym.name_span.start)
                )
            }
            None => {
                let kind = if modules::is_package(&t.path) {
                    "package"
                } else {
                    "module"
                };
                let mut s = format!(
                    "```craw\n{kind} {module}\n```\n\n`{}`",
                    modules::display_path(&t.path)
                );
                // A module's doc: its leading comment block.
                let doc: Vec<&str> = t
                    .model
                    .text()
                    .lines()
                    .take_while(|l| l.trim_start().starts_with('#'))
                    .map(|l| l.trim_start().trim_start_matches('#').trim())
                    .collect();
                if !doc.is_empty() {
                    s.push_str("\n\n---\n");
                    s.push_str(&doc.join("\n"));
                }
                s
            }
        }
    }

    /// The project-wide entity at `offset` in `uri`, for references and
    /// rename: an imported name (followed to its definition), a module
    /// member, or a top-level definition in this very file.
    pub(crate) fn project_target_at(&self, uri: &str, offset: usize) -> Option<Target> {
        let model = self.documents.get(uri)?.model.clone();
        let tok = ident_token_at(&model, offset)?;
        if let Some(t) = self.cross_target(uri, &model, tok) {
            return t.symbol.is_some().then_some(t);
        }
        let id = symbol_of_token(&model, tok)?;
        let sym = model.symbol(id);
        let top = sym.scope == model.module_scope() && sym.parent.is_none();
        if !top || matches!(sym.kind, SymbolKind::Import | SymbolKind::Parameter) {
            return None;
        }
        Some(Target {
            path: path_of(uri)?,
            model: model.clone(),
            symbol: Some(id),
        })
    }

    /// How far a rename at `offset` must reach.
    pub(crate) fn rename_scope(&self, uri: &str, offset: usize) -> RenameScope {
        let Some(model) = self.documents.get(uri).map(|d| d.model.clone()) else {
            return RenameScope::Refuse;
        };
        let Some(tok) = ident_token_at(&model, offset) else {
            return RenameScope::Local;
        };
        match import_part(&model, tok) {
            Some(ImportPart::Path(_)) => return RenameScope::Refuse,
            Some(ImportPart::Item { .. }) => {
                return match self.import_part_target(uri, &model, tok) {
                    Some(t) if t.symbol.is_some() => RenameScope::Project(t),
                    _ => RenameScope::Refuse,
                };
            }
            None => {}
        }
        if tok >= 2 && matches!(model.tokens()[tok - 1], Token::Dot) {
            return match self.member_target(uri, &model, tok) {
                Some(t) if t.symbol.is_some() => RenameScope::Project(t),
                Some(_) => RenameScope::Refuse,
                None => RenameScope::Local,
            };
        }
        if let Some(id) = symbol_of_token(&model, tok)
            && model.symbol(id).kind == SymbolKind::Import
        {
            let sym = model.symbol(id);
            let Some(src) = &sym.import else {
                return RenameScope::Local;
            };
            let aliased = match &src.name {
                Some(orig) => *orig != sym.name,
                None => src.module.len() > 1 || src.module[0] != sym.name,
            };
            if aliased {
                // Renaming an alias only touches this file.
                return RenameScope::Local;
            }
            return match self.import_target(uri, &model, id) {
                Some(t) if t.symbol.is_some() => RenameScope::Project(t),
                // A whole module (rename its file), or a native import.
                _ => RenameScope::Refuse,
            };
        }
        match self.project_target_at(uri, offset) {
            Some(t) => RenameScope::Project(t),
            None => RenameScope::Local,
        }
    }

    /// Completions inside an import statement (module paths, a module's
    /// names) or after `module.`. `None` when neither applies.
    pub(crate) fn module_completions(
        &self,
        uri: &str,
        model: &SemanticModel,
        offset: usize,
    ) -> Option<Vec<tower_lsp::lsp_types::CompletionItem>> {
        use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind};
        let tokens = model.tokens();
        let ctx = LspAnalyzer::context_token_before(model, offset)?;
        // The logical line so far (statements being typed rarely parse, so
        // this works from tokens alone).
        let line_start = (0..=ctx)
            .rev()
            .find(|&i| matches!(tokens[i], Token::Newline | Token::Indent | Token::Dedent))
            .map_or(0, |i| i + 1);
        let module_items = |names: Vec<String>| -> Vec<CompletionItem> {
            let mut names = names;
            names.sort();
            names.dedup();
            names
                .into_iter()
                .map(|label| CompletionItem {
                    label,
                    kind: Some(CompletionItemKind::MODULE),
                    ..Default::default()
                })
                .collect()
        };
        // Absolute module paths continuing `prefix`.
        let absolute = |prefix: &[String]| -> Vec<String> {
            self.importable_modules(uri)
                .into_iter()
                .chain(
                    crate::lsp_analyzer::IMPORT_PATHS
                        .iter()
                        .map(|s| s.to_string()),
                )
                .filter_map(|m| {
                    let parts: Vec<&str> = m.split('.').collect();
                    (parts.len() > prefix.len()
                        && parts.iter().zip(prefix).all(|(a, b)| *a == b.as_str()))
                    .then(|| parts[prefix.len()..].join("."))
                })
                .collect()
        };
        // Modules inside the package/directory `dir`.
        let children = |dir: &Path| -> Vec<String> {
            let me = path_of(uri);
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    if p.is_dir() && !name.starts_with('.') && name != "target" {
                        return Some(name);
                    }
                    let stem = name.strip_suffix(".craw")?;
                    (stem != "__init__" && me.as_deref() != Some(modules::normalize(&p).as_path()))
                        .then(|| stem.to_string())
                })
                .collect()
        };
        let export_items = |path: &Path| -> Vec<CompletionItem> {
            let Some(m) = self.model_at(path) else {
                return Vec::new();
            };
            let scope = m.module_scope();
            let mut items: Vec<CompletionItem> = m
                .symbols()
                .iter()
                .filter(|s| s.scope == scope && s.parent.is_none())
                .map(|s| {
                    let def = self.export_of(path, &s.name, 0);
                    let (kind, detail, docs) = match &def {
                        Some(t) if t.symbol.is_some() => {
                            let d = t.model.symbol(t.symbol.unwrap());
                            (
                                crate::lsp_analyzer::completion_kind(d.kind),
                                d.signature.as_ref().map(|sig| {
                                    crate::lsp_analyzer::signature_header(
                                        crate::lsp_analyzer::hover_prefix(d.kind),
                                        &d.name,
                                        sig,
                                    )
                                }),
                                d.docs.clone(),
                            )
                        }
                        Some(_) => (CompletionItemKind::MODULE, None, None),
                        None => (crate::lsp_analyzer::completion_kind(s.kind), None, None),
                    };
                    CompletionItem {
                        label: s.name.clone(),
                        kind: Some(kind),
                        detail,
                        documentation: docs.map(tower_lsp::lsp_types::Documentation::String),
                        sort_text: Some(format!("0{}", s.name)),
                        ..Default::default()
                    }
                })
                .collect();
            // A package's submodules.
            if modules::is_package(path)
                && let Some(dir) = path.parent()
            {
                items.extend(module_items(children(dir)).into_iter().map(|mut i| {
                    i.sort_text = Some(format!("1{}", i.label));
                    i
                }));
            }
            items.sort_by(|a, b| a.label.cmp(&b.label));
            items.dedup_by(|a, b| a.label == b.label);
            items
        };

        match tokens.get(line_start) {
            Some(Token::From) => {
                let import_kw = (line_start..=ctx).find(|&i| matches!(tokens[i], Token::Import));
                match import_kw {
                    // `from m import a, |`
                    Some(kw) => {
                        if matches!(tokens[ctx], Token::As) {
                            return Some(Vec::new());
                        }
                        let path = from_path(tokens, line_start + 1, kw, kw);
                        let file = self.resolve_from(uri, &path)?;
                        Some(export_items(&file))
                    }
                    // `from |`, `from a.|`, `from .|`, `from ..pkg.|`
                    None => {
                        let written = from_path(tokens, line_start + 1, ctx + 1, ctx);
                        let (level, rest) = modules::relative_level(&written);
                        let ends_in_dot = matches!(tokens[ctx], Token::Dot | Token::DotDot);
                        if level > 0 {
                            // Relative: list the directory it names.
                            let mut dir = path_of(uri)?.parent()?.to_path_buf();
                            for _ in 1..level {
                                dir = dir.parent()?.to_path_buf();
                            }
                            let rest: Vec<String> = if ends_in_dot {
                                rest.to_vec()
                            } else {
                                rest[..rest.len().saturating_sub(1)].to_vec()
                            };
                            for seg in &rest {
                                dir = dir.join(seg);
                            }
                            return Some(module_items(children(&dir)));
                        }
                        let prefix = if ends_in_dot { written } else { Vec::new() };
                        Some(module_items(absolute(&prefix)))
                    }
                }
            }
            Some(Token::Import) => {
                // `import a, |` / `import a.|`, never after `as`.
                let seg_start = (line_start + 1..=ctx)
                    .rev()
                    .find(|&i| matches!(tokens[i - 1], Token::Comma | Token::Import))
                    .unwrap_or(ctx + 1);
                if (seg_start..=ctx).any(|i| matches!(tokens[i], Token::As)) {
                    return Some(Vec::new());
                }
                let prefix = if matches!(tokens[ctx], Token::Dot) {
                    from_path(tokens, seg_start, ctx + 1, ctx)
                } else {
                    Vec::new()
                };
                Some(module_items(absolute(&prefix)))
            }
            _ => {
                // `module.|`
                if !matches!(tokens[ctx], Token::Dot) || ctx == 0 {
                    return None;
                }
                let module = self.module_alias_at(uri, model, ctx - 1)?;
                Some(export_items(&module.path))
            }
        }
    }

    /// A resolver for the single-file `ide` features, bound to `uri`.
    pub(crate) fn cross_file<'a>(
        &'a self,
        uri: &'a str,
        model: &'a SemanticModel,
    ) -> impl CrossFile + 'a {
        Resolver {
            analyzer: self,
            uri,
            model,
        }
    }
}

/// `from std.x import a, b as c` in Rust syntax: `use std::x::{a, b as c};`.
pub fn rust_use(path: &[String], items: &[(String, Option<String>)]) -> String {
    let rendered: Vec<String> = items
        .iter()
        .map(|(n, a)| match a {
            Some(a) => format!("{n} as {a}"),
            None => n.clone(),
        })
        .collect();
    let base = path.join("::");
    match rendered.as_slice() {
        [one] if one == "*" => format!("use {base}::*;"),
        [one] => format!("use {base}::{one};"),
        _ => format!("use {base}::{{{}}};", rendered.join(", ")),
    }
}

struct Resolver<'a> {
    analyzer: &'a LspAnalyzer,
    uri: &'a str,
    model: &'a SemanticModel,
}

impl CrossFile for Resolver<'_> {
    fn target_kind(&self, tok: usize) -> Option<(SymbolKind, bool)> {
        let t = self.analyzer.cross_target(self.uri, self.model, tok)?;
        Some(match t.symbol {
            Some(id) => (t.model.symbol(id).kind, false),
            None => (SymbolKind::Import, true),
        })
    }

    fn target_signature(&self, tok: usize) -> Option<crate::semantic::Signature> {
        let t = self.analyzer.cross_target(self.uri, self.model, tok)?;
        t.model.symbol(t.symbol?).signature.clone()
    }
}
