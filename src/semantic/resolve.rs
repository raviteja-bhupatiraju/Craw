//! Resolves every identifier token in a document to what it refers to, by
//! walking the token stream once (never the raw source text).

use super::SemanticModel;
use super::symbols::{ScopeId, SymbolId, SymbolKind};
use crate::lexer::{Lexer, Token};
use crate::text::Span;
use std::collections::HashSet;

/// What an identifier token refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A user-defined symbol found by scope lookup or as a definition site.
    Symbol(SymbolId),
    /// An identifier right after `.`/`..`: resolved by name against every
    /// known field, since dynamic typing means the receiver's concrete
    /// type usually isn't known. Empty when no field anywhere has this
    /// name — still not `Unresolved`, since dynamic attribute access is
    /// not an error.
    Field(Vec<SymbolId>),
    /// A name from `transpiler::BUILTINS`.
    Builtin(&'static str),
    /// A built-in type name (`Int`, `String`, ...).
    BuiltinType(&'static str),
    /// Not a keyword, not a field access, not bound anywhere, not a
    /// builtin or builtin type.
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// The token is the symbol's own name (where it's defined/bound).
    Definition,
    /// The token is the target of an assignment to an existing binding.
    Write,
    /// Any other occurrence.
    Read,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub span: Span,
    pub token: usize,
    pub name: String,
    pub target: Resolution,
    pub kind: RefKind,
    pub scope: ScopeId,
}

/// Built-in type names recognized in annotations (`-> Int`, `x: String`),
/// matched the same way `transpiler::BUILTINS` is: dynamically-typed Craw
/// has no separate "type" namespace, these are just names the transpiler
/// gives special meaning to that never get user `Symbol`s of their own.
const BUILTIN_TYPES: &[&str] = &[
    "Int", "Float", "String", "Bool", "List", "Dict", "Set", "Tuple", "Dynamic", "None",
];

fn builtin(name: &str) -> Option<&'static str> {
    crate::transpiler::BUILTINS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, _)| *n)
}

fn builtin_type(name: &str) -> Option<&'static str> {
    BUILTIN_TYPES.iter().find(|&&t| t == name).copied()
}

impl SemanticModel {
    /// The innermost scope containing byte offset `offset`: the containing
    /// scope with the smallest span, since scopes nest properly (a child's
    /// span is always fully inside its parent's).
    pub fn scope_at(&self, offset: usize) -> ScopeId {
        self.scopes()
            .iter()
            .filter(|s| s.span.start <= offset && offset <= s.span.end)
            .min_by_key(|s| s.span.end.saturating_sub(s.span.start))
            .map(|s| s.id)
            .unwrap_or(self.module_scope())
    }

    /// Every resolved reference in the document, in token order.
    pub fn references(&self) -> &[Reference] {
        &self.refs
    }

    pub fn references_named<'a>(&'a self, name: &str) -> Vec<&'a Reference> {
        self.refs.iter().filter(|r| r.name == name).collect()
    }

    pub fn references_to(&self, id: SymbolId) -> Vec<&Reference> {
        self.refs
            .iter()
            .filter(|r| r.target == Resolution::Symbol(id))
            .collect()
    }

    pub(super) fn build_references(&mut self) {
        let defs_by_span: std::collections::HashMap<Span, SymbolId> =
            self.symbols.iter().map(|s| (s.name_span, s.id)).collect();
        let write_targets: HashSet<Span> = self.write_targets.iter().copied().collect();
        let import_labels = self.import_label_tokens(&defs_by_span);

        let mut refs = Vec::new();
        for (i, tok) in self.tokens.iter().enumerate() {
            match tok {
                Token::Ident(name) => {
                    if self.blanked.iter().any(|&(s, e)| i >= s && i < e) {
                        continue;
                    }
                    if import_labels.contains(&i) {
                        continue;
                    }
                    let span = self.spans[i];
                    let scope = self.scope_at(span.start);
                    // A definition site wins even after a `.`: the binding
                    // `circle` in `import shapes.circle`.
                    let is_field = !defs_by_span.contains_key(&span)
                        && matches!(
                        self.tokens.get(i.wrapping_sub(1)),
                        Some(Token::Dot) | Some(Token::DotDot)
                    ) && i > 0;
                    let target = if is_field {
                        let ids: Vec<SymbolId> = self
                            .symbols
                            .iter()
                            .filter(|s| s.kind == SymbolKind::Field && &s.name == name)
                            .map(|s| s.id)
                            .collect();
                        Resolution::Field(ids)
                    } else if let Some(&id) = defs_by_span.get(&span) {
                        Resolution::Symbol(id)
                    } else if let Some(id) = self.lookup(name, scope) {
                        Resolution::Symbol(id)
                    } else if let Some(b) = builtin(name) {
                        Resolution::Builtin(b)
                    } else if let Some(t) = builtin_type(name) {
                        Resolution::BuiltinType(t)
                    } else {
                        Resolution::Unresolved
                    };
                    let kind = if defs_by_span.contains_key(&span) {
                        RefKind::Definition
                    } else if write_targets.contains(&span) {
                        RefKind::Write
                    } else {
                        RefKind::Read
                    };
                    refs.push(Reference {
                        span,
                        token: i,
                        name: name.clone(),
                        target,
                        kind,
                        scope,
                    });
                }
                Token::FStringRaw(raw) => {
                    if self.blanked.iter().any(|&(s, e)| i >= s && i < e) {
                        continue;
                    }
                    let scope = self.scope_at(self.spans[i].start);
                    // `f"..."`: the raw content starts 2 bytes (`f"`) into
                    // the token's own span, and is copied verbatim from the
                    // source with no escape processing, so byte offsets in
                    // `raw` map 1:1 onto source bytes from there.
                    let base = self.spans[i].start + 2;
                    for (seg_start, seg_text) in fstring_segments(raw) {
                        let mut lx = Lexer::new(seg_text);
                        let (seg_tokens, seg_spans) = lx.tokenize_spanned();
                        for (tok, sp) in seg_tokens.iter().zip(seg_spans.iter()) {
                            if let Token::Ident(name) = tok {
                                let abs = Span::new(
                                    base + seg_start + sp.start,
                                    base + seg_start + sp.end,
                                );
                                let target = if let Some(id) = self.lookup(name, scope) {
                                    Resolution::Symbol(id)
                                } else if let Some(b) = builtin(name) {
                                    Resolution::Builtin(b)
                                } else if let Some(t) = builtin_type(name) {
                                    Resolution::BuiltinType(t)
                                } else {
                                    Resolution::Unresolved
                                };
                                refs.push(Reference {
                                    span: abs,
                                    token: i,
                                    name: name.clone(),
                                    target,
                                    kind: RefKind::Read,
                                    scope,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        self.refs = refs;
    }

    /// Identifier tokens inside import statements that aren't bindings:
    /// module path segments (`shapes` in `import shapes.circle`) and the
    /// original name of an aliased item (`c` in `from m import c as d`).
    /// They name things in *another* file, not anything in scope here, so
    /// they get no [`Reference`] (the editor resolves them across files).
    fn import_label_tokens(
        &self,
        defs_by_span: &std::collections::HashMap<Span, SymbolId>,
    ) -> HashSet<usize> {
        let mut out = HashSet::new();
        let mut starts = Vec::new();
        crate::ide::walk_stmts(&self.ast, &mut |stmt| {
            if let crate::ast::Stmt::FromImport(_, _, pos) | crate::ast::Stmt::Import(_, pos) = stmt
            {
                starts.push(*pos);
            }
        });
        for pos in starts {
            let end = (pos..self.tokens.len())
                .find(|&i| matches!(self.tokens[i], Token::Newline | Token::Semicolon | Token::Eof))
                .unwrap_or(self.tokens.len());
            for i in pos..end {
                if matches!(self.tokens[i], Token::Ident(_))
                    && !defs_by_span.contains_key(&self.spans[i])
                {
                    out.insert(i);
                }
            }
        }
        out
    }
}

/// Finds each top-level `{...}` interpolation segment in an f-string's raw
/// content, returning `(byte offset of the segment's first char within
/// `raw`, the segment's inner text)`. Doesn't handle nested braces (a dict
/// literal inside an interpolation) — good enough for the common case.
fn fstring_segments(raw: &str) -> Vec<(usize, &str)> {
    let mut segments = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut depth = 1;
            let mut j = start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    j += 1;
                }
            }
            if depth == 0 {
                segments.push((start, &raw[start..j]));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    segments
}
