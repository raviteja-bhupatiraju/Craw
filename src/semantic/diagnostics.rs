//! Semantic diagnostics: checks that need name resolution or scope
//! structure, as opposed to [`crate::diagnostics`]'s pure syntax errors.

use super::SemanticModel;
use super::resolve::{RefKind, Resolution};
use super::symbols::SymbolKind;
use crate::ast::Stmt;
use crate::lexer::Token;
use crate::text::Span;
use std::collections::HashMap;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString};

fn diag(
    span: Span,
    model: &SemanticModel,
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
        tags: None,
        ..Default::default()
    }
}

/// Runs every semantic check and returns their combined diagnostics.
/// Nothing is reported for anything inside a blanked (recovered-from-error)
/// region, since it never reflects what the author actually wrote.
pub(super) fn compute(model: &SemanticModel) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    out.extend(unresolved_names(model));
    out.extend(unused_locals(model));
    out.extend(duplicate_definitions(model));
    out.extend(arity_mismatches(model));
    out.extend(misplaced_keywords(model));
    out.extend(rust_interop_info(model));
    out
}

fn is_blanked(model: &SemanticModel, token: usize) -> bool {
    model
        .blanked_regions()
        .iter()
        .any(|&(s, e)| token >= s && token < e)
}

/// Names the parser recognizes specially at expression-parsing time
/// without the lexer ever producing a dedicated token for them, so they
/// reach this pass as ordinary (unbound) identifiers: the `True`/`False`/
/// `None` literals, and the `_` placeholder used in wildcard patterns,
/// partial application (`f$(1, _)`), and implicit lambdas (`(_ + 1)`).
fn is_language_constant(name: &str) -> bool {
    matches!(name, "True" | "False" | "None" | "_")
}

fn unresolved_names(model: &SemanticModel) -> Vec<Diagnostic> {
    let declared = global_nonlocal_declaration_tokens(model.ast(), model.tokens());
    let macro_headers = macro_invocation_header_tokens(model.ast(), model.tokens());
    let keyword_names = keyword_argument_name_tokens(model.tokens());
    model
        .references()
        .iter()
        .filter(|r| r.target == Resolution::Unresolved && r.kind != RefKind::Definition)
        .filter(|r| !is_language_constant(&r.name))
        .filter(|r| !is_blanked(model, r.token))
        .filter(|r| !declared.contains(&r.token))
        .filter(|r| !macro_headers.contains(&r.token))
        .filter(|r| !keyword_names.contains(&r.token))
        .map(|r| {
            diag(
                r.span,
                model,
                DiagnosticSeverity::WARNING,
                "E0100",
                format!("cannot find `{}` in this scope", r.name),
            )
        })
        .collect()
}

/// Token indices of every name named in a `global`/`nonlocal` statement:
/// these are declarations, not lookups, and are never bound to a `Symbol`
/// of their own (see `non_local_names` in `build.rs`), so the ordinary
/// token walk in `resolve.rs` has no way to tell them apart from a
/// genuinely unresolved identifier without this.
fn global_nonlocal_declaration_tokens(
    stmts: &[Stmt],
    tokens: &[Token],
) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    collect_global_nonlocal(stmts, tokens, &mut out);
    out
}

/// Token indices of every identifier in a `template`/`macro` *invocation*'s
/// own syntax — the main name's arguments and each branch keyword plus its
/// arguments (`repeat_task 3 times:` / `on complete handler:`) — as
/// opposed to the indented blocks under them, which are ordinary
/// statements. These read as plain identifier expressions structurally,
/// but the template/macro system expands them by purely textual
/// substitution (see the `template`/`macro` sections of the language
/// reference), not by resolving them as variables in the scope they
/// appear in, so they must not be checked as such.
/// The name tokens of keyword-style call arguments (`b` in `f(1, b=2)`).
/// The parser accepts `name=value` and passes `value` positionally, so
/// `name` is a label, not a reference. The `f(b=)` shorthand is excluded:
/// there the value *is* `b`, so it must still resolve.
fn keyword_argument_name_tokens(tokens: &[Token]) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    let mut brackets = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        match tok {
            Token::LParen | Token::LBracket | Token::LBrace => brackets.push(tok),
            Token::RParen | Token::RBracket | Token::RBrace => {
                brackets.pop();
            }
            Token::Ident(_)
                if matches!(brackets.last(), Some(Token::LParen))
                    && matches!(
                        tokens.get(i.wrapping_sub(1)),
                        Some(Token::LParen | Token::Comma)
                    )
                    && matches!(tokens.get(i + 1), Some(Token::Assign))
                    && !matches!(tokens.get(i + 2), Some(Token::RParen | Token::Comma) | None) =>
            {
                out.insert(i);
            }
            _ => {}
        }
    }
    out
}

fn macro_invocation_header_tokens(
    stmts: &[Stmt],
    tokens: &[Token],
) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    collect_macro_headers(stmts, tokens, &mut out);
    out
}

fn collect_macro_headers(
    stmts: &[Stmt],
    tokens: &[Token],
    out: &mut std::collections::HashSet<usize>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::MacroBlock {
                token_pos,
                branches,
                body,
                ..
            } => {
                let mut i = *token_pos;
                i = mark_header_run(tokens, i, out);
                i = skip_colon_and_block(tokens, i);
                for (_, _, branch_body) in branches {
                    while matches!(tokens.get(i), Some(Token::Newline)) {
                        i += 1;
                    }
                    if matches!(tokens.get(i), Some(Token::Ident(_))) {
                        out.insert(i);
                        i += 1;
                    }
                    i = mark_header_run(tokens, i, out);
                    i = skip_colon_and_block(tokens, i);
                    collect_macro_headers(branch_body, tokens, out);
                }
                collect_macro_headers(body, tokens, out);
            }
            Stmt::FunctionDef { body, .. } => collect_macro_headers(body, tokens, out),
            Stmt::If(_, body, else_body) => {
                collect_macro_headers(body, tokens, out);
                collect_macro_headers(else_body, tokens, out);
            }
            Stmt::While(_, body) | Stmt::MatchFor(_, _, body, _) => {
                collect_macro_headers(body, tokens, out)
            }
            Stmt::Match(_, arms, _) => {
                for (_, _, body) in arms {
                    collect_macro_headers(body, tokens, out);
                }
            }
            Stmt::ClassDef { body, .. }
            | Stmt::TraitDef(_, body, _)
            | Stmt::ImplBlock(_, _, body, _) => collect_macro_headers(body, tokens, out),
            _ => {}
        }
    }
}

/// Marks every `Ident` token from `i` up to (not including) the next
/// depth-0 `Colon`, returning that `Colon`'s index (or the end of the
/// stream if the invocation is malformed).
fn mark_header_run(
    tokens: &[Token],
    mut i: usize,
    out: &mut std::collections::HashSet<usize>,
) -> usize {
    let mut depth = 0i32;
    while i < tokens.len() {
        match &tokens[i] {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => depth -= 1,
            Token::Colon if depth == 0 => return i,
            Token::Ident(_) => {
                out.insert(i);
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// From a `Colon` index, skips it, any blank-line `Newline`s, and the
/// indented block that follows (if any), returning the index right after.
fn skip_colon_and_block(tokens: &[Token], colon: usize) -> usize {
    let mut i = colon + 1;
    while matches!(tokens.get(i), Some(Token::Newline)) {
        i += 1;
    }
    if matches!(tokens.get(i), Some(Token::Indent)) {
        i = matching_dedent(tokens, i) + 1;
    }
    i
}

fn matching_dedent(tokens: &[Token], indent_idx: usize) -> usize {
    let mut depth = 0i32;
    for (i, tok) in tokens.iter().enumerate().skip(indent_idx) {
        match tok {
            Token::Indent => depth += 1,
            Token::Dedent => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    tokens.len().saturating_sub(1)
}

fn collect_global_nonlocal(
    stmts: &[Stmt],
    tokens: &[Token],
    out: &mut std::collections::HashSet<usize>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Global(names, pos) | Stmt::Nonlocal(names, pos) => {
                let mut cursor = *pos;
                for name in names {
                    if let Some(i) = (cursor..tokens.len())
                        .find(|&i| matches!(&tokens[i], Token::Ident(n) if n == name))
                    {
                        out.insert(i);
                        cursor = i + 1;
                    }
                }
            }
            Stmt::FunctionDef { body, .. } => collect_global_nonlocal(body, tokens, out),
            Stmt::If(_, body, else_body) => {
                collect_global_nonlocal(body, tokens, out);
                collect_global_nonlocal(else_body, tokens, out);
            }
            Stmt::While(_, body) | Stmt::MatchFor(_, _, body, _) => {
                collect_global_nonlocal(body, tokens, out)
            }
            Stmt::Match(_, arms, _) => {
                for (_, _, body) in arms {
                    collect_global_nonlocal(body, tokens, out);
                }
            }
            Stmt::ClassDef { body, .. }
            | Stmt::TraitDef(_, body, _)
            | Stmt::ImplBlock(_, _, body, _) => collect_global_nonlocal(body, tokens, out),
            _ => {}
        }
    }
}

/// Parameters and locals (not module-level variables, fields, or anything
/// `_`-prefixed) that are never read after their own definition.
fn unused_locals(model: &SemanticModel) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for sym in model.symbols() {
        if !matches!(sym.kind, SymbolKind::Parameter | SymbolKind::Variable) {
            continue;
        }
        if sym.scope == model.module_scope() {
            continue;
        }
        if sym.name.starts_with('_') {
            continue;
        }
        if is_blanked(model, sym.def_token) {
            continue;
        }
        let used = model
            .references_to(sym.id)
            .iter()
            .any(|r| r.kind == RefKind::Read);
        if !used {
            let mut d = diag(
                sym.name_span,
                model,
                DiagnosticSeverity::HINT,
                "W0200",
                format!("`{}` is never used", sym.name),
            );
            d.tags = Some(vec![DiagnosticTag::UNNECESSARY]);
            out.push(d);
        }
    }
    out
}

/// `def`/`data`/`struct`/`trait`/`class`/`template`/`macro` names that
/// collide with an earlier one in the same scope. `addpattern def`
/// (multiple-dispatch overloads) is explicitly not a collision.
fn duplicate_definitions(model: &SemanticModel) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    walk_duplicates(model.ast(), model, &mut out);
    out
}

fn walk_duplicates(stmts: &[Stmt], model: &SemanticModel, out: &mut Vec<Diagnostic>) {
    let mut seen: HashMap<&str, ()> = HashMap::new();
    for stmt in stmts {
        match stmt {
            Stmt::FunctionDef {
                name,
                is_addpattern,
                id,
                body,
                ..
            } => {
                if let Some(fn_name) = name.last() {
                    if *is_addpattern {
                        // Explicit multiple-dispatch overload: never a
                        // collision, and doesn't itself become the
                        // "first" definition other overloads collide with.
                    } else if seen.contains_key(fn_name.as_str()) {
                        if !is_blanked(model, *id) {
                            let span = definition_name_span(model, *id, fn_name);
                            out.push(diag(
                                span,
                                model,
                                DiagnosticSeverity::WARNING,
                                "E0101",
                                format!("`{fn_name}` is defined more than once in this scope"),
                            ));
                        }
                    } else {
                        seen.insert(fn_name.as_str(), ());
                    }
                }
                walk_duplicates(body, model, out);
            }
            Stmt::DataDef(name, _, _)
            | Stmt::StructDef(name, _, _)
            | Stmt::TraitDef(name, _, _)
            | Stmt::TemplateDef(name, _, _, _) => {
                check_simple_duplicate(name, stmt_pos(stmt), model, &mut seen, out);
            }
            Stmt::ClassDef { name, body, .. } => {
                check_simple_duplicate(name, stmt_pos(stmt), model, &mut seen, out);
                walk_duplicates(body, model, out);
            }
            Stmt::MacroDef { name, pos, .. } => {
                check_simple_duplicate(name, Some(*pos), model, &mut seen, out);
            }
            _ => {}
        }
    }
}

fn stmt_pos(stmt: &Stmt) -> Option<usize> {
    match stmt {
        Stmt::DataDef(_, _, id)
        | Stmt::StructDef(_, _, id)
        | Stmt::TraitDef(_, _, id)
        | Stmt::TemplateDef(_, _, _, id)
        | Stmt::ClassDef { id, .. } => Some(*id),
        _ => None,
    }
}

fn check_simple_duplicate<'a>(
    name: &'a str,
    pos: Option<usize>,
    model: &SemanticModel,
    seen: &mut HashMap<&'a str, ()>,
    out: &mut Vec<Diagnostic>,
) {
    let Some(pos) = pos else { return };
    if seen.contains_key(name) {
        if !is_blanked(model, pos) {
            let span = definition_name_span(model, pos, name);
            out.push(diag(
                span,
                model,
                DiagnosticSeverity::WARNING,
                "E0101",
                format!("`{name}` is defined more than once in this scope"),
            ));
        }
    } else {
        seen.insert(name, ());
    }
}

fn definition_name_span(model: &SemanticModel, def_token: usize, name: &str) -> Span {
    let tokens = model.tokens();
    let spans = model.spans();
    (def_token..tokens.len())
        .find(|&i| matches!(&tokens[i], Token::Ident(n) if n == name))
        .map(|i| spans[i])
        .unwrap_or(spans[def_token.min(spans.len() - 1)])
}

/// Calls to a known, non-variadic `def`/method with too few required
/// arguments or more than it accepts. Skipped whenever the call can't be
/// counted statically (a splat argument) or the callee takes `*args`.
fn arity_mismatches(model: &SemanticModel) -> Vec<Diagnostic> {
    let tokens = model.tokens();
    let mut out = Vec::new();
    for r in model.references() {
        if r.kind == RefKind::Definition || is_blanked(model, r.token) {
            continue;
        }
        let Resolution::Symbol(id) = &r.target else {
            continue;
        };
        let sym = model.symbol(*id);
        if !matches!(sym.kind, SymbolKind::Function | SymbolKind::Method) {
            continue;
        }
        let Some(sig) = &sym.signature else { continue };
        if sig.vararg.is_some() {
            continue;
        }
        if !matches!(tokens.get(r.token + 1), Some(Token::LParen)) {
            continue;
        }
        let Some(count) = count_call_args(tokens, r.token + 1) else {
            continue;
        };
        let required = sig.params.iter().filter(|p| p.default.is_none()).count();
        let max = sig.params.len();
        if count < required || count > max {
            let expected = if required == max {
                format!("{required}")
            } else {
                format!("{required} to {max}")
            };
            out.push(diag(
                r.span,
                model,
                DiagnosticSeverity::ERROR,
                "E0102",
                format!(
                    "`{}` expects {} argument{}, found {}",
                    sym.name,
                    expected,
                    if max == 1 { "" } else { "s" },
                    count
                ),
            ));
        }
    }
    out
}

/// Counts the top-level (bracket-depth-aware) arguments in a call whose
/// `(` is at `lparen`, or `None` if the count can't be known statically (a
/// `*`/`**` splat argument) or the parens are unbalanced.
fn count_call_args(tokens: &[Token], lparen: usize) -> Option<usize> {
    // Start just past the call's own opening paren, already "inside" it
    // (depth 1), so that paren itself is never mistaken for a token
    // belonging to the first argument (which would make `f()` count as
    // one empty argument instead of zero).
    let mut depth = 1i32;
    let mut count = 0usize;
    let mut saw_token_since_comma = false;
    let mut i = lparen + 1;
    loop {
        let tok = tokens.get(i)?;
        match tok {
            Token::LParen | Token::LBracket | Token::LBrace => {
                depth += 1;
                saw_token_since_comma = true;
            }
            Token::RParen if depth == 1 => {
                if saw_token_since_comma {
                    count += 1;
                }
                return Some(count);
            }
            Token::RBracket | Token::RBrace | Token::RParen => {
                depth -= 1;
                saw_token_since_comma = true;
            }
            Token::Comma if depth == 1 => {
                if saw_token_since_comma {
                    count += 1;
                }
                saw_token_since_comma = false;
            }
            Token::Star | Token::Power if depth == 1 && !saw_token_since_comma => {
                // `*args`/`**kwargs`-style splat at an argument position:
                // the call's arity can't be determined statically.
                return None;
            }
            Token::Newline | Token::Indent | Token::Dedent | Token::Eof => {
                saw_token_since_comma = true;
            }
            _ => saw_token_since_comma = true,
        }
        i += 1;
    }
}

/// `return` with no enclosing function, `break`/`continue` with no
/// enclosing loop, `nonlocal` with no enclosing function. `yield` is never
/// flagged: any function containing one is already inferred to be a
/// generator by the parser, `def` or `gen` alike.
fn misplaced_keywords(model: &SemanticModel) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    walk_keywords(model.ast(), model, false, false, &mut cursor, &mut out);
    out
}

fn walk_keywords(
    stmts: &[Stmt],
    model: &SemanticModel,
    in_function: bool,
    in_loop: bool,
    cursor: &mut usize,
    out: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Return(_) if !in_function => {
                report_keyword(
                    model,
                    &Token::Return,
                    cursor,
                    "E0103",
                    "`return` outside a function",
                    out,
                );
            }
            Stmt::Break if !in_loop => {
                report_keyword(
                    model,
                    &Token::Break,
                    cursor,
                    "E0104",
                    "`break` outside a loop",
                    out,
                );
            }
            Stmt::Continue if !in_loop => {
                report_keyword(
                    model,
                    &Token::Continue,
                    cursor,
                    "E0104",
                    "`continue` outside a loop",
                    out,
                );
            }
            Stmt::Nonlocal(_, pos) if !in_function => {
                if !is_blanked(model, *pos) {
                    out.push(diag(
                        model.spans()[*pos],
                        model,
                        DiagnosticSeverity::ERROR,
                        "E0105",
                        "`nonlocal` outside a function".to_string(),
                    ));
                }
            }
            Stmt::FunctionDef { body, .. } => walk_keywords(body, model, true, false, cursor, out),
            Stmt::While(_, body) => walk_keywords(body, model, in_function, true, cursor, out),
            Stmt::MatchFor(_, _, body, _) => {
                walk_keywords(body, model, in_function, true, cursor, out)
            }
            Stmt::If(_, body, else_body) => {
                walk_keywords(body, model, in_function, in_loop, cursor, out);
                walk_keywords(else_body, model, in_function, in_loop, cursor, out);
            }
            Stmt::Match(_, arms, _) => {
                for (_, _, body) in arms {
                    walk_keywords(body, model, in_function, in_loop, cursor, out);
                }
            }
            Stmt::ClassDef { body, .. }
            | Stmt::TraitDef(_, body, _)
            | Stmt::ImplBlock(_, _, body, _) => {
                walk_keywords(body, model, false, false, cursor, out);
            }
            Stmt::TemplateDef(_, _, body, _) | Stmt::MacroDef { body, .. } => {
                // Textually inlined at every call site; checking their
                // body against the *template's* context (rather than each
                // call site's) would risk false positives either way, so
                // this conservatively never flags inside one.
                let _ = body;
            }
            _ => {}
        }
    }
}

fn report_keyword(
    model: &SemanticModel,
    target: &Token,
    cursor: &mut usize,
    code: &str,
    message: &str,
    out: &mut Vec<Diagnostic>,
) {
    let tokens = model.tokens();
    if let Some(i) = (*cursor..tokens.len()).find(|&i| &tokens[i] == target) {
        *cursor = i + 1;
        if !is_blanked(model, i) {
            out.push(diag(
                model.spans()[i],
                model,
                DiagnosticSeverity::ERROR,
                code,
                message.to_string(),
            ));
        }
    }
}

fn rust_interop_info(model: &SemanticModel) -> Vec<Diagnostic> {
    model
        .ast()
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Passthrough(_, pos) if !is_blanked(model, *pos) => Some(diag(
                model.spans()[*pos],
                model,
                DiagnosticSeverity::INFORMATION,
                "I0300",
                "inline Rust runs under `craw run`/`craw build` only".to_string(),
            )),
            _ => None,
        })
        .collect()
}
