//! Regression tests for two lexer bugs where source text vanished without
//! a trace: code on the same line as `rust:`, and a bare `!`.

use craw::lexer::{Lexer, Token};
use craw::semantic::SemanticModel;

fn lex(src: &str) -> Vec<(Token, String)> {
    let (tokens, spans) = Lexer::new(src).tokenize_spanned();
    let mut last = 0;
    tokens
        .into_iter()
        .zip(spans)
        .map(|(t, s)| {
            assert!(s.start >= last, "span {s:?} out of order in {src:?}");
            last = s.start;
            (t, src[s.start..s.end].to_string())
        })
        .collect()
}

fn rust_payloads(src: &str) -> Vec<String> {
    lex(src)
        .into_iter()
        .filter_map(|(t, _)| match t {
            Token::String(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn transpile(src: &str) -> Result<String, Vec<String>> {
    let model = SemanticModel::build(src);
    if !model.syntax_errors().is_empty() {
        return Err(model
            .syntax_errors()
            .iter()
            .map(|e| e.message.clone())
            .collect());
    }
    Ok(craw::transpiler::transpile(model.ast()))
}

#[test]
fn code_on_the_rust_line_is_kept() {
    let src = "rust: panic!(\"boom\");\nx = 1\n";
    assert_eq!(rust_payloads(src), vec!["panic!(\"boom\");"]);
    let (t, text) = lex(src)
        .into_iter()
        .find(|(t, _)| matches!(t, Token::String(_)))
        .unwrap();
    assert!(matches!(t, Token::String(_)));
    assert_eq!(text, "panic!(\"boom\");\n", "span covers the code itself");
    assert!(transpile(src).unwrap().contains("panic!(\"boom\");"));
}

#[test]
fn same_line_code_comes_before_the_indented_block() {
    let src = "rust: let a = 1;\n    let b = a + 1;\nx = 1\n";
    assert_eq!(rust_payloads(src), vec!["let a = 1;\nlet b = a + 1;"]);
    let out = transpile(src).unwrap();
    assert!(out.find("let a = 1;").unwrap() < out.find("let b = a + 1;").unwrap());
}

#[test]
fn a_craw_comment_after_rust_colon_is_still_a_comment() {
    let src = "rust:  # native helpers\n    fn helper() {}\n";
    assert_eq!(rust_payloads(src), vec!["fn helper() {}"]);
}

#[test]
fn rust_attributes_after_rust_colon_are_code() {
    let src = "rust: #[inline] fn one() -> i64 { 1 }\n";
    assert_eq!(rust_payloads(src), vec!["#[inline] fn one() -> i64 { 1 }"]);
}

#[test]
fn inline_rust_in_an_indented_block_keeps_the_block_structure() {
    // The exact shape from samples/showcase.craw's `assert_safe` macro.
    let src = "def check(cond):\n    if not cond:\n        print(\"bad\")\n        rust: panic!(\"failed\");\n    return cond\n";
    let out = transpile(src).unwrap();
    assert!(out.contains("panic!(\"failed\");"));
    let model = SemanticModel::build(src);
    assert!(model.syntax_errors().is_empty());
    assert!(model.symbol_named("check").is_some());
}

#[test]
fn plain_rust_blocks_are_unchanged() {
    let src = "rust:\n    fn a() {}\n    fn b() {}\nx = 1\n";
    assert_eq!(rust_payloads(src), vec!["fn a() {}\nfn b() {}"]);
    let kinds: Vec<_> = lex(src).into_iter().map(|(t, _)| t).take(5).collect();
    assert_eq!(
        kinds,
        vec![
            Token::Rust,
            Token::Colon,
            Token::Newline,
            Token::Indent,
            Token::String("fn a() {}\nfn b() {}".into())
        ]
    );
}

#[test]
fn a_bare_bang_is_a_syntax_error_with_a_hint() {
    let toks = lex("x = !y\n");
    assert!(toks.contains(&(Token::Operator("!".into()), "!".into())));
    let errs = transpile("x = !y\n").unwrap_err();
    assert!(errs[0].contains("Craw negates with `not`"), "{errs:?}");
    assert!(transpile("x = a.!\n").is_err());
}

#[test]
fn bang_equals_forms_still_work() {
    assert!(lex("a != b\n").iter().any(|(t, _)| *t == Token::NotEqual));
    assert!(
        lex("a .!= b\n")
            .iter()
            .any(|(t, _)| *t == Token::DotNotEqual)
    );
    assert!(transpile("ok = 1 != 2\nall = [1] .!= [2]\n").is_ok());
}

#[test]
fn the_lsp_reports_a_bare_bang_at_its_position() {
    let mut a = craw::lsp_analyzer::LspAnalyzer::new();
    let diags = a.update_document("file:///b.craw", "x = 1\ny = !x\n");
    let d = diags
        .iter()
        .find(|d| d.message.contains("`!`"))
        .expect("a diagnostic for `!`");
    assert_eq!((d.range.start.line, d.range.start.character), (1, 4));
}
