//! Editor features built on top of [`SemanticModel`]: everything the LSP
//! server offers beyond plain name resolution (semantic highlighting,
//! folding, selection ranges, inlay hints, code actions, formatting, call
//! hierarchy). Each submodule is a pure function of one document's model,
//! so every feature is testable without a running server; `lsp_analyzer`
//! only looks up the model by URI and delegates here.

pub mod call_hierarchy;
pub mod code_actions;
pub mod folding;
pub mod formatting;
pub mod inlay_hints;
pub mod keyword_docs;
pub mod selection;
pub mod semantic_tokens;

use crate::ast::Stmt;
use crate::lexer::Token;
use crate::lsp_analyzer::InferredReceiver;
use crate::semantic::SemanticModel;
use crate::text::Span;

/// Cross-file knowledge the single-file features can ask for: what an
/// imported name (or `module.name` member) really is. The language server
/// implements it over the whole project; [`NoCrossFile`] knows nothing.
pub trait CrossFile {
    /// For identifier token `tok`: the kind of what it refers to in
    /// another file, and whether that is a whole module.
    fn target_kind(&self, tok: usize) -> Option<(crate::semantic::SymbolKind, bool)>;
    /// For identifier token `tok`: the signature of the function it
    /// refers to in another file.
    fn target_signature(&self, tok: usize) -> Option<crate::semantic::Signature>;
}

pub struct NoCrossFile;

impl CrossFile for NoCrossFile {
    fn target_kind(&self, _: usize) -> Option<(crate::semantic::SymbolKind, bool)> {
        None
    }
    fn target_signature(&self, _: usize) -> Option<crate::semantic::Signature> {
        None
    }
}

/// The display name of an inferred type (`Int`, `List`, or a user
/// `data`/`class` name).
pub(crate) fn type_name(t: InferredReceiver<'_>) -> String {
    match t {
        InferredReceiver::Named(n) => n.to_string(),
        InferredReceiver::Custom(sym) => sym.name.clone(),
    }
}

/// Zero-width layout markers the lexer emits between real tokens.
pub(crate) fn is_marker(t: &Token) -> bool {
    matches!(
        t,
        Token::Newline | Token::Indent | Token::Dedent | Token::Eof | Token::PassthroughStart
    )
}

/// Index of the bracket matching the opener at `open`, scanning forward.
pub(crate) fn matching_close(tokens: &[Token], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether two byte spans overlap (or touch, for zero-width spans).
pub(crate) fn overlaps(a: Span, b: Span) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Calls `f` on every statement in `stmts`, recursing into every nested
/// statement body (functions, branches, loops, match arms, classes, traits,
/// impls). Template and macro bodies are skipped: they are expanded by
/// textual substitution, so their contents aren't ordinary code.
pub(crate) fn walk_stmts<'a>(stmts: &'a [Stmt], f: &mut dyn FnMut(&'a Stmt)) {
    for stmt in stmts {
        f(stmt);
        match stmt {
            Stmt::FunctionDef { body, .. }
            | Stmt::While(_, body)
            | Stmt::MatchFor(_, _, body, _)
            | Stmt::ClassDef { body, .. }
            | Stmt::TraitDef(_, body, _)
            | Stmt::ImplBlock(_, _, body, _) => walk_stmts(body, f),
            Stmt::If(_, body, else_body) => {
                walk_stmts(body, f);
                walk_stmts(else_body, f);
            }
            Stmt::Match(_, arms, _) => {
                for (_, _, body) in arms {
                    walk_stmts(body, f);
                }
            }
            Stmt::MacroBlock { body, branches, .. } => {
                walk_stmts(body, f);
                for (_, _, b) in branches {
                    walk_stmts(b, f);
                }
            }
            _ => {}
        }
    }
}

/// The source text a token covers.
pub(crate) fn token_text(model: &SemanticModel, i: usize) -> &str {
    let s = model.spans()[i];
    model.text().get(s.start..s.end).unwrap_or("")
}
