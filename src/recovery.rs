//! Parse-error recovery for the LSP: on a syntax error, blank out the
//! smallest enclosing region of the token stream — never changing the
//! token *count*, so every position already recorded in a partially-parsed
//! AST stays a valid index — and retry, so one bad statement doesn't take
//! down semantic analysis for the rest of the file.
//!
//! The region starts as "this line" (the tokens between the nearest
//! enclosing `Newline`/`Indent`/`Dedent` and the line's own terminating
//! `Newline`). If that line was a block header (its last real token before
//! the `Newline` was `:`, and an `Indent` immediately follows), blanking it
//! away without also blanking the block it introduces would orphan that
//! `Indent` — there would be no `:` immediately before it any more — so the
//! region is extended forward through the block's matching `Dedent` first.
//! This repeats (a header can itself be followed by another header-shaped
//! line, e.g. a chain of blanked sibling statements) until stable. Because
//! `Indent`/`Dedent` are always nested and paired by construction in the
//! lexer's output, a region built this way is always exactly one or more
//! *complete* statements — never a partial block — so blanking it can never
//! desynchronize indentation for anything outside it.

use crate::ast::Stmt;
use crate::diagnostics::{SyntaxError, render_parse_error};
use crate::lexer::{Lexer, Token};
use crate::parser;
use crate::text::Span;
use chumsky::Parser;

/// Number of blank-and-retry iterations to attempt before giving up on a
/// document and returning whatever partial AST the last attempt produced.
pub const MAX_RETRIES: usize = 20;

/// Result of parsing a document with recovery.
#[derive(Debug)]
pub struct Recovered {
    /// The original, unmodified token stream. Every token-index position
    /// recorded anywhere in `ast` indexes into this (not into whatever
    /// blanked copy recovery used internally).
    pub tokens: Vec<Token>,
    /// The original, unmodified per-token byte spans, parallel to `tokens`.
    pub spans: Vec<Span>,
    /// The best AST obtained: exactly what a normal parse would produce if
    /// `errors` is empty, otherwise parsed from a copy of the token stream
    /// with every broken region blanked to `Newline`s.
    pub ast: Vec<Stmt>,
    /// One entry per blank-and-retry iteration that hit an error (so, empty
    /// for a clean document). Capped at `MAX_RETRIES + 1`.
    pub errors: Vec<SyntaxError>,
    /// Half-open `[start, end)` *token-index* ranges that were blanked, in
    /// the order they were found. A semantic pass should not report
    /// "unresolved name" or similar diagnostics for anything in these,
    /// since they never reflect what the author actually wrote.
    pub blanked: Vec<(usize, usize)>,
}

/// Lexes and parses `text`, recovering from syntax errors by blanking the
/// smallest enclosing statement(s) and retrying. Always returns some AST
/// (empty if nothing could be salvaged) and never panics on malformed
/// input.
pub fn parse_with_recovery(text: &str) -> Recovered {
    let mut lexer = Lexer::new(text);
    let (tokens, spans) = lexer.tokenize_spanned();
    let mut work = tokens.clone();
    let mut errors = Vec::new();
    let mut blanked = Vec::new();
    let mut ast = Vec::new();

    for attempt in 0..=MAX_RETRIES {
        let (out, parse_errors) = parser::parser().parse(&work).into_output_errors();
        if parse_errors.is_empty() {
            ast = out.unwrap_or_default();
            break;
        }
        ast = out.unwrap_or_default();
        let err = &parse_errors[0];
        errors.push(render_parse_error(err, &spans, text.len()));

        if attempt == MAX_RETRIES {
            // Out of retries; keep whatever partial AST this last attempt
            // produced and stop rather than blanking further.
            break;
        }

        let err_idx = err.span().start.min(work.len().saturating_sub(1));
        match blank_region(&mut work, err_idx) {
            Some(region) => blanked.push(region),
            // Nothing blankable left (the error is at the end of input).
            None => break,
        }
    }

    Recovered {
        tokens,
        spans,
        ast,
        errors,
        blanked,
    }
}

/// Finds the smallest region around `err_idx` that can be safely replaced
/// with `Newline` tokens without desynchronizing indentation elsewhere,
/// replaces it in `work`, and returns its `[start, end)` bounds.
fn blank_region(work: &mut [Token], err_idx: usize) -> Option<(usize, usize)> {
    let start = line_start(work, err_idx);
    // One past the line's own terminating `Newline` (or the last token in
    // `work`, if the line runs off the end of the stream).
    let mut end = (line_end(work, err_idx) + 1).min(work.len());

    while matches!(work.get(end), Some(Token::Indent)) {
        end = (matching_dedent(work, end) + 1).min(work.len());
    }

    // Defensive: guarantee forward progress even in a degenerate case
    // (e.g. an error pinned at a lone trailing token) so recovery can never
    // spin without blanking anything.
    if end <= start {
        end = (start + 1).min(work.len());
    }
    // Never blank the final `Eof`: without it no retry can ever parse, so
    // an unfinished last statement (`f(1, ` while typing) would cost the
    // whole file's AST instead of just that statement.
    if matches!(work.get(end.wrapping_sub(1)), Some(Token::Eof)) {
        end -= 1;
    }
    if end <= start || work[start..end].iter().all(|t| matches!(t, Token::Newline)) {
        return None;
    }

    for tok in &mut work[start..end] {
        *tok = Token::Newline;
    }
    Some((start, end))
}

/// Index of the first token of the line containing `idx`: the token right
/// after the nearest preceding `Newline`/`Indent`/`Dedent`, or `0` if `idx`
/// is on the first line.
fn line_start(work: &[Token], idx: usize) -> usize {
    let mut i = idx;
    while i > 0 {
        i -= 1;
        if matches!(work[i], Token::Newline | Token::Indent | Token::Dedent) {
            return i + 1;
        }
    }
    0
}

/// Index of the line's own terminating `Newline`, scanning forward from
/// `idx` (inclusive, so an error *at* a `Newline` returns that same
/// index). Every `Newline` the lexer emits already sits at bracket depth
/// 0 — it never emits one inside an open `(`/`[`/`{` — so no depth
/// tracking is needed here. Falls back to the last index in `work` (the
/// `Eof`) if no `Newline` follows.
fn line_end(work: &[Token], idx: usize) -> usize {
    let mut i = idx;
    while i < work.len() {
        if matches!(work[i], Token::Newline) {
            return i;
        }
        i += 1;
    }
    work.len().saturating_sub(1)
}

/// Given the index of an `Indent`, finds its matching `Dedent` by counting
/// nesting depth forward. Every `Indent` the lexer emits is paired with a
/// `Dedent`, so this always finds one; if it somehow didn't, that would be
/// a lexer bug worth panicking loudly on rather than silently misbehaving.
fn matching_dedent(work: &[Token], indent_idx: usize) -> usize {
    let mut depth = 0i32;
    for (i, tok) in work.iter().enumerate().skip(indent_idx) {
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
    panic!("unbalanced Indent at token {indent_idx}: no matching Dedent (lexer bug)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Pattern;

    fn rec(src: &str) -> Recovered {
        parse_with_recovery(src)
    }

    #[test]
    fn clean_input_has_no_errors() {
        let r = rec("x = 1\n");
        assert!(r.errors.is_empty());
        assert_eq!(r.ast.len(), 1);
    }

    #[test]
    fn bad_line_inside_function_keeps_function_and_siblings() {
        let r = rec("def f():\n    x = \n    y = 1\nz = 2\n");
        assert_eq!(r.errors.len(), 1);
        let names: Vec<_> = r
            .ast
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(name.join(".")),
                Stmt::Assign(Pattern::Var(n, _), _, _) => Some(n.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["f".to_string(), "z".to_string()]);
        let Stmt::FunctionDef { body, .. } = &r.ast[0] else {
            panic!("expected FunctionDef, got {:?}", r.ast[0])
        };
        assert!(matches!(&body[0], Stmt::Assign(Pattern::Var(n, _), _, _) if n == "y"));
    }

    #[test]
    fn broken_def_header_drops_only_that_def() {
        // A missing function name (rather than a missing `)`) so the
        // header's error doesn't leave a bracket unclosed — an unclosed
        // `(`/`[`/`{` makes this lexer stop emitting Newline/Indent/Dedent
        // for the rest of the file (see Lexer::tokenize's nesting_level
        // guards), collapsing everything after it into one line with no
        // structure left to recover along. That's a lexer property, not
        // something token-level recovery can work around.
        let r = rec("def :\n    x = 1\ndef g():\n    return 2\n");
        assert_eq!(r.errors.len(), 1);
        assert!(
            matches!(&r.ast[0], Stmt::FunctionDef { name, .. } if name == &vec!["g".to_string()])
        );
    }

    #[test]
    fn recovered_positions_still_index_the_original_tokens() {
        let r = rec("x = \ny = 2\n");
        let Stmt::Assign(_, _, pos) = &r.ast[0] else {
            panic!("expected Assign, got {:?}", r.ast[0])
        };
        assert!(matches!(&r.tokens[*pos], Token::Ident(n) if n == "y"));
    }

    #[test]
    fn an_unfinished_last_statement_keeps_everything_before_it() {
        // An unclosed call at end of input: the lexer emits no `Newline`
        // before `Eof`, so the broken line runs into it.
        let r = rec("from geometry import area\nx = 1\narea(1, \n");
        assert!(!r.errors.is_empty());
        assert_eq!(r.ast.len(), 2, "{:?}", r.ast);
        assert!(matches!(r.ast[0], Stmt::FromImport(..)));
        assert_eq!(r.blanked.len(), 1, "one region, not a retry per attempt");
    }

    #[test]
    fn gives_up_after_limit_without_panicking() {
        let src = "x = \n".repeat(50);
        let r = rec(&src);
        assert!(!r.errors.is_empty() && r.errors.len() <= MAX_RETRIES + 1);
    }
}
