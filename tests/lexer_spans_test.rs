use craw::lexer::{Lexer, Token};
use craw::text::Span;

fn spanned(src: &str) -> Vec<(Token, Span)> {
    let mut lx = Lexer::new(src);
    let (toks, spans) = lx.tokenize_spanned();
    assert_eq!(toks.len(), spans.len(), "one span per token");
    toks.into_iter().zip(spans).collect()
}

#[test]
fn simple_assignment_spans() {
    let t = spanned("x = 12\n");
    assert_eq!(t[0], (Token::Ident("x".into()), Span::new(0, 1)));
    assert_eq!(t[1], (Token::Assign, Span::new(2, 3)));
    assert_eq!(t[2], (Token::Number(12), Span::new(4, 6)));
}

#[test]
fn identifiers_round_trip_through_spans() {
    // For every Ident token, the source slice at its span must equal the name.
    for path in [
        "samples/showcase.craw",
        "samples/master_showcase.craw",
        "samples/macro_blocks.craw",
        "tests/feature_test.craw",
    ] {
        let src = std::fs::read_to_string(path).unwrap();
        for (tok, sp) in spanned(&src) {
            match &tok {
                Token::Ident(name) => {
                    assert_eq!(&src[sp.start..sp.end], name, "{path} @ {sp:?}")
                }
                Token::Number(_) | Token::Float(_) => {
                    assert!(!src[sp.start..sp.end].trim().is_empty())
                }
                _ => {}
            }
            assert!(
                sp.start <= sp.end && sp.end <= src.len(),
                "{path} {tok:?} {sp:?}"
            );
        }
    }
}

#[test]
fn spans_are_monotonic() {
    let src = std::fs::read_to_string("samples/master_showcase.craw").unwrap();
    let mut last = 0;
    for (_, sp) in spanned(&src) {
        assert!(sp.start >= last, "span {sp:?} regressed before {last}");
        last = sp.start;
    }
}

#[test]
fn tokenize_is_unchanged() {
    let src = "def f(a):\n    return a + 1\n";
    let plain = Lexer::new(src).tokenize();
    let (with_spans, _) = Lexer::new(src).tokenize_spanned();
    assert_eq!(plain, with_spans);
}

#[test]
fn multibyte_operator_spans() {
    let t = spanned("a ≈ b\n");
    // '≈' occupies bytes 2..5
    assert_eq!(t[1].1, Span::new(2, 5));
    assert_eq!(t[2].1, Span::new(6, 7));
}
