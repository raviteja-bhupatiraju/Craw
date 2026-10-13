//! `textDocument/selectionRange` ("Expand Selection"): from the token under
//! the cursor outward through bracket contents, brackets, the logical
//! statement, the enclosing indented block, the block with its header, and
//! finally the whole document.

use crate::lexer::Token;
use crate::semantic::SemanticModel;
use crate::text::Span;
use tower_lsp::lsp_types::SelectionRange;

/// Every candidate span containing `offset`, innermost first, each
/// strictly containing the previous one.
pub fn selection_spans(model: &SemanticModel, offset: usize) -> Vec<Span> {
    let tokens = model.tokens();
    let spans = model.spans();
    let contains = |s: Span| s.start <= offset && offset <= s.end;
    let mut cands: Vec<Span> = Vec::new();

    // 1. The token itself.
    for (i, t) in tokens.iter().enumerate() {
        let s = spans[i];
        if !super::is_marker(t) && s.start <= offset && offset < s.end.max(s.start + 1) {
            cands.push(s);
        }
    }

    // 2. Bracket contents and whole brackets.
    for (i, t) in tokens.iter().enumerate() {
        if !matches!(t, Token::LParen | Token::LBracket | Token::LBrace) {
            continue;
        }
        let Some(close) = super::matching_close(tokens, i) else {
            continue;
        };
        let outer = Span::new(spans[i].start, spans[close].end);
        if !contains(outer) {
            continue;
        }
        if close > i + 1 {
            cands.push(Span::new(spans[i + 1].start, spans[close - 1].end));
        }
        cands.push(outer);
    }

    // 3. Logical statements: token runs between layout markers.
    let statements = statement_spans(model);
    cands.extend(statements.iter().copied().filter(|s| contains(*s)));

    // 4. Indented blocks, alone and with their header statement.
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::Indent => stack.push(i),
            Token::Dedent | Token::Eof => {
                let closes = if matches!(t, Token::Eof) {
                    stack.len()
                } else {
                    1
                };
                for _ in 0..closes {
                    let Some(open) = stack.pop() else { break };
                    let body = real_extent(model, open + 1, i);
                    let Some(body) = body else { continue };
                    if contains(body) {
                        cands.push(body);
                    }
                    if let Some(header) =
                        statements.iter().rev().find(|s| s.end <= spans[open].start)
                    {
                        let whole = Span::new(header.start, body.end);
                        if contains(whole) {
                            cands.push(whole);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // 5. The whole document.
    cands.push(Span::new(0, model.text().len()));

    cands.sort_by_key(|s| (s.len(), s.start));
    let mut chain: Vec<Span> = Vec::new();
    for c in cands {
        match chain.last() {
            Some(prev) if !(c.start <= prev.start && prev.end <= c.end) || c == *prev => {}
            _ => chain.push(c),
        }
    }
    chain
}

/// The span from the first to the last non-marker token in `from..to`,
/// trimmed of a trailing newline a multi-line token may carry.
fn real_extent(model: &SemanticModel, from: usize, to: usize) -> Option<Span> {
    let tokens = model.tokens();
    let spans = model.spans();
    let first = (from..to).find(|&i| !super::is_marker(&tokens[i]))?;
    let last = (from..to).rev().find(|&i| !super::is_marker(&tokens[i]))?;
    let text = model.text();
    let mut end = spans[last].end;
    while end > spans[first].start && text.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    Some(Span::new(spans[first].start, end))
}

/// Spans of every logical statement (a maximal run of real tokens with no
/// layout marker in between; brackets never contain markers).
pub(crate) fn statement_spans(model: &SemanticModel) -> Vec<Span> {
    let tokens = model.tokens();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..=tokens.len() {
        let is_break = i == tokens.len() || super::is_marker(&tokens[i]);
        if is_break {
            if let Some(s) = start.take()
                && let Some(span) = real_extent(model, s, i)
            {
                out.push(span);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    out
}

pub fn selection_range(model: &SemanticModel, offset: usize) -> SelectionRange {
    let li = model.line_index();
    let mut result: Option<SelectionRange> = None;
    for span in selection_spans(model, offset).into_iter().rev() {
        result = Some(SelectionRange {
            range: li.range(span),
            parent: result.map(Box::new),
        });
    }
    result.unwrap_or(SelectionRange {
        range: li.range(Span::new(offset, offset)),
        parent: None,
    })
}
