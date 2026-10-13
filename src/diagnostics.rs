//! Human-readable syntax diagnostics with exact byte ranges.
//!
//! Turns chumsky's `Rich<Token>` parse errors (token-index spans, a raw
//! `Debug`-formatted set of alternatives) into messages and byte spans a
//! person or an LSP client can use directly.

use crate::lexer::Token;
use crate::text::{LineIndex, Span};
use chumsky::Parser;
use chumsky::error::{Rich, RichPattern};
use std::collections::HashSet;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};

/// A human-readable name for a single token, used both to describe what was
/// found and, via [`RichPattern`], what was expected.
pub fn token_label(t: &Token) -> String {
    match t {
        Token::Ident(name) => format!("identifier `{name}`"),
        Token::Number(n) => format!("number `{n}`"),
        Token::Float(n) => format!("number `{n}`"),
        Token::String(s) => format!("string {s:?}"),
        Token::FStringRaw(s) => format!("f-string \"{s}\""),
        Token::BacktickString(s) => format!("`` `{s}` ``"),
        Token::Operator(op) => format!("`{op}`"),
        Token::Assign => "`=`".to_string(),
        Token::Plus => "`+`".to_string(),
        Token::Minus => "`-`".to_string(),
        Token::Star => "`*`".to_string(),
        Token::Slash => "`/`".to_string(),
        Token::Percent => "`%`".to_string(),
        Token::Equal => "`==`".to_string(),
        Token::NotEqual => "`!=`".to_string(),
        Token::LessEqual => "`<=`".to_string(),
        Token::GreaterEqual => "`>=`".to_string(),
        Token::LessThan => "`<`".to_string(),
        Token::GreaterThan => "`>`".to_string(),
        Token::Def => "`def`".to_string(),
        Token::Return => "`return`".to_string(),
        Token::If => "`if`".to_string(),
        Token::Else => "`else`".to_string(),
        Token::Indent => "indented block".to_string(),
        Token::Dedent => "end of indented block".to_string(),
        Token::Newline => "end of line".to_string(),
        Token::Colon => "`:`".to_string(),
        Token::Comma => "`,`".to_string(),
        Token::LParen => "`(`".to_string(),
        Token::RParen => "`)`".to_string(),
        Token::Eof => "end of file".to_string(),
        Token::Data => "`data`".to_string(),
        Token::Class => "`class`".to_string(),
        Token::Extends => "`extends`".to_string(),
        Token::With => "`with`".to_string(),
        Token::Arrow => "`=>`".to_string(),
        Token::ThinArrow => "`->`".to_string(),
        Token::PipeForward => "`|>`".to_string(),
        Token::PipeBackward => "`<|`".to_string(),
        Token::DotDot => "`..`".to_string(),
        Token::ComposeForward => "`..>`".to_string(),
        Token::ComposeBackward => "`<..`".to_string(),
        Token::Match => "`match`".to_string(),
        Token::Case => "`case`".to_string(),
        Token::In => "`in`".to_string(),
        Token::For => "`for`".to_string(),
        Token::NoneCoalesce => "`??`".to_string(),
        Token::Partial => "`$`".to_string(),
        Token::Pipe => "`|`".to_string(),
        Token::Dot => "`.`".to_string(),
        Token::DotPlus => "`.+`".to_string(),
        Token::DotMinus => "`.-`".to_string(),
        Token::DotStar => "`.*`".to_string(),
        Token::DotSlash => "`./`".to_string(),
        Token::DotPercent => "`.%`".to_string(),
        Token::DotEqual => "`.==`".to_string(),
        Token::DotNotEqual => "`.!=`".to_string(),
        Token::DotLess => "`.<`".to_string(),
        Token::DotLessEqual => "`.<=`".to_string(),
        Token::DotGreater => "`.>`".to_string(),
        Token::DotGreaterEqual => "`.>=`".to_string(),
        Token::Question => "`?`".to_string(),
        Token::LBracket => "`[`".to_string(),
        Token::RBracket => "`]`".to_string(),
        Token::LBrace => "`{`".to_string(),
        Token::RBrace => "`}`".to_string(),
        Token::PipeForwardStar => "`|*>`".to_string(),
        Token::PipeForwardDoubleStar => "`|**>`".to_string(),
        Token::PipeForwardNone => "`|?>`".to_string(),
        Token::PipeForwardNoneStar => "`|?*>`".to_string(),
        Token::PipeForwardNoneDoubleStar => "`|?**>`".to_string(),
        Token::PipeBackwardStar => "`<*|`".to_string(),
        Token::PipeBackwardDoubleStar => "`<**|`".to_string(),
        Token::PipeBackwardNone => "`<?|`".to_string(),
        Token::PipeBackwardNoneStar => "`<*?|`".to_string(),
        Token::PipeBackwardNoneDoubleStar => "`<**?|`".to_string(),
        Token::ComposeStar => "`..*>`".to_string(),
        Token::ComposeDoubleStar => "`..**>`".to_string(),
        Token::ComposeNone => "`..?>`".to_string(),
        Token::Copyclosure => "`copyclosure`".to_string(),
        Token::Addpattern => "`addpattern`".to_string(),
        Token::Where => "`where`".to_string(),
        Token::Global => "`global`".to_string(),
        Token::Nonlocal => "`nonlocal`".to_string(),
        Token::Semicolon => "`;`".to_string(),
        Token::Lambda => "`lambda`".to_string(),
        Token::OperatorKeyword => "`operator`".to_string(),
        Token::Struct => "`struct`".to_string(),
        Token::Trait => "`trait`".to_string(),
        Token::Impl => "`impl`".to_string(),
        Token::From => "`from`".to_string(),
        Token::Import => "`import`".to_string(),
        Token::Yield => "`yield`".to_string(),
        Token::While => "`while`".to_string(),
        Token::Break => "`break`".to_string(),
        Token::Continue => "`continue`".to_string(),
        Token::Then => "`then`".to_string(),
        Token::At => "`@`".to_string(),
        Token::PassthroughStart => "start of embedded Rust".to_string(),
        Token::Not => "`not`".to_string(),
        Token::And => "`and`".to_string(),
        Token::Or => "`or`".to_string(),
        Token::Is => "`is`".to_string(),
        Token::PlusAssign => "`+=`".to_string(),
        Token::MinusAssign => "`-=`".to_string(),
        Token::StarAssign => "`*=`".to_string(),
        Token::SlashAssign => "`/=`".to_string(),
        Token::Power => "`**`".to_string(),
        Token::PowerAssign => "`**=`".to_string(),
        Token::Template => "`template`".to_string(),
        Token::Select => "`select`".to_string(),
        Token::OrderBy => "`orderby`".to_string(),
        Token::Ascending => "`ascending`".to_string(),
        Token::Descending => "`descending`".to_string(),
        Token::Tilde => "`~`".to_string(),
        Token::Macro => "`macro`".to_string(),
        Token::To => "`to`".to_string(),
        Token::Until => "`until`".to_string(),
        Token::Gen => "`gen`".to_string(),
        Token::Use => "`use`".to_string(),
        Token::As => "`as`".to_string(),
        Token::Rust => "`rust`".to_string(),
    }
}

/// Whether `t` can begin an expression. Used to collapse a long list of
/// alternatives (typical when a parse error occurs where any expression
/// would have been valid) down to a single readable hint.
fn is_expr_starter(t: &Token) -> bool {
    matches!(
        t,
        Token::Ident(_) | Token::Number(_) | Token::String(_) | Token::LParen | Token::LBracket
    )
}

fn rich_pattern_label(pat: &RichPattern<'_, Token>) -> (String, bool) {
    match pat {
        RichPattern::Token(t) => {
            let tok: &Token = t;
            (token_label(tok), is_expr_starter(tok))
        }
        RichPattern::Label(s) => (s.to_string(), false),
        RichPattern::Identifier(s) => (format!("`{s}`"), false),
        RichPattern::Any => ("any token".to_string(), false),
        RichPattern::SomethingElse => ("something else".to_string(), false),
        RichPattern::EndOfInput => ("end of file".to_string(), false),
        _ => ("something else".to_string(), false),
    }
}

fn format_alternatives(labels: &[String]) -> String {
    if labels.is_empty() {
        return "expected something else".to_string();
    }
    let take = &labels[..labels.len().min(3)];
    match take {
        [a] => format!("expected {a}"),
        [a, b] => format!("expected {a} or {b}"),
        [a, b, c, ..] => format!("expected {a}, {b} or {c}"),
        [] => unreachable!(),
    }
}

/// A single syntax error: an exact byte range plus a ready-to-display
/// message (no `found`/`expected` debug dumps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}

/// Renders one parser error. `spans` is the per-token byte-span table from
/// [`crate::lexer::Lexer::tokenize_spanned`]; chumsky's own span on `err` is
/// a *token index* range (our parser runs over `&[Token]`), so it is looked
/// up in `spans` to get a real byte range. An index past the end of `spans`
/// (an error at EOF) maps to a zero-width span at `text_len`.
pub fn render_parse_error(err: &Rich<'_, Token>, spans: &[Span], text_len: usize) -> SyntaxError {
    let idx = err.span().start;
    let span = spans
        .get(idx)
        .copied()
        .unwrap_or(Span::new(text_len, text_len));

    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    let mut has_expr_hint = false;
    for pat in err.expected() {
        let (label, is_expr) = rich_pattern_label(pat);
        has_expr_hint |= is_expr;
        if seen.insert(label.clone()) {
            labels.push(label);
        }
    }

    let message_body = if labels.len() > 3 && has_expr_hint {
        "expected expression".to_string()
    } else {
        format_alternatives(&labels)
    };

    let found = match err.found() {
        // A Rust/C habit: point at the Craw spelling.
        Some(Token::Operator(op)) if op == "!" => {
            ", found `!` (Craw negates with `not`)".to_string()
        }
        Some(t) => format!(", found {}", token_label(t)),
        None => ", found end of file".to_string(),
    };

    SyntaxError {
        span,
        message: format!("{message_body}{found}"),
    }
}

/// Lexes and parses `text`, returning one [`Diagnostic`] per syntax error
/// with an exact range, a readable message, `source: "craw"`, and
/// `code: "E0001"`. Empty when `text` parses cleanly.
pub fn syntax_diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut lexer = crate::lexer::Lexer::new(text);
    let (tokens, spans) = lexer.tokenize_spanned();
    let (_, errors) = crate::parser::parser().parse(&tokens).into_output_errors();
    if errors.is_empty() {
        return Vec::new();
    }
    let line_index = LineIndex::new(text);
    errors
        .iter()
        .map(|err| {
            let syntax_error = render_parse_error(err, &spans, text.len());
            Diagnostic {
                range: line_index.range(syntax_error.span),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("E0001".to_string())),
                source: Some("craw".to_string()),
                message: syntax_error.message,
                ..Default::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_rhs_points_at_newline_token() {
        let src = "def f():\n    x = \n";
        let d = syntax_diagnostics(src);
        assert_eq!(d.len(), 1);
        let d0 = &d[0];
        assert_eq!((d0.range.start.line, d0.range.start.character), (1, 8));
        assert!(
            d0.message.starts_with("expected expression"),
            "{}",
            d0.message
        );
        assert_eq!(d0.code, Some(NumberOrString::String("E0001".to_string())));
        assert_eq!(d0.source.as_deref(), Some("craw"));
    }

    #[test]
    fn unexpected_token_names_the_token() {
        let d = syntax_diagnostics("x = 1 +* 2\n");
        assert!(d[0].message.contains("found `*`"), "{}", d[0].message);
    }

    #[test]
    fn error_inside_brackets_still_gets_right_line() {
        let d = syntax_diagnostics("a = [1,\n     2,\n     ,]\n");
        assert_eq!(d[0].range.start.line, 2);
    }

    #[test]
    fn token_labels_are_readable() {
        assert_eq!(token_label(&Token::Newline), "end of line");
        assert_eq!(token_label(&Token::Indent), "indented block");
        assert_eq!(token_label(&Token::Ident("foo".into())), "identifier `foo`");
        assert_eq!(token_label(&Token::PipeForward), "`|>`");
    }
}
