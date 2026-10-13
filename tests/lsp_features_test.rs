use craw::lsp_analyzer::LspAnalyzer;
use tower_lsp::lsp_types::{CompletionItemKind, InsertTextFormat};

/// Splits `src` at the first `$0`, returning `(text without the marker,
/// line, UTF-16 column)` — the position the marker stood at.
fn cursor(src: &str) -> (String, u32, u32) {
    let idx = src
        .find("$0")
        .expect("fixture must contain a $0 cursor marker");
    let before = &src[..idx];
    let text = format!("{}{}", before, &src[idx + 2..]);
    let line = before.matches('\n').count() as u32;
    let col_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = before[col_start..].encode_utf16().count() as u32;
    (text, line, col)
}

fn complete(src: &str) -> Vec<tower_lsp::lsp_types::CompletionItem> {
    let (text, line, col) = cursor(src);
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", &text);
    a.get_completions("file:///m.craw", line, col)
}

#[test]
fn member_completion_on_a_known_constructor_lists_its_fields() {
    let items = complete("data P(x, y)\np = P(1, 2)\np.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["x", "y"]);
}

#[test]
fn member_completion_on_an_unknown_receiver_lists_every_field() {
    let items = complete("data P(x)\ndata Q(z)\nw.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"x") && labels.contains(&"z"));
    assert!(!labels.contains(&"def"));
}

/// The exact bug reported against `pad_string`'s `pad.` (a plain string
/// value with no known constructor): fields-only was correct in kind but
/// incomplete in content. A receiver of unknown type must also offer
/// built-in instance methods (string/list/dict/set/...), since those are
/// exactly as "unknown but plausible" as every known field is.
#[test]
fn member_completion_on_a_truly_unresolved_receiver_also_lists_builtin_methods() {
    // `s` is a bare parameter with no assignment anywhere to infer a type
    // from — genuinely unknown, unlike the narrowed cases below.
    let items = complete("def f(s):\n    s.$0\n");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    for method in ["len", "upper", "append", "get", "add"] {
        assert!(labels.contains(&method), "missing builtin method {method}");
    }
    assert!(!labels.contains(&"def"), "keywords must not leak in");
}

#[test]
fn member_completion_builtin_methods_have_detail_and_method_kind() {
    let items = complete("def f(s):\n    s.$0\n");
    let upper = items.iter().find(|c| c.label == "upper").unwrap();
    assert_eq!(upper.kind, Some(CompletionItemKind::METHOD));
    assert_eq!(upper.detail.as_deref(), Some("s.upper()"));
}

/// The exact bug reported against `pad_string`: assigning `pad` a string
/// (even through a ternary of two string-producing branches) must
/// resolve to *only* String's own methods, not the shotgun union that a
/// genuinely unknown receiver gets — the whole point of "real type
/// inference like rust-analyzer" is that a known type narrows the list.
#[test]
fn member_completion_narrows_to_string_methods_for_an_inferred_string() {
    let items = complete("pad = if 1 > 0 then \" \" * 3 else \"\"\npad.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "len",
            "lower",
            "upper",
            "strip",
            "split",
            "join",
            "replace",
            "startswith",
            "endswith"
        ]
    );
    assert!(
        !labels.contains(&"append"),
        "list methods must not leak in for a known string"
    );
}

#[test]
fn member_completion_narrows_for_a_plain_string_literal_assignment() {
    let items = complete("s = \"hello\"\ns.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper"));
    assert!(!labels.contains(&"append") && !labels.contains(&"get"));
}

#[test]
fn member_completion_narrows_for_a_list_literal_assignment() {
    let items = complete("xs = [1, 2, 3]\nxs.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"append") && labels.contains(&"pop"));
    assert!(!labels.contains(&"upper") && !labels.contains(&"get"));
}

#[test]
fn member_completion_narrows_through_a_simple_alias_chain() {
    let items = complete("a = \"x\"\nb = a\nb.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper"));
    assert!(!labels.contains(&"append"));
}

#[test]
fn member_completion_narrows_via_a_known_builtin_return_type() {
    let items = complete("s = str(42)\ns.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper"));
    assert!(!labels.contains(&"append"));
}

#[test]
fn member_completion_narrows_via_an_annotated_return_type() {
    let items = complete("def make() -> String = \"x\"\nv = make()\nv.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper"));
    assert!(!labels.contains(&"append"));
}

/// The other half of "real type inference": when a name has no
/// inferable structural shape (a bare function parameter, the exact
/// case that blocked `pad_string`'s `length`), an explicit type
/// annotation is the primary signal rust-analyzer-style tooling relies
/// on, and it must be honored the same way literal inference is.
#[test]
fn member_completion_narrows_for_an_annotated_parameter() {
    let items = complete("def f(s: String):\n    s.$0\n");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper"));
    assert!(!labels.contains(&"append"));
}

#[test]
fn member_completion_narrows_for_an_annotated_assignment() {
    let items = complete("x: String = get_it()\nx.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper"));
    assert!(!labels.contains(&"append"));
}

#[test]
fn member_completion_narrows_for_an_annotated_custom_type_parameter() {
    // `f` itself is a legitimate UFCS candidate here (`p.f()` really is
    // callable, since `f` takes exactly one parameter) so it's expected
    // alongside P's fields — the assertion is that only P's own fields
    // and UFCS candidates appear, not every builtin method too.
    let items = complete("data P(x, y)\ndef f(p: P):\n    p.$0\n");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"x") && labels.contains(&"y"));
    assert!(!labels.contains(&"upper") && !labels.contains(&"append"));
}

/// The full motivating scenario, once annotated: `length: Int` plus
/// `len(s)`'s known Int return type let `diff` infer as Int (via `-`),
/// which lets `" " * diff` infer as String (repetition), which lets the
/// ternary and finally `pad` infer as String. Every hop matters; this
/// is the end-to-end proof the chain actually works, not just each
/// isolated rule.
#[test]
fn member_completion_narrows_the_full_pad_string_chain_once_annotated() {
    let items = complete(concat!(
        "def pad_string(s: String, length: Int):\n",
        "    diff = length - len(s)\n",
        "    pad = if diff > 0 then \" \" * diff else \"\"\n",
        "    pad.$0\n",
        "    return pad + s\n",
    ));
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "len",
            "lower",
            "upper",
            "strip",
            "split",
            "join",
            "replace",
            "startswith",
            "endswith"
        ]
    );
}

/// Without annotations, `length` and `s` are bare, unannotated
/// parameters with no assignment to infer from — genuinely unknowable,
/// the same limitation Pylance has for undeclared Python parameters.
/// This must still fall back honestly (every method), not guess wrong.
#[test]
fn member_completion_falls_back_honestly_for_the_pad_string_chain_when_unannotated() {
    let items = complete(concat!(
        "def pad_string(s, length):\n",
        "    diff = length - len(s)\n",
        "    pad = if diff > 0 then \" \" * diff else \"\"\n",
        "    pad.$0\n",
        "    return pad + s\n",
    ));
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper") && labels.contains(&"append"));
}

#[test]
fn member_completion_numeric_promotion_rules() {
    // Int - Int -> Int
    let items = complete("a = 5 - 2\na.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"abs") && !labels.contains(&"upper"));

    // Int / Int -> Float always (craw_div2 never returns Int)
    let items = complete("a = 5 / 2\na.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"Int") && labels.contains(&"abs"));

    // Int + Float -> Float
    let items = complete("a = 5 + 2.0\na.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"Int"));
}

#[test]
fn member_completion_string_plus_anything_is_always_string() {
    let items = complete("a = \"x: \" + 5\na.$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper") && !labels.contains(&"abs"));
}

#[test]
fn member_completion_does_not_infinite_loop_on_a_self_referential_binop() {
    // `a` is defined in terms of itself through a binary op (not a plain
    // alias) — this must terminate (bounded by the recursion depth
    // guard) and fall back to the general list, not hang or panic.
    let items = complete("a = a + b\na.$0");
    assert!(!items.is_empty());
}

#[test]
fn statement_start_offers_keywords_snippets_and_locals() {
    let items = complete("x = 1\n$0");
    assert!(
        items
            .iter()
            .any(|c| c.label == "def" && c.kind == Some(CompletionItemKind::KEYWORD))
    );
    assert!(items.iter().any(|c| c.label == "def"
        && c.kind == Some(CompletionItemKind::SNIPPET)
        && c.insert_text_format == Some(InsertTextFormat::SNIPPET)));
    assert!(items.iter().any(|c| c.label == "x"));
}

#[test]
fn expression_position_has_locals_and_builtins_but_no_keywords() {
    let items = complete("x = 1\ny = $0");
    assert!(items.iter().any(|c| c.label == "x"));
    let len_item = items.iter().find(|c| c.label == "len").unwrap();
    assert_eq!(len_item.detail.as_deref(), Some("len(xs)"));
    assert!(!items.iter().any(|c| c.label == "def"));
}

#[test]
fn locals_sort_before_builtins() {
    let items = complete("abs_val = 1\nz = a$0");
    let sort_text = |label: &str| {
        items
            .iter()
            .find(|c| c.label == label)
            .and_then(|c| c.sort_text.clone())
            .unwrap_or_default()
    };
    assert!(sort_text("abs_val") < sort_text("abs"));
}

#[test]
fn import_position_suggests_module_paths() {
    let items = complete("from $0");
    assert!(items.iter().any(|c| c.label == "std.math"));
}

/// The single most common real-world case: the user has typed at least
/// one character of what they're completing. Every other completion
/// test in this file places the cursor immediately after a trigger
/// with *nothing* typed yet, which never exercises this — and it's
/// exactly where context detection broke: a partial word being typed
/// is itself the last token before the cursor, and completion_context
/// used to read *that* token instead of what precedes it, always
/// landing on the Expression fallback regardless of the real context.
#[test]
fn statement_start_completions_survive_typing_a_partial_keyword() {
    let items = complete("x = 1\nd$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(
        items
            .iter()
            .any(|c| c.label == "def" && c.kind == Some(CompletionItemKind::KEYWORD))
    );
    assert!(
        items
            .iter()
            .any(|c| c.label == "def" && c.kind == Some(CompletionItemKind::SNIPPET))
    );
    assert!(labels.contains(&"x"), "locals must still be offered too");
}

#[test]
fn statement_start_completions_survive_typing_a_partial_keyword_inside_a_function() {
    let items = complete("def f():\n    m$0");
    assert!(items.iter().any(|c| c.label == "match"));
}

#[test]
fn member_completion_survives_typing_a_partial_member_name() {
    let items = complete("data P(xanadu, yellow)\np = P(1, 2)\np.x$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["xanadu", "yellow"]);
}

#[test]
fn member_completion_survives_typing_a_partial_builtin_method_name() {
    let items = complete("s = \"hi\"\ns.up$0");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"upper"));
    assert!(
        !labels.contains(&"append"),
        "must still narrow by inferred type"
    );
}

#[test]
fn no_completions_inside_inline_rust() {
    let items = complete("rust:\n    let x = $0\n");
    assert!(items.is_empty());
}

/// Regression test: right after `receiver.` with nothing else on the line
/// (a *real*, mid-file newline follows — not the synthetic one the lexer
/// appends at end-of-text, which `token_before` already excludes), the
/// line's own `Newline` token is zero-width at the exact same offset the
/// `Dot` token ends at. `token_before` must still resolve to `Dot`, not
/// that tied zero-width `Newline`, or completion silently falls back to
/// the generic statement-start list (keywords + every local + snippets)
/// instead of narrowing to members.
#[test]
fn member_completion_still_narrows_at_end_of_line_before_a_real_newline() {
    let items = complete("data P(x, y)\np = P(1, 2)\np.$0\nz = 1\n");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, vec!["x", "y"]);
}

#[test]
fn builtin_completions_defer_documentation_to_resolve() {
    let items = complete("y = $0");
    let len_item = items.iter().find(|c| c.label == "len").unwrap();
    assert!(
        len_item.documentation.is_none(),
        "docs should be lazy, not inlined"
    );
    assert!(
        len_item.data.is_some(),
        "resolve needs data to look the builtin back up"
    );

    let a = LspAnalyzer::new();
    let resolved = a.resolve_completion(len_item.clone());
    assert!(resolved.documentation.is_some());
}

#[test]
fn builtin_and_method_completions_trigger_parameter_hints_and_commit_on_paren() {
    let items = complete("y = $0");
    let len_item = items.iter().find(|c| c.label == "len").unwrap();
    assert_eq!(
        len_item.command.as_ref().map(|c| c.command.as_str()),
        Some("editor.action.triggerParameterHints")
    );
    assert_eq!(
        len_item.commit_characters,
        Some(vec!["(".to_string(), ".".to_string()])
    );
}

fn hover(src: &str) -> Option<tower_lsp::lsp_types::Hover> {
    let (text, line, col) = cursor(src);
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", &text);
    a.get_hover_info("file:///m.craw", line, col)
}

fn hover_markdown(src: &str) -> String {
    let h = hover(src).unwrap();
    match h.contents {
        tower_lsp::lsp_types::HoverContents::Markup(m) => m.value,
        other => panic!("expected Markup hover contents, got {other:?}"),
    }
}

#[test]
fn hover_on_a_documented_function_shows_signature_and_docs() {
    let md =
        hover_markdown("# Adds things.\n# Second line.\ndef add(a, b=2) -> Int = a + b\n$0add(1)");
    assert_eq!(
        md,
        "```craw\ndef add(a, b=2) -> Int\n```\n\n---\nAdds things.\nSecond line."
    );
}

#[test]
fn hover_on_a_builtin_shows_its_signature_and_markdown() {
    let md = hover_markdown("$0len([1])");
    assert_eq!(
        md,
        "```craw\nlen(xs)\n```\n\n---\nThe number of elements in `xs`."
    );
}

#[test]
fn hover_on_a_data_constructor_shows_its_definition() {
    let md = hover_markdown("data P(x)\n$0P(1)");
    assert_eq!(md, "```craw\ndata P(x)\n```");
}

#[test]
fn hover_on_an_unambiguous_field_names_its_owner() {
    let md = hover_markdown("data P(x)\np = P(1)\np.$0x");
    assert_eq!(md, "```craw\nfield x of P\n```");
}

#[test]
fn hover_on_a_plain_variable_names_its_scope() {
    let md = hover_markdown("x = 1\n$0x");
    assert_eq!(md, "```craw\nx: Int\n```\n\n(variable, module scope)");
}

#[test]
fn hover_on_a_variable_of_unknown_type_shows_just_its_name() {
    let md = hover_markdown("x = input()\n$0x");
    assert_eq!(md, "```craw\nx\n```\n\n(variable, module scope)");
}

#[test]
fn document_highlight_marks_write_and_read_occurrences() {
    let (text, line, col) = cursor("$0x = 1\nx = x + 1\nprint(x)\ny = 2\n");
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", &text);
    let highlights = a.document_highlight("file:///m.craw", line, col).unwrap();
    let kinds: Vec<_> = highlights.iter().map(|h| h.kind).collect();
    use tower_lsp::lsp_types::DocumentHighlightKind as K;
    assert_eq!(
        kinds,
        vec![Some(K::WRITE), Some(K::WRITE), Some(K::READ), Some(K::READ)]
    );
}

#[test]
fn prepare_rename_rejects_keywords_and_builtins_but_accepts_a_variable() {
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", "x = 1\nlen(x)\n");
    assert!(a.prepare_rename("file:///m.craw", 1, 0).is_none()); // "len", a builtin
    let (range, placeholder) = a.prepare_rename("file:///m.craw", 0, 0).unwrap();
    assert_eq!(placeholder, "x");
    assert_eq!((range.start.character, range.end.character), (0, 1));
}

#[test]
fn rename_rejects_invalid_new_names() {
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", "x = 1\nprint(x)\n");
    assert!(
        a.rename_symbol("file:///m.craw", 0, 0, "def".to_string())
            .is_err()
    );
    assert!(
        a.rename_symbol("file:///m.craw", 0, 0, "1abc".to_string())
            .is_err()
    );
    assert!(
        a.rename_symbol("file:///m.craw", 0, 0, "y2".to_string())
            .unwrap()
            .is_some()
    );
}

fn signature_help(src: &str) -> Option<tower_lsp::lsp_types::SignatureHelp> {
    let (text, line, col) = cursor(src);
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", &text);
    a.get_signature_help("file:///m.craw", line, col)
}

#[test]
fn signature_help_for_a_known_def() {
    let help = signature_help("def solve(a, b, tol=1e-9) = a\nsolve(1, $0").unwrap();
    assert_eq!(help.signatures[0].label, "solve(a, b, tol=1e-9)");
    assert_eq!(help.active_parameter, Some(1));
    assert_eq!(
        help.signatures[0]
            .parameters
            .as_ref()
            .map(|p| p.len())
            .unwrap_or(0),
        3
    );
}

#[test]
fn signature_help_for_a_builtin() {
    let help = signature_help("len($0").unwrap();
    assert_eq!(help.signatures[0].label, "len(xs)");
}

#[test]
fn signature_help_for_a_data_constructor() {
    let help = signature_help("data P(x, y)\nP(1, $0").unwrap();
    assert_eq!(help.signatures[0].label, "P(x, y)");
    assert_eq!(help.active_parameter, Some(1));
}

#[test]
fn signature_help_vararg_absorbs_every_argument() {
    let help = signature_help("def f(*xs) = xs\nf(1, 2, $0").unwrap();
    assert_eq!(help.active_parameter, Some(0));
}

#[test]
fn no_signature_help_when_not_in_a_call() {
    assert!(signature_help("x = (1 + $0").is_none());
}

#[test]
fn signature_help_inside_auto_closed_parens() {
    let help = signature_help("def solve(a, b) = a\nsolve($0)").unwrap();
    assert_eq!(help.active_parameter, Some(0));
    let help = signature_help("def solve(a, b) = a\nsolve(1, $0)").unwrap();
    assert_eq!(help.active_parameter, Some(1));
}

#[test]
fn signature_help_not_offered_before_the_open_paren() {
    assert!(signature_help("def solve(a, b) = a\nsolve$0()").is_none());
}

#[test]
fn member_completion_infers_from_the_cursors_own_scope() {
    let items =
        complete("data P(x, y)\ndef f():\n    s = P(1, 2)\n    s.$0\ndef g():\n    s = \"hi\"\n");
    let labels: Vec<_> = items.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"x") && labels.contains(&"y"), "{labels:?}");
    assert!(!labels.contains(&"upper"), "{labels:?}");
}
