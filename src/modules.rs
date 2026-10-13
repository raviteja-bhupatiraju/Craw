//! Craw's module system: resolving `import`s of other `.craw` files and
//! linking a program's modules into one source the transpiler and the
//! interpreter consume unchanged.
//!
//! # Semantics (Python's, except where noted)
//!
//! - `import a.b.c` runs `a`, `a.b` and `a.b.c` and binds `a`; names are
//!   used as `a.b.c.f()`. `import a.b as m` binds `m` to `a.b` itself.
//!   `import a, b` imports several.
//! - `from m import x, y as z` binds names; `from m import (x,\n y)` may
//!   span lines; `from m import *` binds every public name (`__all__` if
//!   the module defines it, else every name not starting with `_`).
//! - `from pkg import sub` binds a submodule when `pkg` has no top-level
//!   `sub`.
//! - Modules: `a/b.craw` is module `a.b`; a directory is a package, whose
//!   own code (if any) is `__init__.craw`.
//! - Absolute imports search the program root (the entry file's
//!   directory), then each `CRAWPATH` directory. Relative imports
//!   (`from . import x`, `from ..pkg import y`) search from the importing
//!   file's package.
//! - Every module runs once, before the first module that imports it.
//!   Circular imports are an error (Python allows partially-initialized
//!   modules; Craw doesn't).
//! - Each module has its own namespace, types included: two modules can
//!   both define `helper` or `Point`. Templates and macros are expanded
//!   textually before a program runs, so they are shared program-wide.
//! - `from std.collections import HashMap` (a `std`, `core` or `alloc`
//!   path) is still a native Rust `use`, for compatibility; `use
//!   std::collections::HashMap` is the preferred spelling. Any other
//!   import that names no `.craw` file is an error.
//! - Not Python: a module can't be stored in a variable (`m = geometry`),
//!   and imports must be at the top level of a file.
//!
//! # Mechanism
//!
//! Linking is scope-correct source rewriting driven by each file's
//! [`SemanticModel`] (the same resolution the editor's rename uses). A
//! module's top-level names get a `<module>__` prefix; `from` imports of
//! values become assignments, imported types and `a.b.name` chains are
//! rewritten to the prefixed names, and import statements are blanked in
//! place so line numbers are preserved. The rewritten files are
//! concatenated modules-first into one program, parsed once, so node ids
//! stay unique. At runtime a data value's printed type name drops the
//! module prefix (everything up to its last `__`).

use crate::ast::{Expr, Pattern, Stmt};
use crate::lexer::Token;
use crate::semantic::{RefKind, Resolution, SemanticModel, SymbolId, SymbolKind};
use crate::text::Span;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A package's own code.
pub const INIT: &str = "__init__.craw";

/// Reads source files, so tests and the editor can supply unsaved text.
pub trait SourceLoader {
    fn load(&self, path: &Path) -> Option<String>;
    /// Whether `path` exists as a file, without necessarily reading it.
    fn exists(&self, path: &Path) -> bool {
        self.load(path).is_some()
    }
    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

/// Reads from disk, except for paths whose (unsaved) text is overridden.
#[derive(Debug, Default)]
pub struct FsLoader {
    pub overrides: HashMap<PathBuf, String>,
}

impl SourceLoader for FsLoader {
    fn load(&self, path: &Path) -> Option<String> {
        if let Some(text) = self.overrides.get(&normalize(path)) {
            return Some(text.clone());
        }
        std::fs::read_to_string(path).ok()
    }
    fn exists(&self, path: &Path) -> bool {
        self.overrides.contains_key(&normalize(path)) || path.is_file()
    }
}

/// A file's identity: canonical when it exists on disk. A package's
/// `__init__.craw` may not exist (a namespace package); it is then its
/// directory's canonical path joined with `__init__.craw`.
pub fn normalize(path: &Path) -> PathBuf {
    if let Ok(p) = std::fs::canonicalize(path) {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) if !dir.as_os_str().is_empty() => std::fs::canonicalize(dir)
            .map(|d| d.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

/// Whether `path` is a package's `__init__.craw`.
pub fn is_package(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == INIT)
}

/// A module's source: its file, or empty for a package directory with no
/// `__init__.craw`.
pub fn module_source(loader: &dyn SourceLoader, path: &Path) -> Option<String> {
    loader.load(path).or_else(|| {
        (is_package(path) && path.parent().is_some_and(|d| loader.is_dir(d))).then(String::new)
    })
}

/// Whether a `from` path names Rust's standard library (a native `use`).
pub fn is_native_path(path: &[String]) -> bool {
    matches!(
        path.first().map(String::as_str),
        Some("std" | "core" | "alloc")
    )
}

/// `(number of leading dots, the rest)` of a module path.
pub fn relative_level(path: &[String]) -> (usize, &[String]) {
    let level = path.iter().take_while(|s| *s == ".").count();
    (level, &path[level..])
}

/// A module path as written: `..pkg.mod`, `a.b`.
pub fn display_module(path: &[String]) -> String {
    let (level, rest) = relative_level(path);
    format!("{}{}", ".".repeat(level), rest.join("."))
}

/// `["shapes", "circle"]` → `shapes/circle.craw`.
pub fn module_relative_path(module: &[String]) -> PathBuf {
    let (_, rest) = relative_level(module);
    let mut p: PathBuf = rest.iter().collect();
    p.set_extension("craw");
    p
}

/// The module `rest` names under directory `base`: `base/a/b.craw`, else
/// the package `base/a/b/` (its `__init__.craw`, which may not exist).
fn under(base: &Path, rest: &[String], loader: &dyn SourceLoader) -> Option<PathBuf> {
    if rest.is_empty() {
        return loader.is_dir(base).then(|| normalize(&base.join(INIT)));
    }
    let dir: PathBuf = rest.iter().fold(base.to_path_buf(), |p, s| p.join(s));
    let mut file = dir.clone();
    file.set_extension("craw");
    if loader.exists(&file) {
        return Some(normalize(&file));
    }
    let init = dir.join(INIT);
    if loader.exists(&init) || loader.is_dir(&dir) {
        return Some(normalize(&init));
    }
    None
}

/// The file a module path names, as imported from `importer`. Absolute
/// paths search `roots` in order; relative ones (leading `.`s) start from
/// the importer's package. `None` if there is no such module (or a
/// relative path climbs above the filesystem root).
pub fn resolve_module(
    module: &[String],
    importer: &Path,
    roots: &[PathBuf],
    loader: &dyn SourceLoader,
) -> Option<PathBuf> {
    let (level, rest) = relative_level(module);
    if level > 0 {
        let mut dir = importer.parent()?;
        for _ in 1..level {
            dir = dir.parent()?;
        }
        return under(dir, rest, loader);
    }
    roots.iter().find_map(|r| under(r, rest, loader))
}

/// Submodule `name` of the package whose `__init__.craw` is `package`.
pub fn submodule(package: &Path, name: &str, loader: &dyn SourceLoader) -> Option<PathBuf> {
    if !is_package(package) {
        return None;
    }
    under(package.parent()?, &[name.to_string()], loader)
}

/// The `CRAWPATH` directories (`:`-separated, `;` on Windows).
pub fn crawpath() -> Vec<PathBuf> {
    std::env::var_os("CRAWPATH")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Module search roots for a program whose entry file is `entry`.
pub fn program_roots(entry: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = entry.parent().map(normalize).into_iter().collect();
    roots.extend(crawpath().iter().map(|p| normalize(p)));
    roots
}

/// Module search roots the editor uses for `file`, which isn't known to be
/// any particular program's entry: every workspace folder, then the file's
/// own directory unless that directory is a package (a module inside a
/// package imports its siblings relatively), then `CRAWPATH`.
pub fn editor_roots(file: &Path, workspace: &[PathBuf], loader: &dyn SourceLoader) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = workspace.to_vec();
    if let Some(dir) = file.parent()
        && !loader.exists(&dir.join(INIT))
    {
        let dir = normalize(dir);
        if !roots.contains(&dir) {
            roots.push(dir);
        }
    }
    roots.extend(crawpath().iter().map(|p| normalize(p)));
    roots
}

/// The names `from m import *` binds: `__all__` when the module assigns a
/// list of strings to it, else every top-level name not starting with `_`.
pub fn star_names(model: &SemanticModel) -> Vec<String> {
    for stmt in model.ast() {
        if let Stmt::Assign(Pattern::Var(n, _), Expr::List(items), _) = stmt
            && n == "__all__"
        {
            return items
                .iter()
                .filter_map(|e| match e {
                    Expr::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
        }
    }
    let mut names: Vec<String> = model.scopes()[model.module_scope().0 as usize]
        .bindings
        .keys()
        .filter(|n| !n.starts_with('_'))
        .cloned()
        .collect();
    names.sort();
    names
}

/// A module's name in messages: its file stem (`geometry`), or a
/// package's directory name.
pub fn module_name(path: &Path) -> String {
    let p = if is_package(path) {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    p.file_stem()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// A problem found while linking, located in the file it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    pub file: PathBuf,
    /// 0-based line and UTF-16 column, like LSP positions.
    pub line: u32,
    pub character: u32,
    pub message: String,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: error: {}",
            self.file.display(),
            self.line + 1,
            self.character + 1,
            self.message
        )
    }
}

/// One file's lines within the linked source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub file: PathBuf,
    /// First line of this file in the linked source (0-based).
    pub start_line: u32,
    pub line_count: u32,
}

/// A linked program.
#[derive(Debug, Clone)]
pub struct Linked {
    pub source: String,
    /// Every file the program is made of, modules first, entry last
    /// (namespace packages, which have no file, excluded).
    pub files: Vec<PathBuf>,
    pub segments: Vec<Segment>,
}

impl Linked {
    /// The file and line (0-based) a line of the linked source came from.
    pub fn original_line(&self, line: u32) -> Option<(&Path, u32)> {
        self.segments
            .iter()
            .find(|s| s.start_line <= line && line < s.start_line + s.line_count)
            .map(|s| (s.file.as_path(), line - s.start_line))
    }
}

/// What a module-level name is, after linking.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Export {
    /// A function, generator or variable: its linked name, bound in an
    /// importer by assignment.
    Value(String),
    /// A `data`/`class`/`struct`/`trait`: its linked name, which importers
    /// reference directly (so patterns match it).
    Type(String),
    /// A template, macro or operator: global, under its own name.
    Macro(String),
    /// A module.
    Module(usize),
    /// Imported by the module from a native Rust `use`.
    Native(String),
}

#[derive(Debug, Clone)]
struct ModuleImport {
    path: Vec<String>,
    alias: Option<String>,
    /// The module for each prefix of `path` (`a`, `a.b`, `a.b.c`).
    chain: Vec<usize>,
}

#[derive(Debug, Clone)]
enum ImportKind {
    Modules(Vec<ModuleImport>),
    From {
        /// `None` for a native Rust `use` (or a module that failed to
        /// resolve, already reported).
        module: Option<usize>,
        items: Vec<(String, Option<String>)>,
        /// Items that name submodules rather than top-level names.
        submodules: HashMap<String, usize>,
    },
}

#[derive(Debug, Clone)]
struct Import {
    pos: usize,
    kind: ImportKind,
}

struct Module {
    path: PathBuf,
    model: SemanticModel,
    imports: Vec<Import>,
    /// Renaming prefix (`geometry__`); empty for the entry file.
    prefix: String,
    /// Whether this is a namespace package with no `__init__.craw`.
    virtual_package: bool,
}

impl Module {
    fn top_level(&self, name: &str) -> Option<SymbolId> {
        let module = self.model.module_scope();
        self.model.scopes()[module.0 as usize]
            .bindings
            .get(name)
            .copied()
    }
}

/// A binding for a whole module in an importer, with the submodules
/// imported beneath it (`import a.b` binds `a` with child `b`).
#[derive(Debug, Clone)]
struct Namespace {
    module: usize,
    children: BTreeMap<String, Namespace>,
}

impl Namespace {
    fn new(module: usize) -> Self {
        Namespace {
            module,
            children: BTreeMap::new(),
        }
    }

    fn merge(&mut self, chain: &[(String, usize)]) {
        if let Some(((name, m), rest)) = chain.split_first() {
            self.children
                .entry(name.clone())
                .or_insert_with(|| Namespace::new(*m))
                .merge(rest);
        }
    }
}

fn error_at(model: &SemanticModel, file: &Path, offset: usize, message: String) -> LinkError {
    let p = model.line_index().position(offset);
    LinkError {
        file: file.to_path_buf(),
        line: p.line,
        character: p.character,
        message,
    }
}

/// The token index one past the end of the statement starting at `pos`.
fn statement_end(tokens: &[Token], pos: usize) -> usize {
    (pos..tokens.len())
        .find(|&i| matches!(tokens[i], Token::Newline | Token::Semicolon | Token::Eof))
        .unwrap_or(tokens.len())
}

/// Token indices of every import statement nested below the top level.
fn nested_imports(stmts: &[Stmt], top: bool, out: &mut Vec<usize>) {
    for stmt in stmts {
        match stmt {
            Stmt::FromImport(_, _, pos) | Stmt::Import(_, pos) if !top => out.push(*pos),
            Stmt::FunctionDef { body, .. }
            | Stmt::While(_, body)
            | Stmt::MatchFor(_, _, body, _)
            | Stmt::ClassDef { body, .. }
            | Stmt::TraitDef(_, body, _)
            | Stmt::ImplBlock(_, _, body, _)
            | Stmt::TemplateDef(_, _, body, _)
            | Stmt::MacroDef { body, .. } => nested_imports(body, false, out),
            Stmt::If(_, a, b) => {
                nested_imports(a, false, out);
                nested_imports(b, false, out);
            }
            Stmt::Match(_, arms, _) => {
                for (_, _, b) in arms {
                    nested_imports(b, false, out);
                }
            }
            _ => {}
        }
    }
}

/// A short name for a file in messages: its file name (`pkg/` for a
/// package).
fn display_name(path: &Path) -> String {
    if is_package(path) {
        return format!("{}/", module_name(path));
    }
    path.file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

struct Linker<'a> {
    loader: &'a dyn SourceLoader,
    roots: Vec<PathBuf>,
    modules: Vec<Module>,
    index: HashMap<PathBuf, usize>,
    order: Vec<usize>,
    errors: Vec<LinkError>,
    prefixes: HashSet<String>,
}

impl Linker<'_> {
    fn prefix_for(&mut self, path: &Path) -> String {
        let stem: String = module_name(path)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let mut candidate = format!("{stem}__");
        let mut n = 2;
        while !self.prefixes.insert(candidate.clone()) {
            candidate = format!("{stem}{n}__");
            n += 1;
        }
        candidate
    }

    fn err(&mut self, m: usize, tok: usize, message: String) {
        let module = &self.modules[m];
        let offset = module.model.spans().get(tok).map_or(0, |s| s.start);
        let e = error_at(&module.model, &module.path, offset, message);
        self.errors.push(e);
    }

    /// Loads `target` as imported by module `m` at token `tok` (reporting a
    /// cycle there), returning its index.
    fn load_dep(
        &mut self,
        m: usize,
        tok: usize,
        target: &Path,
        stack: &mut Vec<PathBuf>,
    ) -> Option<usize> {
        if let Some(at) = stack.iter().position(|p| p == target) {
            let cycle: Vec<String> = stack[at..]
                .iter()
                .map(|p| display_name(p))
                .chain(std::iter::once(display_name(target)))
                .collect();
            self.err(m, tok, format!("circular import: {}", cycle.join(" → ")));
            return None;
        }
        self.load(target, false, stack)
    }

    /// Loads `path` and (depth-first) everything it imports, appending each
    /// module to `order` after its dependencies.
    fn load(&mut self, path: &Path, entry: bool, stack: &mut Vec<PathBuf>) -> Option<usize> {
        if let Some(&i) = self.index.get(path) {
            return Some(i);
        }
        let text = module_source(self.loader, path)?;
        let virtual_package = is_package(path) && !self.loader.exists(path);
        let model = SemanticModel::build(&text);
        for e in model.syntax_errors() {
            self.errors
                .push(error_at(&model, path, e.span.start, e.message.clone()));
        }
        let mut nested = Vec::new();
        nested_imports(model.ast(), true, &mut nested);
        for pos in nested {
            let offset = model.spans()[pos].start;
            self.errors.push(error_at(
                &model,
                path,
                offset,
                "imports must be at the top level of a file".to_string(),
            ));
        }
        let prefix = if entry {
            String::new()
        } else {
            self.prefix_for(path)
        };
        let m = self.modules.len();
        self.modules.push(Module {
            path: path.to_path_buf(),
            model,
            imports: Vec::new(),
            prefix,
            virtual_package,
        });
        self.index.insert(path.to_path_buf(), m);
        stack.push(path.to_path_buf());

        let stmts: Vec<Stmt> = self.modules[m]
            .model
            .ast()
            .iter()
            .filter(|s| matches!(s, Stmt::Import(..) | Stmt::FromImport(..)))
            .cloned()
            .collect();
        let mut imports = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::Import(list, pos) => {
                    let mut out = Vec::new();
                    for (mpath, alias) in list {
                        let mut chain = Vec::new();
                        for n in 1..=mpath.len() {
                            let prefix = &mpath[..n];
                            match resolve_module(prefix, path, &self.roots, self.loader) {
                                Some(target) => match self.load_dep(m, pos, &target, stack) {
                                    Some(i) => chain.push(i),
                                    None => break,
                                },
                                None => {
                                    self.err(
                                        m,
                                        pos,
                                        format!(
                                            "cannot find module `{}` (looked for `{}`)",
                                            prefix.join("."),
                                            module_relative_path(prefix).display()
                                        ),
                                    );
                                    break;
                                }
                            }
                        }
                        if chain.len() == mpath.len() {
                            out.push(ModuleImport {
                                path: mpath,
                                alias,
                                chain,
                            });
                        }
                    }
                    imports.push(Import {
                        pos,
                        kind: ImportKind::Modules(out),
                    });
                }
                Stmt::FromImport(fpath, items, pos) => {
                    let target = resolve_module(&fpath, path, &self.roots, self.loader);
                    let module = match &target {
                        Some(t) => self.load_dep(m, pos, t, stack),
                        None if is_native_path(&fpath) => None,
                        None => {
                            let (level, _) = relative_level(&fpath);
                            let hint = if level > 0 {
                                ""
                            } else {
                                " (for a Rust item, write `use crate_name::item`)"
                            };
                            self.err(
                                m,
                                pos,
                                format!("cannot find module `{}`{hint}", display_module(&fpath)),
                            );
                            None
                        }
                    };
                    let mut submodules = HashMap::new();
                    if let (Some(t), Some(ti)) = (&target, module) {
                        for (name, _) in &items {
                            if name != "*"
                                && self.modules[ti].top_level(name).is_none()
                                && let Some(sub) = submodule(t, name, self.loader)
                                && let Some(si) = self.load_dep(m, pos, &sub, stack)
                            {
                                submodules.insert(name.clone(), si);
                            }
                        }
                    }
                    imports.push(Import {
                        pos,
                        kind: ImportKind::From {
                            module,
                            items,
                            submodules,
                        },
                    });
                }
                _ => {}
            }
        }
        self.modules[m].imports = imports;
        stack.pop();
        self.order.push(m);
        Some(m)
    }

    /// What `name` in module `m` is after linking (following re-exports).
    fn export(&self, m: usize, name: &str, depth: u8) -> Option<Export> {
        if depth > 32 {
            return None;
        }
        let module = &self.modules[m];
        if let Some(id) = module.top_level(name) {
            let sym = module.model.symbol(id);
            return match sym.kind {
                SymbolKind::Import => self.binding_export(m, name, depth),
                SymbolKind::DataClass
                | SymbolKind::Class
                | SymbolKind::Struct
                | SymbolKind::Trait => Some(Export::Type(format!("{}{name}", module.prefix))),
                SymbolKind::Template | SymbolKind::Macro | SymbolKind::Operator => {
                    Some(Export::Macro(name.to_string()))
                }
                SymbolKind::Function | SymbolKind::Generator | SymbolKind::Variable => {
                    Some(Export::Value(format!("{}{name}", module.prefix)))
                }
                _ => None,
            };
        }
        // A name `m` got through `from x import *`.
        for imp in &module.imports {
            if let ImportKind::From {
                module: Some(t),
                items,
                ..
            } = &imp.kind
                && items.iter().any(|(n, _)| n == "*")
                && star_names(&self.modules[*t].model)
                    .iter()
                    .any(|n| n == name)
            {
                return self.export(*t, name, depth + 1);
            }
        }
        // A package's submodule, if some import loaded it.
        if is_package(&module.path)
            && let Some(sub) = submodule(&module.path, name, self.loader)
            && let Some(&i) = self.index.get(&sub)
        {
            return Some(Export::Module(i));
        }
        None
    }

    /// What the import binding `local` in module `m` refers to.
    fn binding_export(&self, m: usize, local: &str, depth: u8) -> Option<Export> {
        for imp in &self.modules[m].imports {
            match &imp.kind {
                ImportKind::Modules(list) => {
                    for mi in list {
                        match &mi.alias {
                            Some(a) if a == local => {
                                return mi.chain.last().map(|&i| Export::Module(i));
                            }
                            None if mi.path[0] == local => {
                                return Some(Export::Module(mi.chain[0]));
                            }
                            _ => {}
                        }
                    }
                }
                ImportKind::From {
                    module,
                    items,
                    submodules,
                } => {
                    for (orig, alias) in items {
                        if alias.as_deref().unwrap_or(orig) != local {
                            continue;
                        }
                        let Some(t) = module else {
                            return Some(Export::Native(orig.clone()));
                        };
                        if let Some(&s) = submodules.get(orig) {
                            return Some(Export::Module(s));
                        }
                        return self.export(*t, orig, depth + 1);
                    }
                }
            }
        }
        None
    }

    fn missing(&self, module: usize, name: &str) -> String {
        let path = &self.modules[module].path;
        let mname = module_name(path);
        if is_package(path) && submodule(path, name, self.loader).is_some() {
            format!("`{mname}.{name}` is a module that isn't imported; add `import {mname}.{name}`")
        } else {
            format!("module `{mname}` has no top-level `{name}`")
        }
    }

    /// Checks that no template or macro is defined in two files.
    fn check_global_clashes(&mut self) {
        let mut seen: HashMap<String, PathBuf> = HashMap::new();
        for &i in &self.order {
            let module = &self.modules[i];
            let scope = module.model.module_scope();
            for sym in module.model.symbols() {
                if sym.scope != scope
                    || sym.parent.is_some()
                    || !matches!(sym.kind, SymbolKind::Template | SymbolKind::Macro)
                {
                    continue;
                }
                match seen.get(&sym.name) {
                    Some(first) if first != &module.path => self.errors.push(error_at(
                        &module.model,
                        &module.path,
                        sym.name_span.start,
                        format!(
                            "`{}` is already defined in {} (templates and macros are shared by every module)",
                            sym.name,
                            display_name(first)
                        ),
                    )),
                    Some(_) => {}
                    None => {
                        seen.insert(sym.name.clone(), module.path.clone());
                    }
                }
            }
        }
    }

    /// Rewrites module `m`'s text for linking.
    fn rewrite(&mut self, m: usize) -> String {
        let mut edits: Vec<(Span, String)> = Vec::new();
        let mut errors: Vec<(usize, String)> = Vec::new();
        let module = &self.modules[m];
        let model = &module.model;
        let text = model.text();
        let tokens = model.tokens();
        let spans = model.spans();

        let mut replaced: Vec<Span> = Vec::new();
        // Bindings resolved at link time rather than by assignment.
        let mut namespaces: HashMap<String, Namespace> = HashMap::new();
        let mut direct: HashMap<String, String> = HashMap::new();
        // `from m import *` names, which this file's model sees as
        // unresolved.
        let mut starred: HashMap<String, String> = HashMap::new();
        // Import bindings bound by an assignment (renamed with the prefix).
        let mut assigned: HashSet<String> = HashSet::new();

        for imp in &module.imports {
            let end = statement_end(tokens, imp.pos);
            let Some(last) = (imp.pos..end)
                .rev()
                .find(|&i| !crate::ide::is_marker(&tokens[i]))
            else {
                continue;
            };
            let span = Span::new(spans[imp.pos].start, spans[last].end);
            let mut bindings = Vec::new();
            match &imp.kind {
                ImportKind::Modules(list) => {
                    for mi in list {
                        match &mi.alias {
                            Some(a) => {
                                namespaces
                                    .insert(a.clone(), Namespace::new(*mi.chain.last().unwrap()));
                            }
                            None => {
                                let rest: Vec<(String, usize)> = mi.path[1..]
                                    .iter()
                                    .cloned()
                                    .zip(mi.chain[1..].iter().copied())
                                    .collect();
                                namespaces
                                    .entry(mi.path[0].clone())
                                    .or_insert_with(|| Namespace::new(mi.chain[0]))
                                    .merge(&rest);
                            }
                        }
                    }
                }
                ImportKind::From { module: None, .. } => continue, // native `use`
                ImportKind::From {
                    module: Some(t),
                    items,
                    submodules,
                } => {
                    let star = items.iter().any(|(n, _)| n == "*");
                    let expanded: Vec<(String, Option<String>)> = if star {
                        star_names(&self.modules[*t].model)
                            .into_iter()
                            .map(|n| (n, None))
                            .collect()
                    } else {
                        items.clone()
                    };
                    for (orig, alias) in expanded {
                        let local = alias.clone().unwrap_or_else(|| orig.clone());
                        let found = match submodules.get(&orig) {
                            Some(&s) => Some(Export::Module(s)),
                            None => self.export(*t, &orig, 0),
                        };
                        let name_tok = (imp.pos + 1..end)
                            .find(|&i| matches!(&tokens[i], Token::Ident(n) if *n == orig))
                            .unwrap_or(imp.pos);
                        match found {
                            Some(Export::Module(i)) => {
                                namespaces.insert(local, Namespace::new(i));
                            }
                            Some(Export::Value(linked)) if star => {
                                starred.insert(local, linked);
                            }
                            Some(Export::Value(linked)) => {
                                let lhs = format!("{}{local}", module.prefix);
                                if lhs != linked {
                                    bindings.push(format!("{lhs} = {linked}"));
                                }
                                assigned.insert(local);
                            }
                            Some(Export::Type(linked)) => {
                                if star {
                                    starred.insert(local, linked);
                                } else {
                                    direct.insert(local, linked);
                                }
                            }
                            Some(Export::Macro(name)) => {
                                if local != name {
                                    errors.push((
                                        name_tok,
                                        format!("`{name}` is a template or macro and can't be imported under another name"),
                                    ));
                                }
                            }
                            Some(Export::Native(n)) => errors.push((
                                name_tok,
                                format!(
                                    "`{n}` is a native Rust import of `{}`; import it with `use` directly",
                                    module_name(&self.modules[*t].path)
                                ),
                            )),
                            None if star => {}
                            None => errors.push((name_tok, self.missing(*t, &orig))),
                        }
                    }
                }
            }
            // Keep the statement's line count (a parenthesized import may
            // span lines) so line numbers stay exact.
            let newlines = text[span.start..span.end].matches('\n').count();
            let mut replacement = bindings.join("; ");
            replacement.push_str(&"\n".repeat(newlines));
            edits.push((span, replacement));
            replaced.push(span);
        }

        // Uses of import bindings, and names from `import *`.
        for r in model.references() {
            if r.kind == RefKind::Definition || replaced.iter().any(|s| s.contains(r.span.start)) {
                continue;
            }
            let local = r.name.as_str();
            let is_import_binding = match r.target {
                Resolution::Symbol(id) => {
                    let sym = model.symbol(id);
                    sym.kind == SymbolKind::Import && sym.scope == model.module_scope()
                }
                Resolution::Unresolved => false,
                _ => continue,
            };
            if is_import_binding {
                if let Some(ns) = namespaces.get(local) {
                    // Walk `a.b.name`.
                    let mut ns = ns.clone();
                    let mut tok = r.token;
                    let mut dotted = local.to_string();
                    loop {
                        let (Some(Token::Dot), Some(Token::Ident(n))) =
                            (tokens.get(tok + 1), tokens.get(tok + 2))
                        else {
                            errors.push((
                                r.token,
                                format!("module `{dotted}` can only be used as `{dotted}.name`"),
                            ));
                            break;
                        };
                        if let Some(child) = ns.children.get(n) {
                            ns = child.clone();
                            tok += 2;
                            dotted = format!("{dotted}.{n}");
                            continue;
                        }
                        match self.export(ns.module, n, 0) {
                            Some(
                                Export::Value(linked)
                                | Export::Type(linked)
                                | Export::Macro(linked),
                            ) => {
                                edits.push((Span::new(r.span.start, spans[tok + 2].end), linked));
                                break;
                            }
                            Some(Export::Module(i)) => {
                                ns = Namespace::new(i);
                                tok += 2;
                                dotted = format!("{dotted}.{n}");
                            }
                            Some(Export::Native(n2)) => {
                                errors.push((
                                    tok + 2,
                                    format!("`{n2}` is a native Rust import; use it through `use`"),
                                ));
                                break;
                            }
                            None => {
                                errors.push((tok + 2, self.missing(ns.module, n)));
                                break;
                            }
                        }
                    }
                } else if let Some(linked) = direct.get(local) {
                    edits.push((r.span, linked.clone()));
                }
            } else if r.target == Resolution::Unresolved
                && let Some(linked) = starred.get(local)
            {
                edits.push((r.span, linked.clone()));
            }
        }

        // A (non-entry) module's own top-level names get its prefix.
        if !module.prefix.is_empty() {
            let scope = model.module_scope();
            let mut renamed: HashMap<String, String> = HashMap::new();
            for sym in model.symbols() {
                if sym.scope != scope || sym.parent.is_some() {
                    continue;
                }
                let renameable = match sym.kind {
                    SymbolKind::Function
                    | SymbolKind::Generator
                    | SymbolKind::Variable
                    | SymbolKind::DataClass
                    | SymbolKind::Class
                    | SymbolKind::Struct
                    | SymbolKind::Trait => true,
                    SymbolKind::Import => assigned.contains(&sym.name),
                    _ => false,
                };
                if !renameable || matches!(tokens.get(sym.def_token), Some(Token::String(_))) {
                    continue;
                }
                let new = format!("{}{}", module.prefix, sym.name);
                renamed.insert(sym.name.clone(), new.clone());
                for r in model.references_to(sym.id) {
                    if replaced.iter().any(|s| s.contains(r.span.start)) {
                        continue;
                    }
                    if text.get(r.span.start..r.span.end) == Some(sym.name.as_str()) {
                        edits.push((r.span, new.clone()));
                    }
                }
            }
            // `global name` declarations aren't references; rename them too.
            for (i, t) in tokens.iter().enumerate() {
                if !matches!(t, Token::Global) {
                    continue;
                }
                for j in i + 1..tokens.len() {
                    match &tokens[j] {
                        Token::Ident(n) => {
                            if let Some(new) = renamed.get(n) {
                                edits.push((spans[j], new.clone()));
                            }
                        }
                        Token::Comma => {}
                        _ => break,
                    }
                }
            }
        }

        let out = apply_edits(text, edits);
        for (tok, message) in errors {
            self.err(m, tok, message);
        }
        out
    }
}

fn apply_edits(text: &str, mut edits: Vec<(Span, String)>) -> String {
    // Widest edit first at each start, so an enclosing edit wins.
    edits.sort_by_key(|(s, _)| (s.start, std::cmp::Reverse(s.end)));
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (span, new) in edits {
        if span.start < cursor {
            continue; // Inside an earlier (enclosing) edit.
        }
        out.push_str(&text[cursor..span.start]);
        out.push_str(&new);
        cursor = span.end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Links the program whose entry file is `entry`. `roots` are extra
/// module search directories, searched after the entry's own directory
/// and `CRAWPATH`.
pub fn link(
    entry: &Path,
    roots: &[PathBuf],
    loader: &dyn SourceLoader,
) -> Result<Linked, Vec<LinkError>> {
    let entry = normalize(entry);
    let mut search = program_roots(&entry);
    search.extend(roots.iter().map(|r| normalize(r)));
    let mut linker = Linker {
        loader,
        roots: search,
        modules: Vec::new(),
        index: HashMap::new(),
        order: Vec::new(),
        errors: Vec::new(),
        prefixes: HashSet::new(),
    };
    if linker.load(&entry, true, &mut Vec::new()).is_none() {
        return Err(vec![LinkError {
            file: entry.clone(),
            line: 0,
            character: 0,
            message: "cannot read file".to_string(),
        }]);
    }
    linker.check_global_clashes();

    let order = linker.order.clone();
    let mut source = String::new();
    let mut segments = Vec::new();
    let mut line = 0u32;
    for &m in &order {
        let mut text = linker.rewrite(m);
        if text.is_empty() {
            continue;
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let count = text.matches('\n').count() as u32;
        segments.push(Segment {
            file: linker.modules[m].path.clone(),
            start_line: line,
            line_count: count,
        });
        line += count;
        source.push_str(&text);
    }
    if !linker.errors.is_empty() {
        let mut errors = linker.errors;
        errors.sort_by(|a, b| (&a.file, a.line, a.character).cmp(&(&b.file, b.line, b.character)));
        errors.dedup();
        return Err(errors);
    }
    Ok(Linked {
        source,
        files: order
            .iter()
            .filter(|&&m| !linker.modules[m].virtual_package)
            .map(|&m| linker.modules[m].path.clone())
            .collect(),
        segments,
    })
}

/// Links and parses the program rooted at `entry`: the AST the transpiler
/// and interpreter run, with the link result (for its file list). Any
/// error is reported against the file and line it came from.
pub fn parse_program(
    entry: &Path,
    roots: &[PathBuf],
    loader: &dyn SourceLoader,
) -> Result<(Vec<Stmt>, Linked), Vec<LinkError>> {
    use chumsky::Parser;
    let linked = link(entry, roots, loader)?;
    let (tokens, spans) = crate::lexer::Lexer::new(&linked.source).tokenize_spanned();
    let (ast, errors) = crate::parser::parser().parse(&tokens).into_output_errors();
    if !errors.is_empty() {
        // Every file parsed on its own, so this only happens if linking
        // produced something unparsable; still point at the source line.
        let li = crate::text::LineIndex::new(&linked.source);
        return Err(errors
            .iter()
            .map(|err| {
                let e = crate::diagnostics::render_parse_error(err, &spans, linked.source.len());
                let p = li.position(e.span.start);
                let (file, line) = linked
                    .original_line(p.line)
                    .map(|(f, l)| (f.to_path_buf(), l))
                    .unwrap_or_else(|| (entry.to_path_buf(), p.line));
                LinkError {
                    file,
                    line,
                    character: p.character,
                    message: e.message,
                }
            })
            .collect());
    }
    Ok((ast.unwrap_or_default(), linked))
}

/// `path` relative to the current directory when it is inside it (what
/// editors' problem matchers and humans expect), else as is.
pub fn display_path(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            path.strip_prefix(normalize(&cwd))
                .ok()
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// Formats errors one per line as `file:line:col: error: message`.
pub fn format_errors(errors: &[LinkError]) -> String {
    errors
        .iter()
        .map(|e| {
            format!(
                "{}:{}:{}: error: {}",
                display_path(&e.file),
                e.line + 1,
                e.character + 1,
                e.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
