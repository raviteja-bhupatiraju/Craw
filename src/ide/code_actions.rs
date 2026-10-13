//! `textDocument/codeAction`: quick fixes for the semantic model's own
//! diagnostics, plus the refactors that make sense for a dynamically-typed
//! indentation language (annotate a binding with its inferred type, extract
//! an expression into a variable, inline a variable).

use crate::ast::{Pattern, Stmt};
use crate::lexer::Token;
use crate::lsp_analyzer::infer_expr_type;
use crate::semantic::{RefKind, Resolution, ScopeId, SemanticModel, SymbolId, SymbolKind};
use crate::text::Span;
use std::collections::HashMap;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Diagnostic, NumberOrString, TextEdit, Url, WorkspaceEdit,
};

pub fn code_actions(
    model: &SemanticModel,
    uri: &Url,
    range: Span,
    diagnostics: &[Diagnostic],
) -> Vec<CodeAction> {
    let mut out = Vec::new();
    let li = model.line_index();
    for d in diagnostics {
        let Some(start) = li.offset(d.range.start.into()) else {
            continue;
        };
        let Some(end) = li.offset(d.range.end.into()) else {
            continue;
        };
        let span = Span::new(start, end);
        match d.code.as_ref() {
            Some(NumberOrString::String(c)) if c == "W0200" => {
                out.extend(prefix_unused(model, uri, span, d));
            }
            Some(NumberOrString::String(c)) if c == "E0100" => {
                out.extend(did_you_mean(model, uri, span, d));
            }
            _ => {}
        }
    }
    out.extend(add_annotation(model, uri, range));
    out.extend(extract_variable(model, uri, range));
    out.extend(inline_variable(model, uri, range));
    out
}

fn edit(uri: &Url, edits: Vec<TextEdit>) -> WorkspaceEdit {
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }
}

fn action(
    title: String,
    kind: CodeActionKind,
    uri: &Url,
    edits: Vec<TextEdit>,
    diagnostic: Option<&Diagnostic>,
    preferred: bool,
) -> CodeAction {
    CodeAction {
        title,
        kind: Some(kind),
        diagnostics: diagnostic.map(|d| vec![d.clone()]),
        edit: Some(edit(uri, edits)),
        command: None,
        is_preferred: preferred.then_some(true),
        disabled: None,
        data: None,
    }
}

fn symbol_defined_at(model: &SemanticModel, span: Span) -> Option<SymbolId> {
    model.references().iter().find_map(|r| match r.target {
        Resolution::Symbol(id) if r.span == span => Some(id),
        _ => None,
    })
}

fn rename_edits(model: &SemanticModel, id: SymbolId, new_name: &str) -> Vec<TextEdit> {
    model
        .references_to(id)
        .into_iter()
        .map(|r| TextEdit {
            range: model.line_index().range(r.span),
            new_text: new_name.to_string(),
        })
        .collect()
}

/// `W0200` (unused): rename the binding to `_name` everywhere, which is
/// the documented way to mark a binding as intentionally unused.
fn prefix_unused(
    model: &SemanticModel,
    uri: &Url,
    span: Span,
    d: &Diagnostic,
) -> Option<CodeAction> {
    let id = symbol_defined_at(model, span)?;
    let name = &model.symbol(id).name;
    let new_name = format!("_{name}");
    Some(action(
        format!("Prefix `{name}` with an underscore"),
        CodeActionKind::QUICKFIX,
        uri,
        rename_edits(model, id, &new_name),
        Some(d),
        true,
    ))
}

/// Levenshtein distance, for "did you mean" suggestions.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Every name visible by lookup from `scope` (walking outward), plus
/// builtins.
pub(crate) fn visible_names(model: &SemanticModel, scope: ScopeId) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut current = Some(scope);
    while let Some(id) = current {
        let s = &model.scopes()[id.0 as usize];
        names.extend(s.bindings.keys().cloned());
        current = s.parent;
    }
    names.extend(
        crate::transpiler::BUILTINS
            .iter()
            .map(|(n, _)| n.to_string()),
    );
    names.sort();
    names.dedup();
    names
}

/// `E0100` (unresolved): offer the closest visible names. A candidate
/// qualifies when it is within roughly a third of the name's length in
/// edit distance (at least 1, at most 3), or differs only in case.
fn did_you_mean(model: &SemanticModel, uri: &Url, span: Span, d: &Diagnostic) -> Vec<CodeAction> {
    let Some(name) = model.text().get(span.start..span.end) else {
        return Vec::new();
    };
    let budget = (name.chars().count() / 3).clamp(1, 3);
    let mut scored: Vec<(usize, String)> = visible_names(model, model.scope_at(span.start))
        .into_iter()
        .filter(|c| c != name)
        .filter_map(|c| {
            let dist = if c.eq_ignore_ascii_case(name) {
                0
            } else {
                edit_distance(name, &c)
            };
            (dist <= budget).then_some((dist, c))
        })
        .collect();
    scored.sort();
    scored.truncate(3);
    let best_unique = scored.len() == 1 || (scored.len() > 1 && scored[0].0 < scored[1].0);
    scored
        .into_iter()
        .enumerate()
        .map(|(i, (_, c))| {
            action(
                format!("Change to `{c}`"),
                CodeActionKind::QUICKFIX,
                uri,
                vec![TextEdit {
                    range: model.line_index().range(span),
                    new_text: c,
                }],
                Some(d),
                i == 0 && best_unique,
            )
        })
        .collect()
}

/// The unannotated `name = expr` statement whose name token the range
/// touches, with the statement's token index.
fn assignment_at(model: &SemanticModel, range: Span) -> Option<(&Stmt, usize)> {
    let mut found = None;
    super::walk_stmts(model.ast(), &mut |stmt| {
        if let Stmt::Assign(Pattern::Var(name, None), _, tok) = stmt
            && let Some(Token::Ident(n)) = model.tokens().get(*tok)
            && n == name
            && super::overlaps(model.spans()[*tok], range)
        {
            found = Some((stmt, *tok));
        }
    });
    found
}

/// `x = expr` → `x: Type = expr` when `expr`'s type is inferable.
fn add_annotation(model: &SemanticModel, uri: &Url, range: Span) -> Option<CodeAction> {
    let (stmt, tok) = assignment_at(model, range)?;
    let Stmt::Assign(_, expr, _) = stmt else {
        return None;
    };
    let is_definition = model
        .references()
        .iter()
        .any(|r| r.token == tok && r.kind == RefKind::Definition);
    if !is_definition {
        return None;
    }
    let span = model.spans()[tok];
    let t = infer_expr_type(model, expr, span.start, 0)?;
    let ty = super::type_name(t);
    Some(action(
        format!("Add explicit type annotation `{ty}`"),
        CodeActionKind::REFACTOR_REWRITE,
        uri,
        vec![TextEdit {
            range: model.line_index().range(Span::new(span.end, span.end)),
            new_text: format!(": {ty}"),
        }],
        None,
        false,
    ))
}

/// Tokens whose spans lie entirely within `range`, as an index range —
/// but only when `range` starts and ends exactly on token boundaries.
fn tokens_exactly_covering(model: &SemanticModel, range: Span) -> Option<(usize, usize)> {
    let spans = model.spans();
    let tokens = model.tokens();
    let text = model.text();
    // Tolerate surrounding whitespace in the selection.
    let mut start = range.start;
    let mut end = range.end;
    while start < end && text.as_bytes()[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && text.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let first =
        (0..tokens.len()).find(|&i| !super::is_marker(&tokens[i]) && spans[i].start == start)?;
    let last = (first..tokens.len()).find(|&i| spans[i].end == end)?;
    if (first..=last).any(|i| super::is_marker(&tokens[i])) {
        return None;
    }
    Some((first, last))
}

fn balanced(tokens: &[Token]) -> bool {
    let mut depth = 0i32;
    for t in tokens {
        match t {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// Whether a token can't be part of a standalone expression (statement
/// syntax, or an assignment operator).
fn statement_only(t: &Token) -> bool {
    matches!(
        t,
        Token::Assign
            | Token::PlusAssign
            | Token::MinusAssign
            | Token::StarAssign
            | Token::SlashAssign
            | Token::PowerAssign
            | Token::Colon
            | Token::Semicolon
            | Token::Def
            | Token::Gen
            | Token::Return
            | Token::Yield
            | Token::Data
            | Token::Class
            | Token::Match
            | Token::Case
            | Token::While
            | Token::For
            | Token::Break
            | Token::Continue
            | Token::Global
            | Token::Nonlocal
            | Token::Import
    )
}

/// Whether a token run contains statement syntax. Inside brackets,
/// `:`/`=`/`for` are dict entries, slices, keyword arguments and
/// comprehensions, so only depth-0 tokens count.
fn has_statement_syntax(tokens: &[Token]) -> bool {
    let mut depth = 0i32;
    for t in tokens {
        match t {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => depth -= 1,
            // Raw Rust passthrough is opaque.
            Token::PassthroughStart | Token::Rust => return true,
            t if depth == 0 && statement_only(t) => return true,
            _ => {}
        }
    }
    false
}

/// The statement containing token `idx`: `(first token, leading
/// indentation)`.
fn enclosing_statement(model: &SemanticModel, idx: usize) -> (usize, String) {
    let tokens = model.tokens();
    let first = (0..idx)
        .rev()
        .find(|&i| super::is_marker(&tokens[i]))
        .map_or(0, |i| i + 1);
    let li = model.line_index();
    let line = li.position(model.spans()[first].start).line;
    let text = li.line_text(line);
    let indent: String = text
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    (first, indent)
}

fn fresh_name(model: &SemanticModel, scope: ScopeId, base: &str) -> String {
    let taken = visible_names(model, scope);
    if !taken.iter().any(|n| n == base) {
        return base.to_string();
    }
    (1..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken.contains(n))
        .unwrap()
}

/// Selected expression → `value = <expr>` on its own line before the
/// statement, and the selection replaced by `value`. Refused when the
/// selection isn't a whole balanced run of expression tokens within one
/// statement, or when it sits in a narrower scope than the statement
/// (inside a lambda or comprehension, whose bindings the new line
/// couldn't see).
fn extract_variable(model: &SemanticModel, uri: &Url, range: Span) -> Option<CodeAction> {
    if range.is_empty() {
        return None;
    }
    let (first, last) = tokens_exactly_covering(model, range)?;
    let tokens = model.tokens();
    let slice = &tokens[first..=last];
    if !balanced(slice) || has_statement_syntax(slice) {
        return None;
    }
    let (stmt_first, indent) = enclosing_statement(model, first);
    // The whole right-hand side of `x = ...` or the whole statement is
    // already a statement of its own; extracting it would add nothing.
    if first == stmt_first {
        return None;
    }
    let spans = model.spans();
    let stmt_scope = model.scope_at(spans[stmt_first].start);
    if model.scope_at(spans[first].start) != stmt_scope
        || model.scope_at(spans[last].start) != stmt_scope
    {
        return None;
    }
    let expr_span = Span::new(spans[first].start, spans[last].end);
    let expr_text = &model.text()[expr_span.start..expr_span.end];
    let name = fresh_name(model, stmt_scope, "value");
    let li = model.line_index();
    let line = li.position(spans[stmt_first].start).line;
    let insert_at = tower_lsp::lsp_types::Position { line, character: 0 };
    Some(action(
        "Extract into variable".to_string(),
        CodeActionKind::REFACTOR_EXTRACT,
        uri,
        vec![
            TextEdit {
                range: tower_lsp::lsp_types::Range {
                    start: insert_at,
                    end: insert_at,
                },
                new_text: format!("{indent}{name} = {expr_text}\n"),
            },
            TextEdit {
                range: li.range(expr_span),
                new_text: name,
            },
        ],
        None,
        false,
    ))
}

/// `x = expr` used later → every read of `x` replaced by `expr`
/// (parenthesized unless it is a single token), and the definition line
/// removed. Only offered when it can't change meaning: `x` is assigned
/// exactly once, and `expr` contains no calls and reads only names that
/// are never reassigned.
fn inline_variable(model: &SemanticModel, uri: &Url, range: Span) -> Option<CodeAction> {
    let (_, tok) = assignment_at(model, range).or_else(|| {
        // Cursor on a *use* of the variable: find its definition.
        let r = model
            .references()
            .iter()
            .find(|r| super::overlaps(r.span, range) && r.kind == RefKind::Read)?;
        let Resolution::Symbol(id) = r.target else {
            return None;
        };
        let def_span = model.symbol(id).name_span;
        assignment_at(model, def_span)
    })?;
    let spans = model.spans();
    let tokens = model.tokens();
    let id = symbol_defined_at(model, spans[tok])?;
    let sym = model.symbol(id);
    if sym.kind != SymbolKind::Variable {
        return None;
    }
    let refs = model.references_to(id);
    if refs.iter().any(|r| r.kind == RefKind::Write) {
        return None;
    }
    let reads: Vec<_> = refs.iter().filter(|r| r.kind == RefKind::Read).collect();
    if reads.is_empty() {
        return None;
    }
    // `tok` is the name, `tok + 1` the `=`; the initializer runs to the
    // statement's end.
    if !matches!(tokens.get(tok + 1), Some(Token::Assign)) {
        return None;
    }
    let init_first = tok + 2;
    let init_last = (init_first..tokens.len())
        .find(|&i| super::is_marker(&tokens[i]))?
        .checked_sub(1)?;
    if init_last < init_first {
        return None;
    }
    for i in init_first..=init_last {
        if matches!(tokens[i], Token::LParen)
            && i > init_first
            && matches!(tokens[i - 1], Token::Ident(_))
        {
            return None;
        }
        if matches!(tokens[i], Token::Lambda | Token::Arrow) {
            return None;
        }
    }
    if has_statement_syntax(&tokens[init_first..=init_last]) {
        return None;
    }
    for r in model.references() {
        if r.token < init_first || r.token > init_last {
            continue;
        }
        match r.target {
            Resolution::Symbol(other) => {
                if model
                    .references_to(other)
                    .iter()
                    .any(|x| x.kind == RefKind::Write)
                {
                    return None;
                }
            }
            Resolution::Unresolved => return None,
            _ => {}
        }
    }
    let text = model.text();
    let init = &text[spans[init_first].start..spans[init_last].end];
    let replacement = if init_first == init_last {
        init.to_string()
    } else {
        format!("({init})")
    };
    let li = model.line_index();
    let def_line = li.position(spans[tok].start).line;
    // The statement must be alone on its line for deleting that line to be
    // exactly deleting the statement.
    let line_text = li.line_text(def_line);
    if line_text.trim_start() != &text[spans[tok].start..spans[init_last].end] {
        return None;
    }
    let mut edits: Vec<TextEdit> = reads
        .iter()
        .map(|r| TextEdit {
            range: li.range(r.span),
            new_text: replacement.clone(),
        })
        .collect();
    edits.push(TextEdit {
        range: tower_lsp::lsp_types::Range {
            start: tower_lsp::lsp_types::Position {
                line: def_line,
                character: 0,
            },
            end: tower_lsp::lsp_types::Position {
                line: def_line + 1,
                character: 0,
            },
        },
        new_text: String::new(),
    });
    Some(action(
        format!("Inline variable `{}`", sym.name),
        CodeActionKind::REFACTOR_INLINE,
        uri,
        edits,
        None,
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::edit_distance;

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
        assert_eq!(edit_distance("prnt", "print"), 1);
    }
}
