//! `textDocument/inlayHint`: parameter names at call sites, inferred types
//! on first assignments, and inferred return types on unannotated `def`s.
//!
//! Every hint is conservative: a type hint is shown only when inference is
//! certain (see [`crate::lsp_analyzer::infer_expr_type`]), and hints that
//! would restate what the source already says (`x = 1`, `p = Point(..)`,
//! `f(name)` for parameter `name`) are suppressed.

use crate::ast::{Expr, Pattern, Stmt};
use crate::lexer::Token;
use crate::lsp_analyzer::{InferredReceiver, infer_expr_type};
use crate::semantic::{RefKind, Resolution, SemanticModel, SymbolKind};
use crate::text::Span;
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel};

#[derive(Debug, Clone, Copy)]
pub struct HintConfig {
    pub parameter_names: bool,
    pub types: bool,
    pub return_types: bool,
}

impl Default for HintConfig {
    fn default() -> Self {
        Self {
            parameter_names: true,
            types: true,
            return_types: true,
        }
    }
}

pub fn inlay_hints(
    model: &SemanticModel,
    range: Span,
    config: HintConfig,
    cross: &dyn super::CrossFile,
) -> Vec<InlayHint> {
    let mut out = Vec::new();
    if config.parameter_names {
        parameter_hints(model, cross, &mut out);
    }
    if config.types {
        type_hints(model, &mut out);
    }
    if config.return_types {
        return_type_hints(model, &mut out);
    }
    let li = model.line_index();
    let (start, end) = (li.position(range.start), li.position(range.end));
    out.retain(|h| {
        let p = (h.position.line, h.position.character);
        (start.line, start.character) <= p && p <= (end.line, end.character)
    });
    out.sort_by_key(|h| (h.position.line, h.position.character));
    out
}

fn hint(model: &SemanticModel, offset: usize, label: String, kind: InlayHintKind) -> InlayHint {
    let is_param = kind == InlayHintKind::PARAMETER;
    InlayHint {
        position: model.line_index().position(offset).into(),
        label: InlayHintLabel::String(label),
        kind: Some(kind),
        text_edits: None,
        tooltip: None,
        padding_left: Some(!is_param),
        padding_right: Some(is_param),
        data: None,
    }
}

/// Call sites `f(a, b)` of a user function/constructor with a known
/// signature. Stops at the first keyword argument (positions no longer
/// line up with parameters after it) and never labels a vararg twice.
fn parameter_hints(model: &SemanticModel, cross: &dyn super::CrossFile, out: &mut Vec<InlayHint>) {
    let tokens = model.tokens();
    let spans = model.spans();
    for r in model.references() {
        if r.kind != RefKind::Read {
            continue;
        }
        // A function imported from (or called as `module.f` on) another
        // file carries its signature there.
        let imported = cross.target_signature(r.token);
        let local = match r.target {
            Resolution::Symbol(id) => {
                let sym = model.symbol(id);
                matches!(
                    sym.kind,
                    SymbolKind::Function
                        | SymbolKind::Generator
                        | SymbolKind::Method
                        | SymbolKind::DataClass
                )
                .then(|| sym.signature.clone())
                .flatten()
            }
            _ => None,
        };
        let is_member = imported.is_some();
        let Some(sig) = imported.or(local) else { continue };
        let sig = &sig;
        let open = r.token + 1;
        if !matches!(tokens.get(open), Some(Token::LParen)) || spans[open].start != r.span.end {
            continue;
        }
        // `x.f(y)` passes `x` as the first parameter (UFCS) — unless `x`
        // is a module, where `m.f(y)` is a plain call.
        if !is_member && r.token > 0 && matches!(tokens[r.token - 1], Token::Dot) {
            continue;
        }
        let Some(close) = super::matching_close(tokens, open) else {
            continue;
        };
        for (index, (arg_start, arg_end)) in arguments(tokens, open, close).into_iter().enumerate()
        {
            if matches!(tokens[arg_start], Token::Ident(_))
                && matches!(tokens.get(arg_start + 1), Some(Token::Assign))
            {
                break;
            }
            if matches!(tokens[arg_start], Token::Star | Token::Power) {
                break;
            }
            let name = match sig.params.get(index) {
                Some(p) => p.name.clone(),
                None => match &sig.vararg {
                    Some(v) if index == sig.params.len() => v.clone(),
                    _ => break,
                },
            };
            if redundant(model, &name, arg_start, arg_end) {
                continue;
            }
            out.push(hint(
                model,
                spans[arg_start].start,
                format!("{name}:"),
                InlayHintKind::PARAMETER,
            ));
        }
    }
}

/// `(first, last)` token indices of each depth-0 argument between the
/// brackets at `open` and `close`.
fn arguments(tokens: &[Token], open: usize, close: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = open + 1;
    for (i, t) in tokens.iter().enumerate().take(close + 1).skip(open + 1) {
        match t {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace if depth > 0 => depth -= 1,
            Token::Comma if depth == 0 => {
                if i > start {
                    out.push((start, i - 1));
                }
                start = i + 1;
            }
            _ if i == close && i > start => out.push((start, i - 1)),
            _ => {}
        }
    }
    out
}

/// Whether the argument already says what the hint would: a lone
/// identifier (or attribute chain ending in one) whose name matches the
/// parameter, ignoring case and a leading underscore.
fn redundant(model: &SemanticModel, param: &str, start: usize, end: usize) -> bool {
    let norm = |s: &str| s.trim_start_matches('_').to_ascii_lowercase();
    let param = norm(param);
    if param.len() <= 1 && end == start && matches!(model.tokens()[start], Token::Ident(_)) {
        // `f(x)` for parameter `x`: single letters carry no information.
        return norm(super::token_text(model, start)) == param;
    }
    match &model.tokens()[end] {
        Token::Ident(name) => {
            let lone_or_chain =
                (start..end).all(|i| matches!(model.tokens()[i], Token::Ident(_) | Token::Dot));
            lone_or_chain && norm(name) == param
        }
        _ => false,
    }
}

/// Whether the expression's own shape already states its type.
fn self_evident(expr: &Expr, t: InferredReceiver<'_>) -> bool {
    match expr {
        Expr::Number(_)
        | Expr::Float(_)
        | Expr::String(_)
        | Expr::FString(_)
        | Expr::Bool(_)
        | Expr::List(_)
        | Expr::Dict(_)
        | Expr::Set(_)
        | Expr::Tuple(_) => true,
        Expr::Call(callee, _) => {
            matches!(
                (callee.as_ref(), t),
                (Expr::Ident(name), InferredReceiver::Custom(sym)) if *name == sym.name
            ) || matches!(callee.as_ref(), Expr::Ident(name) if name == "str" || name == "list")
        }
        _ => false,
    }
}

/// `x = expr` (unannotated, and the binding's first definition): the
/// inferred type after the name.
fn type_hints(model: &SemanticModel, out: &mut Vec<InlayHint>) {
    let tokens = model.tokens();
    let spans = model.spans();
    super::walk_stmts(model.ast(), &mut |stmt| {
        let Stmt::Assign(Pattern::Var(name, None), expr, tok) = stmt else {
            return;
        };
        if !matches!(tokens.get(*tok), Some(Token::Ident(n)) if n == name) {
            return;
        }
        let is_definition = model
            .references()
            .iter()
            .any(|r| r.token == *tok && r.kind == RefKind::Definition);
        if !is_definition {
            return;
        }
        let at = spans[*tok].start;
        let Some(t) = infer_expr_type(model, expr, at, 0) else {
            return;
        };
        if self_evident(expr, t) {
            return;
        }
        out.push(hint(
            model,
            spans[*tok].end,
            format!(": {}", super::type_name(t)),
            InlayHintKind::TYPE,
        ));
    });
}

/// `def f(...):` with no `-> Type`: when every `return` in the body
/// infers to the same type, show `-> Type` after the closing parenthesis.
fn return_type_hints(model: &SemanticModel, out: &mut Vec<InlayHint>) {
    let tokens = model.tokens();
    let spans = model.spans();
    super::walk_stmts(model.ast(), &mut |stmt| {
        let Stmt::FunctionDef {
            return_type: None,
            is_generator: false,
            id,
            ..
        } = stmt
        else {
            return;
        };
        let Some(t) = crate::lsp_analyzer::infer_function_return(model, *id, 0) else {
            return;
        };
        let Some(open) = (*id..tokens.len()).find(|&i| matches!(tokens[i], Token::LParen)) else {
            return;
        };
        let Some(close) = super::matching_close(tokens, open) else {
            return;
        };
        out.push(hint(
            model,
            spans[close].end,
            format!("-> {}", super::type_name(t)),
            InlayHintKind::TYPE,
        ));
    });
}
