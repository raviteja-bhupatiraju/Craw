//! `textDocument/formatting` and `rangeFormatting`: a conservative,
//! token-driven formatter.
//!
//! It never reflows code. Per line it (1) normalizes the spacing between
//! tokens (one space around binary operators and after commas, none inside
//! brackets or before call parentheses, ...), keeping the original spacing
//! for any pair no rule covers, and (2) strips trailing whitespace. Across
//! lines it collapses runs of more than two blank lines, drops leading
//! blank lines, and ends the file with exactly one newline.
//!
//! Safety: comments aren't tokens, so they are carried over verbatim, and
//! the result is re-lexed — any line whose respacing changed the token
//! stream is left with only the whitespace-only fixes. Lines covered by a
//! multi-line token (a `rust:` block, a multi-line string) and lines inside
//! a bracket that spans lines are never respaced.

use crate::lexer::{Lexer, Token};
use crate::semantic::SemanticModel;
use std::collections::HashSet;
use tower_lsp::lsp_types::{Position, Range, TextEdit};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Gap {
    Zero,
    One,
    Preserve,
}

fn is_open(t: &Token) -> bool {
    matches!(t, Token::LParen | Token::LBracket | Token::LBrace)
}

fn is_close(t: &Token) -> bool {
    matches!(t, Token::RParen | Token::RBracket | Token::RBrace)
}

/// Binary operators that always get one space on each side.
fn is_spaced_operator(t: &Token) -> bool {
    use Token::*;
    matches!(
        t,
        PlusAssign
            | MinusAssign
            | StarAssign
            | SlashAssign
            | PowerAssign
            | Equal
            | NotEqual
            | LessThan
            | GreaterThan
            | LessEqual
            | GreaterEqual
            | Plus
            | Slash
            | Percent
            | Arrow
            | ThinArrow
            | PipeForward
            | PipeBackward
            | PipeForwardStar
            | PipeForwardDoubleStar
            | PipeForwardNone
            | PipeForwardNoneStar
            | PipeForwardNoneDoubleStar
            | PipeBackwardStar
            | PipeBackwardDoubleStar
            | PipeBackwardNone
            | PipeBackwardNoneStar
            | PipeBackwardNoneDoubleStar
            | ComposeForward
            | ComposeBackward
            | ComposeStar
            | ComposeDoubleStar
            | ComposeNone
            | NoneCoalesce
            | DotPlus
            | DotMinus
            | DotStar
            | DotSlash
            | DotPercent
            | DotEqual
            | DotNotEqual
            | DotLess
            | DotLessEqual
            | DotGreater
            | DotGreaterEqual
    )
}

fn is_word_keyword(t: &Token) -> bool {
    use Token::*;
    matches!(
        t,
        Def | Return
            | If
            | Else
            | Data
            | Class
            | Extends
            | With
            | Match
            | Case
            | In
            | For
            | Where
            | Global
            | Nonlocal
            | Struct
            | Trait
            | Impl
            | From
            | Select
            | OrderBy
            | Ascending
            | Descending
            | Import
            | Yield
            | While
            | Then
            | Not
            | And
            | Or
            | Is
            | Template
            | Macro
            | Gen
            | As
            | Copyclosure
            | Addpattern
    )
}

/// Whether a token ends an operand (so a following `-`/`*` is binary and
/// a following `(`/`[` is a call/index).
fn ends_operand(t: &Token) -> bool {
    matches!(
        t,
        Token::Ident(_)
            | Token::Number(_)
            | Token::Float(_)
            | Token::String(_)
            | Token::FStringRaw(_)
            | Token::RParen
            | Token::RBracket
            | Token::RBrace
            | Token::Question
    )
}

/// The spacing between two adjacent tokens on one line. `prev_prev` is the
/// token before `prev` on the same line (for unary detection);
/// `innermost` the innermost open bracket, if any.
fn gap(
    prev_prev: Option<&Token>,
    prev: &Token,
    next: &Token,
    innermost: Option<&Token>,
    generic: (bool, bool),
) -> Gap {
    use Gap::*;
    // `List<Int>`: type-argument angles hug their neighbors.
    if generic.0 || generic.1 {
        return Zero;
    }
    let in_brackets = innermost.is_some();
    let in_square = matches!(innermost, Some(Token::LBracket));
    let unary = |t: &Token| {
        matches!(t, Token::Minus | Token::Star | Token::Power)
            && !prev_prev.is_some_and(ends_operand)
    };

    if is_open(prev) || is_close(next) {
        return Zero;
    }
    if matches!(next, Token::Comma | Token::Semicolon) {
        return Zero;
    }
    if matches!(next, Token::Colon) {
        return if in_square { Preserve } else { Zero };
    }
    if matches!(prev, Token::Colon) {
        return if in_square { Preserve } else { One };
    }
    if matches!(prev, Token::Comma | Token::Semicolon) {
        return One;
    }
    if matches!(prev, Token::Dot) {
        return Zero;
    }
    if matches!(next, Token::Dot) {
        return if ends_operand(prev) { Zero } else { Preserve };
    }
    if matches!(prev, Token::At) {
        return Zero;
    }
    if matches!(next, Token::LParen | Token::LBracket) && ends_operand(prev) {
        return Zero;
    }
    if unary(prev) {
        return Zero;
    }
    let assign_spaced = |t: &Token| matches!(t, Token::Assign) && !in_brackets;
    let binary_minus_star = |t: &Token, before: Option<&Token>| {
        matches!(t, Token::Minus | Token::Star | Token::Power) && before.is_some_and(ends_operand)
    };
    if matches!(prev, Token::Assign) || matches!(next, Token::Assign) {
        return if in_brackets { Zero } else { One };
    }
    if is_spaced_operator(prev)
        || is_spaced_operator(next)
        || assign_spaced(prev)
        || binary_minus_star(prev, prev_prev)
        || binary_minus_star(next, Some(prev))
    {
        return One;
    }
    if is_word_keyword(prev) || is_word_keyword(next) {
        return One;
    }
    Preserve
}

/// Lines (0-based) that must not be touched at all (continuation lines of
/// a multi-line token: whitespace there is content).
struct LineFacts {
    frozen: HashSet<u32>,
    no_respace: HashSet<u32>,
}

fn line_facts(model: &SemanticModel) -> LineFacts {
    let li = model.line_index();
    let tokens = model.tokens();
    let spans = model.spans();
    let mut frozen = HashSet::new();
    let mut no_respace = HashSet::new();
    let mut depth = 0i32;
    for (i, t) in tokens.iter().enumerate() {
        let s = spans[i];
        let a = li.position(s.start).line;
        let b = li.position(s.end).line;
        if b > a {
            // The token's own lines, except the one it ends on (a token
            // that ends at column 0 ended with its newline, so that line
            // is outside it).
            let end_col = li.position(s.end).character;
            for l in a..b {
                frozen.insert(l);
            }
            if end_col > 0 {
                frozen.insert(b);
            }
            let last = if end_col > 0 { b } else { b - 1 };
            for l in a..=last {
                no_respace.insert(l);
            }
        }
        // Raw Rust (a passthrough payload is the token right before its
        // zero-width `PassthroughStart` marker) and backtick shell strings
        // are opaque.
        let passthrough_payload = matches!(tokens.get(i + 1), Some(Token::PassthroughStart));
        if passthrough_payload || matches!(t, Token::Rust | Token::BacktickString(_)) {
            no_respace.insert(a);
        }
        if is_open(t) {
            if depth == 0 {
                // Record where an outermost bracket opens; if it closes
                // on another line, every line in between is off-limits.
                if let Some(close) = super::matching_close(tokens, i) {
                    let cl = li.position(spans[close].start).line;
                    for l in a..=cl {
                        if cl > a {
                            no_respace.insert(l);
                        }
                    }
                }
            }
            depth += 1;
        } else if is_close(t) {
            depth -= 1;
        }
    }
    LineFacts { frozen, no_respace }
}

/// Respaces one line from its tokens, or `None` if the line can't be
/// safely respaced (text between tokens that isn't whitespace).
fn respace_line(model: &SemanticModel, line: u32) -> Option<String> {
    let li = model.line_index();
    let text = li.line_text(line);
    let line_start = li.offset(crate::text::Pos { line, character: 0 })?;
    let line_end = line_start + text.len();
    let tokens = model.tokens();
    let spans = model.spans();

    // Lines inside a multi-line bracket are never respaced, so a line
    // being respaced always starts at bracket depth 0.
    let mut stack: Vec<&Token> = Vec::new();
    let mut on_line: Vec<usize> = Vec::new();
    let from = spans.partition_point(|s| s.start < line_start);
    for i in from..tokens.len() {
        let (t, s) = (&tokens[i], spans[i]);
        if s.start > line_end || (s.start == line_end && s.end == s.start) {
            break;
        }
        if super::is_marker(t) {
            continue;
        }
        if s.end > line_end {
            return None;
        }
        on_line.push(i);
    }
    if on_line.is_empty() {
        return None;
    }
    let first = spans[on_line[0]].start;
    let indent = &model.text()[line_start..first];
    if !indent.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }
    let full = model.text();
    // Angles written tight after a capitalized name (`List<Int>`,
    // `Dict<String, List<Int>>`) are type arguments, not comparisons.
    let mut generic = vec![false; on_line.len()];
    let mut depth = 0;
    for k in 0..on_line.len() {
        let i = on_line[k];
        match &tokens[i] {
            Token::LessThan if k > 0 && k + 1 < on_line.len() => {
                let p = on_line[k - 1];
                let n = on_line[k + 1];
                let tight = spans[p].end == spans[i].start && spans[i].end == spans[n].start;
                if tight
                    && matches!(&tokens[p], Token::Ident(name) if name.starts_with(|c: char| c.is_ascii_uppercase()))
                {
                    generic[k] = true;
                    depth += 1;
                }
            }
            Token::GreaterThan if depth > 0 => {
                generic[k] = true;
                depth -= 1;
            }
            _ => {}
        }
    }
    let mut out = String::from(indent);
    for (k, &i) in on_line.iter().enumerate() {
        if k > 0 {
            let p = on_line[k - 1];
            let between = &full[spans[p].end..spans[i].start];
            if !between.chars().all(|c| c == ' ' || c == '\t') {
                return None;
            }
            let pp = (k >= 2).then(|| &tokens[on_line[k - 2]]);
            match gap(
                pp,
                &tokens[p],
                &tokens[i],
                stack.last().copied(),
                (generic[k - 1], generic[k]),
            ) {
                Gap::Zero => {}
                Gap::One => out.push(' '),
                Gap::Preserve => out.push_str(between),
            }
            if is_open(&tokens[p]) {
                stack.push(&tokens[p]);
            }
            if is_close(&tokens[i]) {
                stack.pop();
            }
        }
        out.push_str(&full[spans[i].start..spans[i].end]);
    }
    let last_end = spans[*on_line.last().unwrap()].end;
    let rest = &full[last_end..line_end];
    let rest_trim = rest.trim();
    if !rest_trim.is_empty() {
        if !rest_trim.starts_with('#') {
            return None;
        }
        out.push_str("  ");
        out.push_str(rest_trim);
    }
    Some(out)
}

/// The formatted line for each original line (`None` = delete the line).
fn plan(model: &SemanticModel, respace: &dyn Fn(u32) -> bool) -> Vec<Option<String>> {
    let li = model.line_index();
    let facts = line_facts(model);
    let n = li.line_count() as u32;
    // A final empty "line" after a trailing newline isn't a real line.
    let real_lines = if model.text().ends_with('\n') {
        n - 1
    } else {
        n
    };
    let mut out: Vec<Option<String>> = Vec::with_capacity(real_lines as usize);
    let mut blank_run = 0;
    let mut seen_content = false;
    for line in 0..real_lines {
        let raw = li.line_text(line);
        if facts.frozen.contains(&line) {
            out.push(Some(raw.to_string()));
            blank_run = 0;
            seen_content = true;
            continue;
        }
        if raw.trim().is_empty() {
            blank_run += 1;
            if !seen_content || blank_run > 2 {
                out.push(None);
            } else {
                out.push(Some(String::new()));
            }
            continue;
        }
        blank_run = 0;
        seen_content = true;
        let formatted = if !facts.no_respace.contains(&line) && respace(line) {
            respace_line(model, line)
        } else {
            None
        };
        out.push(Some(
            formatted.unwrap_or_else(|| raw.trim_end().to_string()),
        ));
    }
    // Trailing blank lines before EOF.
    for i in (0..out.len()).rev() {
        match &out[i] {
            None => continue,
            Some(l) if l.is_empty() && !facts.frozen.contains(&(i as u32)) => out[i] = None,
            _ => break,
        }
    }
    out
}

fn e_is_line(e: Option<&TextEdit>, line: u32) -> bool {
    e.is_some_and(|e| e.range.start.line == line && e.range.end.line == line)
}

fn render(plan: &[Option<String>], eol: &str) -> String {
    let mut s = String::new();
    for l in plan.iter().flatten() {
        s.push_str(l);
        s.push_str(eol);
    }
    s
}

fn lex(text: &str) -> Vec<Token> {
    let mut toks = Lexer::new(text).tokenize();
    // Layout markers at EOF depend only on trailing blank lines.
    while matches!(
        toks.last(),
        Some(Token::Eof | Token::Newline | Token::Dedent)
    ) {
        toks.pop();
    }
    toks
}

/// The whole formatted document text (for tests and full formatting).
pub fn format_text(model: &SemanticModel) -> String {
    let text = model.text();
    if text.trim().is_empty() {
        return String::new();
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let original = lex(text);
    let all = plan(model, &|_| true);
    let candidate = render(&all, eol);
    if lex(&candidate) == original {
        return candidate;
    }
    // Some line's respacing changed its tokens: find the safe lines by
    // bisection (each probe re-lexes the document, so probing lines one
    // at a time would be quadratic).
    let base = plan(model, &|_| false);
    let changed: Vec<u32> = (0..all.len() as u32)
        .filter(|&l| all[l as usize] != base[l as usize])
        .collect();
    let mut good: HashSet<u32> = HashSet::new();
    bisect(&changed, &base, &all, eol, &original, &mut good);
    render(&plan(model, &|l| good.contains(&l)), eol)
}

fn bisect(
    lines: &[u32],
    base: &[Option<String>],
    all: &[Option<String>],
    eol: &str,
    original: &[Token],
    good: &mut HashSet<u32>,
) {
    if lines.is_empty() {
        return;
    }
    let mut probe = base.to_vec();
    for &l in good.iter().chain(lines) {
        probe[l as usize] = all[l as usize].clone();
    }
    if lex(&render(&probe, eol)) == original {
        good.extend(lines);
    } else if lines.len() > 1 {
        let (a, b) = lines.split_at(lines.len() / 2);
        bisect(a, base, all, eol, original, good);
        bisect(b, base, all, eol, original, good);
    }
}

/// Minimal per-line edits turning the document into its formatted form,
/// optionally restricted to lines `lines.0..=lines.1`.
pub fn format_edits(model: &SemanticModel, lines: Option<(u32, u32)>) -> Vec<TextEdit> {
    let text = model.text();
    if text.trim().is_empty() {
        return Vec::new();
    }
    let formatted = format_text(model);
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    // Re-derive the per-line plan that produced `formatted`, by pairing
    // original lines with formatted ones: `plan` never reorders lines, it
    // only rewrites or deletes them, so a greedy alignment recovers it.
    let li = model.line_index();
    let real_lines = if text.ends_with('\n') {
        li.line_count() - 1
    } else {
        li.line_count()
    };
    let new_lines: Vec<&str> = formatted
        .strip_suffix(eol)
        .unwrap_or(&formatted)
        .split(eol)
        .collect();
    let plan_all = plan(model, &|_| true);
    // Which formatted plan was used: recompute the same way format_text
    // did by checking which lines survived (deletions are identical in
    // every plan, only rewrites differ).
    let mut edits = Vec::new();
    let mut k = 0usize;
    for line in 0..real_lines as u32 {
        let in_range = lines.is_none_or(|(a, b)| a <= line && line <= b);
        let raw = li.line_text(line);
        let deleted = plan_all[line as usize].is_none();
        if deleted {
            if in_range {
                edits.push(TextEdit {
                    range: Range {
                        start: Position { line, character: 0 },
                        end: Position {
                            line: line + 1,
                            character: 0,
                        },
                    },
                    new_text: String::new(),
                });
            }
            continue;
        }
        let new = new_lines.get(k).copied().unwrap_or(raw);
        k += 1;
        if new != raw && in_range {
            edits.push(TextEdit {
                range: Range {
                    start: Position { line, character: 0 },
                    end: Position {
                        line,
                        character: raw.encode_utf16().count() as u32,
                    },
                },
                new_text: new.to_string(),
            });
        }
    }
    // Missing final newline: append it to the last line's own edit if it
    // has one (edits must not overlap), else insert it.
    let wants_eol = !text.ends_with('\n') && lines.is_none_or(|(_, b)| b + 1 >= real_lines as u32);
    let last = real_lines as u32 - 1;
    if wants_eol && plan_all[last as usize].is_some() {
        if e_is_line(edits.last(), last) {
            edits.last_mut().unwrap().new_text.push_str(eol);
        } else {
            let col = li.line_text(last).encode_utf16().count() as u32;
            let at = Position {
                line: last,
                character: col,
            };
            edits.push(TextEdit {
                range: Range { start: at, end: at },
                new_text: eol.to_string(),
            });
        }
    }
    edits
}
