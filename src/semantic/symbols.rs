//! Types shared by the semantic model: symbols (named things a program
//! defines) and scopes (the regions of source where a binding is visible).

use crate::ast::Type;
use crate::text::Span;
use std::collections::HashMap;

/// Identifies a [`Symbol`] within one [`super::SemanticModel`]. Indexes
/// directly into `SemanticModel::symbols()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

/// Identifies a [`Scope`] within one [`super::SemanticModel`]. Indexes
/// directly into `SemanticModel::scopes()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Generator,
    Method,
    Parameter,
    Variable,
    DataClass,
    Field,
    Struct,
    Trait,
    Impl,
    Class,
    Template,
    Macro,
    Operator,
    Import,
    Builtin,
    BuiltinType,
}

/// One parameter of a [`Signature`]. `default` is the default value's
/// original source text (not a parsed `Expr`) since it exists purely for
/// display (hover, signature help), not evaluation.
#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub default: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Signature {
    pub params: Vec<Param>,
    pub vararg: Option<String>,
    pub return_type: Option<String>,
}

/// A named thing a program defines: a function, a variable, a data-class
/// field, and so on.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    /// The exact span of the symbol's name (for hover ranges, rename
    /// ranges, go-to-definition targets).
    pub name_span: Span,
    /// The span of the symbol's entire definition (for outline ranges,
    /// folding).
    pub full_span: Span,
    /// The scope this symbol lives in (for a top-level `def`, the module
    /// scope; for a parameter, the function's own scope).
    pub scope: ScopeId,
    /// The enclosing symbol this one is a child of (a method's class, a
    /// field's data/struct), if any.
    pub parent: Option<SymbolId>,
    pub signature: Option<Signature>,
    /// Contiguous `#` comment lines immediately above the definition,
    /// joined with `\n` and stripped of their leading `# `.
    pub docs: Option<String>,
    /// The token index this symbol's definition starts at (the statement's
    /// recorded position, e.g. `Stmt::FunctionDef`'s `id` field).
    pub def_token: usize,
    /// An explicit `name: Type` annotation on a parameter or assignment,
    /// rendered via [`type_to_string`] — the primary signal type
    /// inference (for member completion) trusts over any structural
    /// guess, exactly like an annotation in a statically-typed language.
    pub declared_type: Option<String>,
    /// For an `Import` binding: what it imports. `None` for everything
    /// else.
    pub import: Option<ImportSource>,
}

/// Where an import binding points: `from geometry import area as a` is
/// `{ module: ["geometry"], name: Some("area") }` bound as `a`;
/// `import shapes.circle` is `{ module: ["shapes", "circle"], name: None }`
/// bound as `circle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSource {
    pub module: Vec<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Module,
    Function,
    Lambda,
    Class,
    Template,
    Macro,
    Comprehension,
    Where,
    Query,
}

/// A region of source where a set of names is visible.
#[derive(Debug, Clone)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub span: Span,
    pub kind: ScopeKind,
    pub bindings: HashMap<String, SymbolId>,
    /// The symbol that introduces this scope (a function's own body scope
    /// is owned by the function's `Symbol`), if any — lambdas,
    /// comprehensions, `where` blocks, and queries introduce a scope with
    /// no owning symbol.
    pub owner: Option<SymbolId>,
}

/// Renders a [`Type`] the way it would appear in Craw source (used in
/// signatures and hover text).
pub(crate) fn type_to_string(t: &Type) -> String {
    match t {
        Type::Int => "Int".to_string(),
        Type::String => "String".to_string(),
        Type::Custom(name) => name.clone(),
        Type::Generic(name, params) => {
            let params_str: Vec<String> = params.iter().map(type_to_string).collect();
            format!("{}<{}>", name, params_str.join(", "))
        }
        Type::Dynamic => "Dynamic".to_string(),
    }
}
