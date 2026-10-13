//! Tests for the editor features in `craw::ide` (semantic tokens, folding,
//! selection ranges, inlay hints, code actions, formatting, call
//! hierarchy) and the analyzer entry points that expose them, plus the
//! hover / signature-help / completion / type-definition upgrades.

use craw::ide::semantic_tokens::{self, Classified};
use craw::lsp_analyzer::{LspAnalyzer, ServerConfig};
use craw::semantic::SemanticModel;
use craw::text::{LineIndex, Pos, Span};
use tower_lsp::lsp_types::*;

const URI: &str = "file:///t.craw";

/// Splits `src` at its `$0` marker: `(text, line, UTF-16 column)`.
fn cursor(src: &str) -> (String, u32, u32) {
    let idx = src.find("$0").expect("fixture needs a $0 marker");
    let text = format!("{}{}", &src[..idx], &src[idx + 2..]);
    let before = &src[..idx];
    let line = before.matches('\n').count() as u32;
    let col_start = before.rfind('\n').map_or(0, |i| i + 1);
    (
        text,
        line,
        before[col_start..].encode_utf16().count() as u32,
    )
}

/// Splits `src` at a `$[ ... $]` selection: `(text, range)`.
fn selection(src: &str) -> (String, Range) {
    let a = src.find("$[").expect("fixture needs $[");
    let without_a = format!("{}{}", &src[..a], &src[a + 2..]);
    let b = without_a.find("$]").expect("fixture needs $]");
    let text = format!("{}{}", &without_a[..b], &without_a[b + 2..]);
    let li = LineIndex::new(&text);
    (
        text,
        Range {
            start: li.position(a).into(),
            end: li.position(b).into(),
        },
    )
}

fn analyzer(text: &str) -> LspAnalyzer {
    let mut a = LspAnalyzer::new();
    a.update_document(URI, text);
    a
}

fn whole(text: &str) -> Range {
    let li = LineIndex::new(text);
    Range {
        start: Position::new(0, 0),
        end: li.position(text.len()).into(),
    }
}

/// Applies non-overlapping LSP edits to `text`.
fn apply(text: &str, edits: &[TextEdit]) -> String {
    let li = LineIndex::new(text);
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|e| {
            (
                li.offset(Pos::from(e.range.start)).unwrap_or(text.len()),
                li.offset(Pos::from(e.range.end)).unwrap_or(text.len()),
                e.new_text.as_str(),
            )
        })
        .collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut out = text.to_string();
    let mut last_start = usize::MAX;
    for (s, e, t) in spans {
        assert!(e <= last_start, "overlapping edits");
        // Deleting a whole final line may address one line past the end.
        let e = e.max(s);
        out.replace_range(s..e.min(out.len()), t);
        last_start = s;
    }
    out
}

// ---------------------------------------------------------------------
// Semantic tokens
// ---------------------------------------------------------------------

fn classified(text: &str) -> Vec<(String, &'static str, Vec<&'static str>)> {
    let model = SemanticModel::build(text);
    semantic_tokens::classify(&model, &craw::ide::NoCrossFile)
        .into_iter()
        .map(
            |Classified {
                 span,
                 token_type,
                 modifiers,
             }| {
                (
                    text[span.start..span.end].to_string(),
                    token_type,
                    modifiers,
                )
            },
        )
        .collect()
}

fn type_of<'a>(
    toks: &'a [(String, &'static str, Vec<&'static str>)],
    name: &str,
    nth: usize,
) -> &'a (String, &'static str, Vec<&'static str>) {
    toks.iter()
        .filter(|t| t.0 == name)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of `{name}` in {toks:?}"))
}

#[test]
fn semantic_tokens_distinguish_parameters_locals_and_globals() {
    let toks = classified("g = 1\ndef f(a):\n    b = a + g\n    return b\n");
    assert_eq!(type_of(&toks, "g", 0).1, "variable");
    assert!(type_of(&toks, "g", 0).2.contains(&"global"));
    assert!(type_of(&toks, "g", 1).2.contains(&"global"));
    assert_eq!(type_of(&toks, "a", 0).1, "parameter");
    assert_eq!(type_of(&toks, "a", 1).1, "parameter");
    assert_eq!(type_of(&toks, "b", 0).1, "variable");
    assert!(!type_of(&toks, "b", 0).2.contains(&"global"));
    assert_eq!(type_of(&toks, "f", 0).1, "function");
}

#[test]
fn semantic_tokens_mark_definitions_and_writes() {
    let toks = classified("x = 1\nx = 2\nprint(x)\n");
    assert!(type_of(&toks, "x", 0).2.contains(&"declaration"));
    assert!(type_of(&toks, "x", 1).2.contains(&"modification"));
    let read = &type_of(&toks, "x", 2).2;
    assert!(!read.contains(&"declaration") && !read.contains(&"modification"));
}

#[test]
fn semantic_tokens_tag_builtins_as_default_library() {
    let toks = classified("print(len([1]))\n");
    for name in ["print", "len"] {
        let t = type_of(&toks, name, 0);
        assert_eq!(t.1, "function");
        assert!(t.2.contains(&"defaultLibrary"), "{name}: {:?}", t.2);
    }
}

#[test]
fn semantic_tokens_classify_types_fields_and_unresolved_names() {
    let toks = classified("data P(x)\np = P(1)\nprint(p.x)\nprint(nope)\n");
    assert_eq!(type_of(&toks, "P", 0).1, "class");
    assert_eq!(type_of(&toks, "P", 1).1, "class");
    assert_eq!(type_of(&toks, "x", 1).1, "property");
    assert_eq!(type_of(&toks, "nope", 0).1, "unresolvedReference");
}

#[test]
fn semantic_tokens_cover_names_interpolated_in_fstrings() {
    let toks = classified("name = 1\ns = f\"hi {name}\"\n");
    assert_eq!(type_of(&toks, "name", 1).1, "variable");
}

#[test]
fn semantic_tokens_skip_rust_blocks() {
    let toks = classified("rust:\n    fn helper() { let q = 1; }\nx = 1\n");
    assert!(
        toks.iter().all(|t| t.0 != "helper" && t.0 != "q"),
        "{toks:?}"
    );
    assert!(toks.iter().any(|t| t.0 == "x"));
}

#[test]
fn semantic_tokens_are_sorted_and_non_overlapping() {
    let src = std::fs::read_to_string("samples/master_showcase.craw").unwrap();
    let model = SemanticModel::build(&src);
    let toks = semantic_tokens::classify(&model, &craw::ide::NoCrossFile);
    for w in toks.windows(2) {
        assert!(
            w[0].span.end <= w[1].span.start,
            "{:?} overlaps {:?}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn semantic_tokens_encode_utf16_deltas() {
    // `日本` is 2 UTF-16 units but 6 UTF-8 bytes.
    let text = "s = \"日本\"; t = s\nu = t\n";
    let a = analyzer(text);
    let data = a.semantic_tokens(URI, None).unwrap();
    // Tokens: s(0,0) t(0,10) s(0,14) u(1,0) t(1,4) — `"日本"` is 4 units.
    let mut line = 0;
    let mut col = 0;
    let mut abs = Vec::new();
    for t in &data {
        if t.delta_line > 0 {
            line += t.delta_line;
            col = t.delta_start;
        } else {
            col += t.delta_start;
        }
        abs.push((line, col, t.length));
    }
    assert_eq!(
        abs,
        vec![(0, 0, 1), (0, 10, 1), (0, 14, 1), (1, 0, 1), (1, 4, 1)]
    );
    let legend = semantic_tokens::legend();
    assert_eq!(legend.token_types.len(), semantic_tokens::TOKEN_TYPES.len());
    assert!(
        data.iter()
            .all(|t| (t.token_type as usize) < legend.token_types.len())
    );
}

#[test]
fn semantic_tokens_range_request_only_returns_that_range() {
    let text = "a = 1\nb = 2\nc = 3\n";
    let a = analyzer(text);
    let data = a
        .semantic_tokens(
            URI,
            Some(Range {
                start: Position::new(1, 0),
                end: Position::new(1, 5),
            }),
        )
        .unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].delta_line, 1);
}

#[test]
fn semantic_tokens_mark_decorators() {
    let toks = classified("def deco(f):\n    return f\n@deco\ndef g():\n    return 1\n");
    let uses: Vec<_> = toks.iter().filter(|t| t.0 == "deco").collect();
    assert!(uses.iter().any(|t| t.1 == "decorator"), "{uses:?}");
}

// ---------------------------------------------------------------------
// Folding
// ---------------------------------------------------------------------

fn folds(text: &str) -> Vec<(u32, u32, Option<FoldingRangeKind>)> {
    analyzer(text)
        .folding_ranges(URI)
        .unwrap()
        .into_iter()
        .map(|f| (f.start_line, f.end_line, f.kind))
        .collect()
}

#[test]
fn folding_covers_function_bodies_and_nested_blocks() {
    let f = folds("def f(a):\n    if a:\n        x = 1\n        y = 2\n    return a\nz = 1\n");
    assert!(f.contains(&(0, 4, None)), "{f:?}");
    assert!(f.contains(&(1, 3, None)), "{f:?}");
}

#[test]
fn folding_ignores_trailing_blank_lines_in_a_block() {
    let f = folds("def f():\n    x = 1\n    return x\n\n\ny = 2\n");
    assert!(f.contains(&(0, 2, None)), "{f:?}");
}

#[test]
fn folding_covers_multi_line_brackets_but_keeps_the_closer_visible() {
    let f = folds("xs = [\n    1,\n    2,\n]\n");
    assert!(f.contains(&(0, 2, None)), "{f:?}");
}

#[test]
fn folding_covers_comment_runs_regions_and_imports() {
    let text = "from std.math import sqrt\nfrom std.fs import read\n\n# one\n# two\n# three\nx = 1\n# region setup\ny = 2\nz = 3\n# endregion\n";
    let f = folds(text);
    assert!(
        f.contains(&(0, 1, Some(FoldingRangeKind::Imports))),
        "{f:?}"
    );
    assert!(
        f.contains(&(3, 5, Some(FoldingRangeKind::Comment))),
        "{f:?}"
    );
    assert!(
        f.contains(&(7, 10, Some(FoldingRangeKind::Region))),
        "{f:?}"
    );
}

#[test]
fn folding_a_single_comment_line_does_nothing() {
    let f = folds("# only\nx = 1\n");
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn folding_covers_rust_blocks() {
    let f = folds("rust:\n    fn a() {}\n    fn b() {}\nx = 1\n");
    assert!(f.contains(&(0, 2, None)), "{f:?}");
}

#[test]
fn folding_ranges_have_unique_start_lines() {
    let src = std::fs::read_to_string("samples/master_showcase.craw").unwrap();
    let f = folds(&src);
    let mut starts: Vec<_> = f.iter().map(|r| r.0).collect();
    starts.dedup();
    assert_eq!(starts.len(), f.len());
    assert!(f.iter().all(|r| r.1 > r.0));
}

// ---------------------------------------------------------------------
// Selection ranges
// ---------------------------------------------------------------------

fn chain(src: &str) -> (String, Vec<String>) {
    let (text, line, col) = cursor(src);
    let a = analyzer(&text);
    let mut r = a
        .selection_ranges(URI, &[Position::new(line, col)])
        .unwrap()
        .remove(0);
    let li = LineIndex::new(&text);
    let mut out = Vec::new();
    loop {
        let s = li.offset(r.range.start.into()).unwrap();
        let e = li.offset(r.range.end.into()).unwrap();
        out.push(text[s..e].to_string());
        match r.parent {
            Some(p) => r = *p,
            None => break,
        }
    }
    (text, out)
}

#[test]
fn selection_expands_from_word_to_args_to_call_to_statement_to_block() {
    let (text, c) = chain("def f(a, b):\n    print(a, $0b)\n    return a\n");
    assert_eq!(c[0], "b");
    assert_eq!(c[1], "a, b");
    assert_eq!(c[2], "(a, b)");
    assert_eq!(c[3], "print(a, b)");
    assert_eq!(c[4], "print(a, b)\n    return a");
    assert!(c[5].starts_with("def f(a, b):") && c[5].ends_with("return a"));
    assert_eq!(c.last().unwrap(), &text);
}

#[test]
fn selection_ranges_strictly_grow() {
    let (_, c) = chain("x = [1, [2, $03], 4]\n");
    for w in c.windows(2) {
        assert!(
            w[1].len() > w[0].len() && w[1].contains(w[0].as_str()),
            "{c:?}"
        );
    }
    assert_eq!(&c[..4], &["3", "2, 3", "[2, 3]", "1, [2, 3], 4"]);
}

#[test]
fn selection_handles_one_response_per_position() {
    let a = analyzer("x = 1\ny = 2\n");
    let r = a
        .selection_ranges(URI, &[Position::new(0, 0), Position::new(1, 0)])
        .unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[1].range.start.line, 1);
}

// ---------------------------------------------------------------------
// Inlay hints
// ---------------------------------------------------------------------

fn hints_with(text: &str, config: ServerConfig) -> Vec<(u32, u32, String)> {
    let mut a = LspAnalyzer::new();
    a.set_config(config);
    a.update_document(URI, text);
    a.inlay_hints(URI, whole(text))
        .unwrap()
        .into_iter()
        .map(|h| {
            let InlayHintLabel::String(l) = h.label else {
                panic!()
            };
            (h.position.line, h.position.character, l)
        })
        .collect()
}

fn hints(text: &str) -> Vec<(u32, u32, String)> {
    hints_with(text, ServerConfig::default())
}

fn labels(text: &str) -> Vec<String> {
    hints(text).into_iter().map(|h| h.2).collect()
}

#[test]
fn parameter_hints_label_positional_arguments() {
    let h = hints("def area(width, height):\n    return width * height\nprint(area(3, 4))\n");
    assert!(h.contains(&(2, 11, "width:".into())), "{h:?}");
    assert!(h.contains(&(2, 14, "height:".into())), "{h:?}");
}

#[test]
fn parameter_hints_are_skipped_when_the_argument_says_the_same_thing() {
    let l = labels("def f(width, height):\n    return width\nwidth = 1\nf(width, self.height)\n");
    assert!(!l.iter().any(|l| l == "width:" || l == "height:"), "{l:?}");
}

#[test]
fn parameter_hints_stop_at_the_first_keyword_argument() {
    let l = labels("def f(a, bb, cc):\n    return a\nf(1, cc=2, bb=3)\n");
    assert!(l.contains(&"a:".to_string()));
    assert!(
        !l.contains(&"bb:".to_string()) && !l.contains(&"cc:".to_string()),
        "{l:?}"
    );
}

#[test]
fn parameter_hints_label_data_constructor_fields() {
    let l = labels("data Point(xx, yy)\nPoint(1, 2)\n");
    assert!(
        l.contains(&"xx:".to_string()) && l.contains(&"yy:".to_string()),
        "{l:?}"
    );
}

#[test]
fn parameter_hints_label_only_the_first_vararg_argument() {
    let l = labels("def f(first, *rest):\n    return first\nf(1, 2, 3)\n");
    assert_eq!(l.iter().filter(|l| *l == "rest:").count(), 1, "{l:?}");
    assert!(l.contains(&"first:".to_string()));
}

#[test]
fn parameter_hints_skip_builtins_and_definitions() {
    let l = labels("def f(value):\n    return value\nprint(1)\n");
    assert!(l.iter().all(|l| !l.ends_with(':')), "{l:?}");
}

#[test]
fn type_hints_show_inferred_types_of_non_obvious_initializers() {
    let h = hints("a = 1\nb = a + 1.5\nc = a * 2\nd = str(a) + \"!\"\n");
    assert!(h.contains(&(1, 1, ": Float".into())), "{h:?}");
    assert!(h.contains(&(2, 1, ": Int".into())), "{h:?}");
    assert!(h.contains(&(3, 1, ": String".into())), "{h:?}");
}

#[test]
fn type_hints_are_hidden_for_literals_and_constructor_calls() {
    let l = labels("data P(x)\na = 1\ns = \"x\"\np = P(1)\nxs = [1]\n");
    assert!(!l.iter().any(|l| l.starts_with(": ")), "{l:?}");
}

#[test]
fn type_hints_follow_aliases_to_user_types() {
    let h = hints("data P(x)\np = P(1)\nq = p\n");
    assert!(h.contains(&(2, 1, ": P".into())), "{h:?}");
}

#[test]
fn type_hints_skip_reassignments_annotations_and_unknowns() {
    let l = labels("a = 1\nb = a + 1\nb = a + 2\nc: Int = a + 1\nd = input()\n");
    assert_eq!(l.iter().filter(|l| l.starts_with(": ")).count(), 1, "{l:?}");
}

#[test]
fn type_hints_work_inside_function_bodies() {
    let h = hints("def f(n: Int):\n    m = n + 1\n    return m\n");
    assert!(h.contains(&(1, 5, ": Int".into())), "{h:?}");
}

#[test]
fn return_type_hints_appear_when_every_return_agrees() {
    let h = hints("def f(n: Int):\n    if n > 0:\n        return n * 2\n    return 0\n");
    assert!(h.contains(&(0, 13, "-> Int".into())), "{h:?}");
}

#[test]
fn return_type_hints_are_omitted_on_disagreement_annotation_or_generator() {
    let l = labels(concat!(
        "def a(n: Int):\n    if n > 0:\n        return n\n    return \"no\"\n",
        "def b() -> Int:\n    return 1\n",
        "def c(x):\n    return x\n",
    ));
    assert!(!l.iter().any(|l| l.starts_with("->")), "{l:?}");
}

#[test]
fn inlay_hint_kinds_can_be_disabled_individually() {
    let text = "def f(width):\n    return str(width)\nv = f(2) + \"!\"\n";
    let all = labels(text);
    assert!(all.contains(&"width:".to_string()));
    assert!(all.iter().any(|l| l.starts_with("->")));
    let off = |p: bool, t: bool, r: bool| {
        let mut c = ServerConfig::default();
        c.inlay.parameter_names = p;
        c.inlay.types = t;
        c.inlay.return_types = r;
        hints_with(text, c)
            .into_iter()
            .map(|h| h.2)
            .collect::<Vec<_>>()
    };
    assert!(!off(false, true, true).contains(&"width:".to_string()));
    assert!(!off(true, true, false).iter().any(|l| l.starts_with("->")));
    assert!(off(false, false, false).is_empty());
}

#[test]
fn inlay_hints_respect_the_requested_range() {
    let text = "def f(width):\n    return width\nf(1)\nf(2)\n";
    let a = analyzer(text);
    let h = a
        .inlay_hints(
            URI,
            Range {
                start: Position::new(3, 0),
                end: Position::new(3, 4),
            },
        )
        .unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].position.line, 3);
}

// ---------------------------------------------------------------------
// Code actions
// ---------------------------------------------------------------------

fn actions_at(src: &str) -> (String, Vec<CodeAction>) {
    let (text, range) = if src.contains("$[") {
        selection(src)
    } else {
        let (t, l, c) = cursor(src);
        (
            t,
            Range {
                start: Position::new(l, c),
                end: Position::new(l, c),
            },
        )
    };
    let a = analyzer(&text);
    let actions = a.code_actions(URI, range).unwrap();
    (text, actions)
}

fn apply_action(text: &str, action: &CodeAction) -> String {
    let edit = action.edit.as_ref().unwrap();
    let edits = &edit.changes.as_ref().unwrap()[&Url::parse(URI).unwrap()];
    apply(text, edits)
}

fn find<'a>(actions: &'a [CodeAction], title: &str) -> &'a CodeAction {
    actions
        .iter()
        .find(|a| a.title == title)
        .unwrap_or_else(|| {
            panic!(
                "no action `{title}` in {:?}",
                actions.iter().map(|a| &a.title).collect::<Vec<_>>()
            )
        })
}

#[test]
fn quick_fix_prefixes_an_unused_local_everywhere() {
    let (text, actions) = actions_at("def f():\n    $0tmp = 1\n    tmp = 2\n    return 0\n");
    let a = find(&actions, "Prefix `tmp` with an underscore");
    assert_eq!(a.kind, Some(CodeActionKind::QUICKFIX));
    assert_eq!(a.is_preferred, Some(true));
    assert!(
        a.diagnostics.as_ref().unwrap()[0].code == Some(NumberOrString::String("W0200".into()))
    );
    assert_eq!(
        apply_action(&text, a),
        "def f():\n    _tmp = 1\n    _tmp = 2\n    return 0\n"
    );
}

#[test]
fn quick_fix_suggests_the_closest_visible_names() {
    let (text, actions) = actions_at("total = 0\nprint($0totl)\n");
    let a = find(&actions, "Change to `total`");
    assert_eq!(a.is_preferred, Some(true));
    assert_eq!(apply_action(&text, a), "total = 0\nprint(total)\n");
}

#[test]
fn quick_fix_marks_no_suggestion_preferred_when_two_tie() {
    // `countr` is one edit from both `counter` and the builtin `count`.
    let (_, actions) = actions_at("counter = 0\nprint($0countr)\n");
    assert_eq!(find(&actions, "Change to `counter`").is_preferred, None);
    assert_eq!(find(&actions, "Change to `count`").is_preferred, None);
}

#[test]
fn quick_fix_suggests_builtins_too() {
    let (_, actions) = actions_at("$0prnt(1)\n");
    find(&actions, "Change to `print`");
}

#[test]
fn quick_fix_offers_nothing_for_names_with_no_close_match() {
    let (_, actions) = actions_at("print($0zzzzqqq)\n");
    assert!(
        !actions.iter().any(|a| a.title.starts_with("Change to")),
        "{:?}",
        actions.iter().map(|a| &a.title).collect::<Vec<_>>()
    );
}

#[test]
fn quick_fix_only_suggests_names_visible_at_the_use() {
    let (_, actions) = actions_at("def f():\n    secret = 1\n    return secret\nprint($0secre)\n");
    assert!(!actions.iter().any(|a| a.title == "Change to `secret`"));
}

#[test]
fn refactor_adds_an_inferred_type_annotation() {
    let (text, actions) = actions_at("a = 1\n$0b = a + 1\n");
    let act = find(&actions, "Add explicit type annotation `Int`");
    assert_eq!(act.kind, Some(CodeActionKind::REFACTOR_REWRITE));
    assert_eq!(apply_action(&text, act), "a = 1\nb: Int = a + 1\n");
    // The annotated result must still parse and analyze cleanly.
    let after = apply_action(&text, act);
    assert!(SemanticModel::build(&after).syntax_errors().is_empty());
}

#[test]
fn refactor_extracts_a_selected_expression_into_a_variable() {
    let (text, actions) = actions_at("def f(a, b):\n    print($[a + b$], 1)\n    return a\n");
    let act = find(&actions, "Extract into variable");
    assert_eq!(act.kind, Some(CodeActionKind::REFACTOR_EXTRACT));
    let after = apply_action(&text, act);
    assert_eq!(
        after,
        "def f(a, b):\n    value = a + b\n    print(value, 1)\n    return a\n"
    );
    assert!(SemanticModel::build(&after).syntax_errors().is_empty());
}

#[test]
fn refactor_extract_picks_a_fresh_name() {
    let (text, actions) = actions_at("value = 1\nprint($[value * 2$])\n");
    let after = apply_action(&text, find(&actions, "Extract into variable"));
    assert_eq!(after, "value = 1\nvalue1 = value * 2\nprint(value1)\n");
}

#[test]
fn refactor_extract_refuses_partial_tokens_unbalanced_and_lambda_bodies() {
    for src in [
        "print(ab$[cd$])\n",
        "print($[(a$] + 1))\n",
        "f = filter(x => $[x + 1$], [1])\n",
        "$[x = 1$]\n",
    ] {
        let (_, actions) = actions_at(src);
        assert!(
            !actions.iter().any(|a| a.title == "Extract into variable"),
            "offered for {src:?}"
        );
    }
}

#[test]
fn refactor_inlines_a_single_assignment_variable() {
    let (text, actions) = actions_at("a = 1\n$0t = a + 1\nprint(t)\nprint(t * 2)\n");
    let act = find(&actions, "Inline variable `t`");
    assert_eq!(
        apply_action(&text, act),
        "a = 1\nprint((a + 1))\nprint((a + 1) * 2)\n"
    );
}

#[test]
fn refactor_inline_works_from_a_use_site_and_skips_parens_for_one_token() {
    let (text, actions) = actions_at("t = 5\nprint($0t)\n");
    assert_eq!(
        apply_action(&text, find(&actions, "Inline variable `t`")),
        "print(5)\n"
    );
}

#[test]
fn refactor_inline_refuses_when_it_could_change_meaning() {
    for src in [
        "$0t = 1\nt = 2\nprint(t)\n",          // reassigned
        "$0t = input()\nprint(t)\nprint(t)\n", // has a call
        "a = 1\na = 2\n$0t = a\nprint(t)\n",   // reads a reassigned name
    ] {
        let (_, actions) = actions_at(src);
        assert!(
            !actions
                .iter()
                .any(|a| a.title.starts_with("Inline variable")),
            "offered for {src:?}"
        );
    }
}

// ---------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------

fn fmt(text: &str) -> String {
    let a = analyzer(text);
    let edits = a.format(URI, None).unwrap();
    let applied = apply(text, &edits);
    assert_eq!(
        applied,
        craw::ide::formatting::format_text(&SemanticModel::build(text)),
        "edits and format_text disagree"
    );
    applied
}

#[test]
fn format_spaces_binary_operators_and_commas() {
    assert_eq!(fmt("x=1+2*3\n"), "x = 1 + 2 * 3\n");
    assert_eq!(fmt("print( a,b )\n"), "print(a, b)\n");
    assert_eq!(fmt("y = x|>f\n"), "y = x |> f\n");
    assert_eq!(fmt("a = b==c\n"), "a = b == c\n");
    assert_eq!(fmt("f = x=>x+1\n"), "f = x => x + 1\n");
}

#[test]
fn format_keeps_keyword_arguments_and_defaults_tight() {
    assert_eq!(
        fmt("def f(a,b = 2):\n    return a\n"),
        "def f(a, b=2):\n    return a\n"
    );
    assert_eq!(fmt("f(1, b = 2)\n"), "f(1, b=2)\n");
}

#[test]
fn format_handles_unary_operators() {
    assert_eq!(fmt("x = - 1\n"), "x = -1\n");
    assert_eq!(fmt("y = x * - 1\n"), "y = x * -1\n");
    assert_eq!(fmt("z = f(- x)\n"), "z = f(-x)\n");
    assert_eq!(fmt("w = a-b\n"), "w = a - b\n");
}

#[test]
fn format_removes_space_before_call_and_index_brackets() {
    assert_eq!(fmt("print (xs [0])\n"), "print(xs[0])\n");
    assert_eq!(fmt("x = p . y\n"), "x = p.y\n");
}

#[test]
fn format_spaces_dict_colons_but_preserves_slices() {
    assert_eq!(fmt("d = {\"a\":1}\n"), "d = {\"a\": 1}\n");
    assert_eq!(fmt("s = xs[1:2]\n"), "s = xs[1:2]\n");
    assert_eq!(fmt("t = xs[1 : 2]\n"), "t = xs[1 : 2]\n");
}

#[test]
fn format_normalizes_statement_headers() {
    assert_eq!(
        fmt("if x>1 :\n    print(x)\nelse :\n    print(0)\n"),
        "if x > 1:\n    print(x)\nelse:\n    print(0)\n"
    );
}

#[test]
fn format_keeps_comments_with_two_spaces_before_them() {
    assert_eq!(
        fmt("x=1 # note\n# own line\n"),
        "x = 1  # note\n# own line\n"
    );
    assert_eq!(fmt("s = \"#notacomment\"\n"), "s = \"#notacomment\"\n");
}

#[test]
fn format_cleans_whitespace_and_blank_lines() {
    assert_eq!(
        fmt("\n\nx = 1   \n\n\n\n\ny = 2\n\n\n"),
        "x = 1\n\n\ny = 2\n"
    );
    assert_eq!(fmt("x = 1"), "x = 1\n");
}

#[test]
fn format_preserves_crlf_line_endings() {
    assert_eq!(fmt("x=1\r\ny=2\r\n"), "x = 1\r\ny = 2\r\n");
}

#[test]
fn format_leaves_rust_blocks_and_multiline_brackets_alone() {
    let rust = "rust:\n    fn  a( ) {   }   \nx=1\n";
    assert_eq!(fmt(rust), "rust:\n    fn  a( ) {   }   \nx = 1\n");
    let bracket = "xs = [1,2,\n      3,4]\n";
    assert_eq!(fmt(bracket), bracket);
}

#[test]
fn format_preserves_generic_type_spacing() {
    assert_eq!(
        fmt("def f(x: List<Int>) -> Int:\n    return 1\n"),
        "def f(x: List<Int>) -> Int:\n    return 1\n"
    );
}

#[test]
fn format_of_an_empty_document_is_empty() {
    assert!(analyzer("").format(URI, None).unwrap().is_empty());
    assert!(analyzer("\n\n").format(URI, None).unwrap().is_empty());
}

#[test]
fn format_never_changes_tokens_and_is_idempotent_on_every_sample() {
    let mut files = Vec::new();
    for dir in ["samples", "tests"] {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "craw") {
                files.push(p);
            }
        }
    }
    assert!(!files.is_empty());
    for p in files {
        let src = std::fs::read_to_string(&p).unwrap();
        let once = fmt(&src);
        let lex = |s: &str| {
            let mut t = craw::lexer::Lexer::new(s).tokenize();
            while matches!(
                t.last(),
                Some(
                    craw::lexer::Token::Eof
                        | craw::lexer::Token::Newline
                        | craw::lexer::Token::Dedent
                )
            ) {
                t.pop();
            }
            t
        };
        assert_eq!(
            lex(&src),
            lex(&once),
            "tokens changed formatting {}",
            p.display()
        );
        let comments = |s: &str| {
            s.lines()
                .filter(|l| l.trim_start().starts_with('#'))
                .count()
        };
        assert_eq!(
            comments(&src),
            comments(&once),
            "comment lines changed in {}",
            p.display()
        );
        assert_eq!(once, fmt(&once), "not idempotent on {}", p.display());
    }
}

#[test]
fn range_formatting_only_touches_the_selected_lines() {
    let text = "a=1\nb=2\nc=3\n";
    let a = analyzer(text);
    let edits = a
        .format(
            URI,
            Some(Range {
                start: Position::new(1, 0),
                end: Position::new(1, 3),
            }),
        )
        .unwrap();
    assert_eq!(apply(text, &edits), "a=1\nb = 2\nc=3\n");
}

// ---------------------------------------------------------------------
// Call hierarchy
// ---------------------------------------------------------------------

const CALLS: &str = "def leaf():\n    return 1\ndef mid():\n    return leaf() + leaf()\ndef top():\n    return mid()\nprint(leaf())\n";

#[test]
fn call_hierarchy_prepares_only_on_callables() {
    let a = analyzer(CALLS);
    let item = a.prepare_call_hierarchy(URI, 0, 5).unwrap().remove(0);
    assert_eq!(item.name, "leaf");
    assert_eq!(item.kind, SymbolKind::FUNCTION);
    let b = analyzer("x = 1\nprint(x)\n");
    assert!(b.prepare_call_hierarchy(URI, 1, 6).is_none());
}

#[test]
fn incoming_calls_group_by_caller_including_module_code() {
    let a = analyzer(CALLS);
    let item = a.prepare_call_hierarchy(URI, 0, 5).unwrap().remove(0);
    let incoming = a.incoming_calls(&item).unwrap();
    let mut callers: Vec<_> = incoming
        .iter()
        .map(|c| (c.from.name.clone(), c.from_ranges.len()))
        .collect();
    callers.sort();
    assert_eq!(
        callers,
        vec![("mid".to_string(), 2), ("t.craw".to_string(), 1)]
    );
}

#[test]
fn outgoing_calls_list_callees_once_each() {
    let a = analyzer(CALLS);
    let top = a.prepare_call_hierarchy(URI, 4, 5).unwrap().remove(0);
    let out = a.outgoing_calls(&top).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].to.name, "mid");
    let mid = a.prepare_call_hierarchy(URI, 2, 5).unwrap().remove(0);
    let out = a.outgoing_calls(&mid).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].from_ranges.len(), 2);
}

#[test]
fn outgoing_calls_exclude_those_made_by_nested_functions() {
    let a = analyzer(
        "def g():\n    return 1\ndef f():\n    def inner():\n        return g()\n    return inner()\n",
    );
    let f = a.prepare_call_hierarchy(URI, 2, 5).unwrap().remove(0);
    let names: Vec<_> = a
        .outgoing_calls(&f)
        .unwrap()
        .into_iter()
        .map(|c| c.to.name)
        .collect();
    assert_eq!(names, vec!["inner"]);
}

// ---------------------------------------------------------------------
// Hover, type definition, signature help, completion, syntax tree
// ---------------------------------------------------------------------

fn hover(src: &str) -> Option<String> {
    let (text, line, col) = cursor(src);
    analyzer(&text)
        .get_hover_info(URI, line, col)
        .map(|h| match h.contents {
            HoverContents::Markup(m) => m.value,
            _ => panic!(),
        })
}

#[test]
fn hover_on_keywords_shows_their_syntax() {
    let md = hover("$0while x:\n    x = x - 1\n").unwrap();
    assert!(md.starts_with("```craw\nwhile cond:"), "{md}");
    let md = hover("def f():\n    $0return 1\n").unwrap();
    assert!(md.contains("Returns a value"), "{md}");
}

#[test]
fn hover_shows_a_parameters_declared_type() {
    let md = hover("def f(n: Int):\n    return $0n\n").unwrap();
    assert!(md.starts_with("```craw\nn: Int\n```"), "{md}");
}

#[test]
fn hover_shows_inferred_user_types() {
    let md = hover("data P(x)\np = P(1)\nprint($0p)\n").unwrap();
    assert!(md.starts_with("```craw\np: P\n```"), "{md}");
}

#[test]
fn type_definition_jumps_to_the_inferred_data_class() {
    let (text, line, col) = cursor("data Point(x)\np = Point(1)\nprint($0p)\n");
    let loc = analyzer(&text).type_definition(URI, line, col).unwrap();
    assert_eq!(loc.range.start, Position::new(0, 5));
}

#[test]
fn type_definition_is_none_for_builtin_types() {
    let (text, line, col) = cursor("x = 1\nprint($0x)\n");
    assert!(analyzer(&text).type_definition(URI, line, col).is_none());
}

fn sig(src: &str) -> SignatureHelp {
    let (text, line, col) = cursor(src);
    analyzer(&text).get_signature_help(URI, line, col).unwrap()
}

#[test]
fn signature_help_for_builtins_lists_parameters() {
    let s = sig("filter(f, $0)\n");
    let info = &s.signatures[0];
    let params: Vec<_> = info
        .parameters
        .as_ref()
        .unwrap()
        .iter()
        .map(|p| match &p.label {
            ParameterLabel::Simple(s) => s.clone(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(params, vec!["pred", "xs"]);
    assert_eq!(s.active_parameter, Some(1));
    assert_eq!(info.active_parameter, Some(1));
    assert!(info.documentation.is_some());
}

#[test]
fn signature_help_follows_keyword_arguments() {
    let s = sig("def f(a, b, c):\n    return a\nf(1, c=$0)\n");
    assert_eq!(s.active_parameter, Some(2));
}

#[test]
fn signature_help_carries_doc_comments() {
    let s = sig("# Adds things.\ndef add(a, b):\n    return a\nadd($0)\n");
    match &s.signatures[0].documentation {
        Some(Documentation::MarkupContent(m)) => assert!(m.value.contains("Adds things.")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn signature_help_past_the_last_parameter_highlights_nothing() {
    let s = sig("def f(a):\n    return a\nf(1, $0)\n");
    let n = s.signatures[0].parameters.as_ref().unwrap().len() as u32;
    assert!(s.active_parameter.unwrap() >= n);
}

fn complete(src: &str) -> Vec<CompletionItem> {
    let (text, line, col) = cursor(src);
    analyzer(&text).get_completions(URI, line, col)
}

#[test]
fn completion_kinds_reflect_what_a_local_is() {
    let items = complete("def helper(a):\n    return a\ndata P(x)\ncount = 1\nprint($0)\n");
    let kind = |n: &str| items.iter().find(|i| i.label == n).unwrap().kind;
    assert_eq!(kind("helper"), Some(CompletionItemKind::FUNCTION));
    assert_eq!(kind("P"), Some(CompletionItemKind::CLASS));
    assert_eq!(kind("count"), Some(CompletionItemKind::VARIABLE));
    let helper = items.iter().find(|i| i.label == "helper").unwrap();
    assert_eq!(helper.detail.as_deref(), Some("def helper(a)"));
    assert!(
        helper.command.is_some(),
        "callables trigger parameter hints"
    );
    let count = items.iter().find(|i| i.label == "count").unwrap();
    assert_eq!(count.detail.as_deref(), Some("Int"));
}

#[test]
fn completion_offers_expression_keywords_after_locals() {
    let items = complete("x = 1\ny = $0\n");
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    let mut unique = labels.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), labels.len(), "duplicate labels");
    let not = items.iter().find(|i| i.label == "lambda").unwrap();
    assert_eq!(not.kind, Some(CompletionItemKind::KEYWORD));
    let x = items.iter().find(|i| i.label == "x").unwrap();
    assert!(x.sort_text < not.sort_text);
    assert!(
        !items.iter().any(|i| i.label == "def"),
        "statement keywords stay out"
    );
}

#[test]
fn syntax_tree_dumps_the_ast_and_errors() {
    let a = analyzer("x = 1\n");
    let tree = a.syntax_tree(URI).unwrap();
    assert!(tree.contains("Assign"), "{tree}");
    let b = analyzer("x = (\n");
    assert!(b.syntax_tree(URI).unwrap().contains("syntax errors"));
    assert!(a.syntax_tree("file:///missing.craw").is_none());
}

#[test]
fn features_return_none_for_unknown_documents() {
    let a = LspAnalyzer::new();
    let u = "file:///nope.craw";
    assert!(a.semantic_tokens(u, None).is_none());
    assert!(a.folding_ranges(u).is_none());
    assert!(a.selection_ranges(u, &[Position::new(0, 0)]).is_none());
    assert!(a.inlay_hints(u, Range::default()).is_none());
    assert!(a.code_actions(u, Range::default()).is_none());
    assert!(a.format(u, None).is_none());
    assert!(a.prepare_call_hierarchy(u, 0, 0).is_none());
    assert!(a.type_definition(u, 0, 0).is_none());
}

#[test]
fn features_never_panic_on_broken_or_partial_input() {
    let inputs = [
        "def f(\n",
        "x = [1, 2\n",
        "if x:\n",
        "data\n",
        "print(f\"{\")\n",
        "rust:\n",
        "@\n",
        "x = 1 +\n",
        "def f(a, b):\n    return a +\n",
        ")))\n",
        "\t\tx = 1\n",
        "日本 = 1\n",
    ];
    for text in inputs {
        let a = analyzer(text);
        let r = whole(text);
        let _ = a.semantic_tokens(URI, None);
        let _ = a.semantic_tokens(URI, Some(r));
        let _ = a.folding_ranges(URI);
        let _ = a.inlay_hints(URI, r);
        let _ = a.code_actions(URI, r);
        let _ = a.format(URI, None);
        let _ = a.syntax_tree(URI);
        let li = LineIndex::new(text);
        for off in 0..=text.len() {
            if !text.is_char_boundary(off) {
                continue;
            }
            let p: Position = li.position(off).into();
            let _ = a.selection_ranges(URI, &[p]);
            let _ = a.get_hover_info(URI, p.line, p.character);
            let _ = a.get_signature_help(URI, p.line, p.character);
            let _ = a.get_completions(URI, p.line, p.character);
            let _ = a.type_definition(URI, p.line, p.character);
            let _ = a.prepare_call_hierarchy(URI, p.line, p.character);
            let _ = a.code_actions(URI, Range { start: p, end: p });
        }
    }
}

#[allow(dead_code)]
fn span(a: usize, b: usize) -> Span {
    Span::new(a, b)
}

#[test]
fn type_hints_see_through_calls_to_unannotated_functions() {
    let h = hints("def double(n: Int):\n    return n * 2\ntotal = double(21)\n");
    assert!(h.contains(&(2, 5, ": Int".into())), "{h:?}");
    let h = hints("def greet(n):\n    return \"hi \" + n\ng = greet(1)\n");
    assert!(h.contains(&(2, 1, ": String".into())), "{h:?}");
}

#[test]
fn inference_terminates_on_recursive_functions() {
    // Directly and mutually recursive: inference must give up, not loop.
    let l = labels(
        "def f(n):\n    return f(n)\ndef a():\n    return b()\ndef b():\n    return a()\nx = f(1)\ny = a()\n",
    );
    assert!(
        !l.iter().any(|l| l.starts_with(": ") || l.starts_with("->")),
        "{l:?}"
    );
}

#[test]
fn inference_uses_one_branch_of_a_recursive_function_that_agrees() {
    // `fact` returns Int from its base case; the recursive branch is
    // `n * fact(n - 1)`, which can't be inferred without assuming the
    // answer, so the whole function stays unknown — conservative.
    let l = labels(
        "def fact(n: Int):\n    if n < 2:\n        return 1\n    return n * fact(n - 1)\nv = fact(5)\n",
    );
    assert!(!l.iter().any(|l| l == ": Int" || l == "-> Int"), "{l:?}");
}
