//! The semantic model: a resolved view of one document combining its AST,
//! token stream, and every symbol it defines. Built once per document
//! version; replaces the old text-search-based `lsp_analyzer` scope
//! tracking with real AST-driven analysis.

mod build;
mod builtins;
mod diagnostics;
mod resolve;
mod symbols;
mod workspace;

pub use builtins::{
    BuiltinDoc, BuiltinMethod, builtin_doc, builtin_method_doc, builtin_methods, builtin_type_doc,
};
pub use resolve::{RefKind, Reference, Resolution};
pub use symbols::{
    ImportSource, Param, Scope, ScopeId, ScopeKind, Signature, Symbol, SymbolId, SymbolKind,
};
pub use workspace::{Hit, Workspace};

use crate::ast::Stmt;
use crate::diagnostics::SyntaxError;
use crate::lexer::Token;
use crate::text::{LineIndex, Span};

#[derive(Debug)]
pub struct SemanticModel {
    text: String,
    line_index: LineIndex,
    tokens: Vec<Token>,
    spans: Vec<Span>,
    ast: Vec<Stmt>,
    symbols: Vec<Symbol>,
    scopes: Vec<Scope>,
    write_targets: Vec<Span>,
    refs: Vec<Reference>,
    semantic_diagnostics: Vec<tower_lsp::lsp_types::Diagnostic>,
    syntax_errors: Vec<SyntaxError>,
    blanked: Vec<(usize, usize)>,
}

impl SemanticModel {
    /// Lexes, parses (with error recovery), and resolves `text` into a
    /// complete semantic model.
    pub fn build(text: &str) -> Self {
        let recovered = crate::recovery::parse_with_recovery(text);
        let line_index = LineIndex::new(text);
        let mut scopes = vec![build::new_module_scope(Span::new(0, text.len()))];
        let mut symbols = Vec::new();
        let mut write_targets = Vec::new();
        let ctx = build::Ctx::new(&recovered.tokens, &recovered.spans, &line_index);
        build::collect(
            &recovered.ast,
            &ctx,
            ScopeId(0),
            None,
            &mut symbols,
            &mut scopes,
            &mut write_targets,
        );

        let mut model = Self {
            text: text.to_string(),
            line_index,
            tokens: recovered.tokens,
            spans: recovered.spans,
            ast: recovered.ast,
            symbols,
            scopes,
            write_targets,
            refs: Vec::new(),
            semantic_diagnostics: Vec::new(),
            syntax_errors: recovered.errors,
            blanked: recovered.blanked,
        };
        model.build_references();
        model.semantic_diagnostics = diagnostics::compute(&model);
        model
    }

    /// Semantic diagnostics: unresolved names, unused locals, duplicate
    /// definitions, call-arity mismatches, misplaced `return`/`break`/
    /// `continue`/`nonlocal`, and an informational note on inline Rust.
    /// Nothing is reported inside a blanked (recovered-from-error) region.
    pub fn semantic_diagnostics(&self) -> &[tower_lsp::lsp_types::Diagnostic] {
        &self.semantic_diagnostics
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    pub fn ast(&self) -> &[Stmt] {
        &self.ast
    }

    pub fn syntax_errors(&self) -> &[SyntaxError] {
        &self.syntax_errors
    }

    pub fn blanked_regions(&self) -> &[(usize, usize)] {
        &self.blanked
    }

    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0 as usize]
    }

    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    pub fn module_scope(&self) -> ScopeId {
        ScopeId(0)
    }

    /// The scope that `id`'s definition itself introduces (a function's or
    /// class's own body scope), if it introduces one at all.
    pub fn scope_of(&self, id: SymbolId) -> Option<ScopeId> {
        self.scopes
            .iter()
            .find(|s| s.owner == Some(id))
            .map(|s| s.id)
    }

    /// Resolves `name` by walking outward from `scope` through its parent
    /// chain, returning the first binding found.
    pub fn lookup(&self, name: &str, scope: ScopeId) -> Option<SymbolId> {
        let mut current = Some(scope);
        while let Some(id) = current {
            let s = &self.scopes[id.0 as usize];
            if let Some(sym) = s.bindings.get(name) {
                return Some(*sym);
            }
            current = s.parent;
        }
        None
    }

    /// The symbol named `name`, preferring one bound in the module scope,
    /// falling back to the first matching symbol anywhere (a field, a
    /// method, a local) if none is bound at module scope.
    pub fn symbol_named(&self, name: &str) -> Option<&Symbol> {
        if let Some(id) = self.scopes[0].bindings.get(name) {
            return Some(self.symbol(*id));
        }
        self.symbols.iter().find(|s| s.name == name)
    }

    /// Every symbol whose `parent` is `id`, in definition order.
    pub fn children(&self, id: SymbolId) -> Vec<&Symbol> {
        self.symbols
            .iter()
            .filter(|s| s.parent == Some(id))
            .collect()
    }
}
