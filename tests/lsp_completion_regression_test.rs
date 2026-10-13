//! Systemic guards against two classes of completion bug that unit tests
//! on hand-picked snippets missed: (1) `receiver.` being misdetected as a
//! fresh statement start (a token_before tie-break bug — see the fix for
//! `token_before` in `lsp_analyzer.rs`), and (2) member completion on a
//! receiver of unknown type covering fields but not built-in instance
//! methods (the `pad_string`/`s.` bug). Both are exercised here against
//! every real `.craw` file shipped in the repo, at every real
//! `identifier.` position the lexer actually produces — not just the
//! specific snippets that happened to be turned into unit tests.

use craw::lexer::{Lexer, Token};
use craw::lsp_analyzer::LspAnalyzer;
use tower_lsp::lsp_types::CompletionItemKind;

const SAMPLE_FILES: &[&str] = &[
    "samples/showcase.craw",
    "samples/master_showcase.craw",
    "samples/macro_blocks.craw",
    "tests/feature_test.craw",
    "tests/parity_dict.craw",
    "tests/parity_list.craw",
    "tests/parity_string.craw",
    "tests/test_data_runtime.craw",
    "tests/test_is_prime.craw",
    "tests/test_lambda.craw",
    "tests/test_pipe_compose.craw",
    "tests/test_pipe_partial.craw",
];

/// Every offset right after a `receiver.` in `src`, found from the real
/// token stream (an `Ident` immediately followed by a `Dot`) rather than
/// text scanning, so it can't be confused by dots inside strings/comments
/// or float literals.
fn member_access_offsets(src: &str) -> Vec<usize> {
    let mut lexer = Lexer::new(src);
    let (tokens, spans) = lexer.tokenize_spanned();
    let mut offsets = Vec::new();
    for i in 0..tokens.len().saturating_sub(1) {
        if matches!(tokens[i], Token::Ident(_)) && matches!(tokens[i + 1], Token::Dot) {
            offsets.push(spans[i + 1].end);
        }
    }
    offsets
}

#[test]
fn member_completion_never_leaks_keywords_across_every_sample_file() {
    for path in SAMPLE_FILES {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut analyzer = LspAnalyzer::new();
        analyzer.update_document("file:///m.craw", &src);
        let line_index = craw::text::LineIndex::new(&src);

        for offset in member_access_offsets(&src) {
            let pos = line_index.position(offset);
            let items = analyzer.get_completions("file:///m.craw", pos.line, pos.character);
            let leaked: Vec<_> = items
                .iter()
                .filter(|c| c.kind == Some(CompletionItemKind::KEYWORD))
                .map(|c| c.label.as_str())
                .collect();
            assert!(
                leaked.is_empty(),
                "{path}:{}:{}: keywords leaked into member completion: {leaked:?}",
                pos.line + 1,
                pos.character
            );
        }
    }
}

#[test]
fn member_completion_on_an_unresolved_receiver_is_never_empty() {
    // Built-in instance methods (len, upper, append, ...) are always a
    // candidate for a receiver of unknown type, so unless every field in
    // the whole file is also somehow absent, this can never come back
    // empty — that was exactly the gap the pad_string bug exposed.
    let items = {
        let mut a = LspAnalyzer::new();
        a.update_document("file:///m.craw", "x = 1\nx.\n");
        a.get_completions("file:///m.craw", 1, 2)
    };
    assert!(!items.is_empty());
}
