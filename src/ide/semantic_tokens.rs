//! `textDocument/semanticTokens`: classifies every resolved identifier by
//! what it *refers to* (a parameter vs a local vs a module variable, a
//! builtin vs a user function), which the TextMate grammar can't know.
//! Keywords, literals and operators are left to the grammar.

use crate::lexer::Token;
use crate::semantic::{RefKind, Resolution, ScopeKind, SemanticModel, SymbolKind};
use crate::text::Span;
use tower_lsp::lsp_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend,
};

/// Token types, in legend order. The last entry is a custom type (as in
/// rust-analyzer) so themes can style names that resolve to nothing.
pub const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "struct",
    "interface",
    "parameter",
    "variable",
    "property",
    "function",
    "method",
    "macro",
    "operator",
    "decorator",
    "unresolvedReference",
];

/// Token modifiers, in legend (bit) order. `global` marks module-scope
/// variables so themes can tell them apart from locals.
pub const TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "modification",
    "defaultLibrary",
    "readonly",
    "global",
];

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES
            .iter()
            .map(|t| SemanticTokenType::new(t))
            .collect(),
        token_modifiers: TOKEN_MODIFIERS
            .iter()
            .map(|m| SemanticTokenModifier::new(m))
            .collect(),
    }
}

fn type_index(name: &str) -> u32 {
    TOKEN_TYPES.iter().position(|t| *t == name).unwrap() as u32
}

fn modifier_bit(name: &str) -> u32 {
    1 << TOKEN_MODIFIERS.iter().position(|m| *m == name).unwrap()
}

/// One classified identifier, before delta encoding — exposed so tests
/// can assert on types by name instead of decoding the wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub span: Span,
    pub token_type: &'static str,
    pub modifiers: Vec<&'static str>,
}

fn symbol_type(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function | SymbolKind::Generator => "function",
        SymbolKind::Method => "method",
        SymbolKind::Parameter => "parameter",
        SymbolKind::Variable => "variable",
        SymbolKind::DataClass | SymbolKind::Class => "class",
        SymbolKind::Field => "property",
        SymbolKind::Struct => "struct",
        SymbolKind::Trait => "interface",
        SymbolKind::Impl => "type",
        SymbolKind::Template | SymbolKind::Macro => "macro",
        SymbolKind::Operator => "operator",
        SymbolKind::Import => "namespace",
        SymbolKind::Builtin => "function",
        SymbolKind::BuiltinType => "type",
    }
}

/// Every identifier reference in source order, classified.
pub fn classify(model: &SemanticModel, cross: &dyn super::CrossFile) -> Vec<Classified> {
    let tokens = model.tokens();
    let text = model.text();
    let mut out = Vec::new();
    for r in model.references() {
        // A reference whose span doesn't cover exactly its own name (e.g. a
        // symbol synthesized for raw Rust) would highlight the wrong text.
        if text.get(r.span.start..r.span.end) != Some(r.name.as_str()) {
            continue;
        }
        let after_at = r.token > 0
            && matches!(tokens.get(r.token - 1), Some(Token::At))
            && model.spans()[r.token - 1].end == r.span.start;
        let mut modifiers = Vec::new();
        let token_type = match &r.target {
            _ if after_at => "decorator",
            // An imported name or `module.name`: classified by what it
            // names in the other file.
            _ if cross.target_kind(r.token).is_some() => {
                let (kind, whole_module) = cross.target_kind(r.token).unwrap();
                if whole_module {
                    "namespace"
                } else {
                    symbol_type(kind)
                }
            }
            Resolution::Symbol(id) => {
                let sym = model.symbol(*id);
                if sym.kind == SymbolKind::Variable
                    && model.scopes()[sym.scope.0 as usize].kind == ScopeKind::Module
                {
                    modifiers.push("global");
                }
                if matches!(sym.kind, SymbolKind::Builtin | SymbolKind::BuiltinType) {
                    modifiers.push("defaultLibrary");
                }
                symbol_type(sym.kind)
            }
            Resolution::Field(_) => "property",
            Resolution::Builtin(_) => {
                modifiers.push("defaultLibrary");
                "function"
            }
            Resolution::BuiltinType(_) => {
                modifiers.push("defaultLibrary");
                "type"
            }
            Resolution::Unresolved => "unresolvedReference",
        };
        match r.kind {
            RefKind::Definition => {
                modifiers.push("declaration");
                modifiers.push("definition");
            }
            RefKind::Write => modifiers.push("modification"),
            RefKind::Read => {}
        }
        out.push(Classified {
            span: r.span,
            token_type,
            modifiers,
        });
    }
    out.sort_by_key(|c| (c.span.start, c.span.end));
    out.dedup_by_key(|c| c.span.start);
    out
}

/// Delta-encodes classified tokens (optionally only those overlapping
/// `range`) into the LSP wire format. Positions are UTF-16, and a token
/// never spans lines (identifiers can't), so each maps to exactly one
/// wire token.
pub fn encode(
    model: &SemanticModel,
    range: Option<Span>,
    cross: &dyn super::CrossFile,
) -> Vec<SemanticToken> {
    let li = model.line_index();
    let mut out = Vec::new();
    let (mut prev_line, mut prev_start) = (0u32, 0u32);
    for c in classify(model, cross) {
        if let Some(r) = range
            && !super::overlaps(c.span, r)
        {
            continue;
        }
        let start = li.position(c.span.start);
        let end = li.position(c.span.end);
        if start.line != end.line {
            continue;
        }
        let delta_line = start.line - prev_line;
        let delta_start = if delta_line == 0 {
            start.character - prev_start
        } else {
            start.character
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: end.character - start.character,
            token_type: type_index(c.token_type),
            token_modifiers_bitset: c.modifiers.iter().fold(0, |acc, m| acc | modifier_bit(m)),
        });
        prev_line = start.line;
        prev_start = start.character;
    }
    out
}
