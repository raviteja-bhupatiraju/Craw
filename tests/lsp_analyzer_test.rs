use craw::lsp_analyzer::{LspAnalyzer, ServerConfig};
use tower_lsp::lsp_types::{Diagnostic, HoverContents, NumberOrString, Position, Range};

fn code(d: &Diagnostic) -> String {
    match d.code.as_ref().unwrap() {
        NumberOrString::Number(n) => n.to_string(),
        NumberOrString::String(s) => s.clone(),
    }
}

#[test]
fn test_lsp_analyzer_init() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "let a = 1;");
    assert!(analyzer.get_document("file:///main.crw").is_some());
}

#[test]
fn test_analyzer_diagnostics() {
    let mut analyzer = LspAnalyzer::new();
    // Use a def block so `let` is not captured as top-level rust passthrough
    let diags = analyzer.update_document("file:///main.crw", "def test():\n    let a = ;\n");
    assert!(!diags.is_empty());
}

#[test]
fn test_autocomplete_variables() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "let my_var = 1;\nm");

    // Request completion at line 1, char 1 (after 'm')
    let completions = analyzer.get_completions("file:///main.crw", 1, 1);
    assert!(completions.iter().any(|c| c.label == "my_var"));
}

#[test]
fn test_autocomplete_scope() {
    let mut analyzer = LspAnalyzer::new();
    let text = "outer_var = 2;\ndef foo():\n    inner_var = 1\n    \n\nanother_outer = 3;";
    analyzer.update_document("file:///main.crw", text);

    // In foo's body (line 2), should see inner_var and outer_var
    let comps_inner = analyzer.get_completions("file:///main.crw", 2, 17);
    assert!(comps_inner.iter().any(|c| c.label == "inner_var"));
    assert!(comps_inner.iter().any(|c| c.label == "outer_var"));

    // Outside foo (line 5), should NOT see inner_var
    let comps_outer = analyzer.get_completions("file:///main.crw", 5, 0);
    assert!(
        !comps_outer.iter().any(|c| c.label == "inner_var"),
        "should not see inner_var from outside"
    );
    assert!(comps_outer.iter().any(|c| c.label == "outer_var"));
}

#[test]
fn test_goto_definition() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "target = 42;\ntarget");
    let loc = analyzer.get_definition("file:///main.crw", 1, 3).unwrap();
    assert_eq!(loc.range.start.line, 0); // defined on line 0
}

#[test]
fn test_hover_info() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "target = 42;\ntarget");
    let hover = analyzer.get_hover_info("file:///main.crw", 1, 3).unwrap();

    if let HoverContents::Markup(m) = hover.contents {
        assert_eq!(
            m.value,
            "```craw\ntarget: Int\n```\n\n(variable, module scope)"
        );
    } else {
        panic!("Expected HoverContents::Markup");
    }
}

#[test]
fn test_find_references() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "x = 1;\ny = x + x;");
    let refs = analyzer.find_references("file:///main.crw", 0, 0).unwrap(); // cursor on 'x'
    assert_eq!(refs.len(), 3); // 1 definition, 2 usages
}

#[test]
fn test_rename_symbol() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "x = 1;\ny = x + x;");
    let edit = analyzer
        .rename_symbol("file:///main.crw", 0, 0, "z".to_string())
        .unwrap()
        .unwrap();

    // We expect the workspace edit to contain changes for file:///main.crw
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::parse("file:///main.crw").unwrap();
    let edits = changes.get(&uri).unwrap();
    assert_eq!(edits.len(), 3); // 1 definition, 2 usages
}

#[test]
fn test_find_references_safe() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document(
        "file:///main.crw",
        r#"def test():
    x = 1
    # this is x
    y = "x"
    z = 'x'
    foo.x
    y = x + 1
"#,
    );
    // x is defined on line 1, col 8 (utf-16 col)
    let refs = analyzer.find_references("file:///main.crw", 1, 4).unwrap();

    // Should find the definition (line 1) and the assignment (line 6)
    assert_eq!(refs.len(), 2, "refs: {:?}", refs);
}

#[test]
fn test_document_symbols() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "def main():\n    x = 1");
    let symbols = analyzer.get_document_symbols("file:///main.crw").unwrap();
    assert_eq!(symbols[0].name, "main");
    assert_eq!(symbols[0].kind, tower_lsp::lsp_types::SymbolKind::FUNCTION);
}

/// A name's own definition site is located by walking the AST and the
/// per-token span table, not by searching the raw document text — so a
/// name that happens to appear earlier inside a string or comment can
/// never be mistaken for the real definition (the old text-search-based
/// analyzer had exactly this bug, previously locked in by a test named
/// `test_scope_builder_bug`).
#[test]
fn definition_ignores_strings_and_comments() {
    let mut a = LspAnalyzer::new();
    a.update_document(
        "file:///m.craw",
        "# b is documented here\na = \"b\"\nb = 2\nprint(b)\n",
    );
    let loc = a.get_definition("file:///m.craw", 3, 6).unwrap();
    assert_eq!(loc.range.start.line, 2);
}

#[test]
fn diagnostics_have_a_precise_column_not_just_a_line() {
    let mut a = LspAnalyzer::new();
    let d = a.update_document("file:///m.craw", "def t():\n    x = \n");
    assert_eq!((d[0].range.start.line, d[0].range.start.character), (1, 8));
}

/// A document that fails to parse must never leave the *previous*
/// version's positions in effect: update_document always replaces the
/// stored model outright (recovery guarantees some AST survives even a
/// broken parse), so a lookup always resolves against the text that is
/// actually there right now.
#[test]
fn stale_state_is_never_served() {
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", "abc = 1\nprint(abc)\n");
    a.update_document("file:///m.craw", "zz = \nabc = 1\nprint(abc)\n"); // line 0 broken (missing RHS)
    let loc = a.get_definition("file:///m.craw", 2, 7).unwrap();
    assert_eq!(loc.range.start.line, 1, "must resolve against the new text");
}

#[test]
fn incremental_edit_applies_utf16_ranges() {
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", "a≈b = 1\n");
    a.apply_change(
        "file:///m.craw",
        Some(Range::new(Position::new(0, 2), Position::new(0, 3))),
        "c",
    );
    assert_eq!(
        a.get_document("file:///m.craw").unwrap().model.text(),
        "a≈c = 1\n"
    );
}

#[test]
fn incremental_edit_with_no_range_replaces_the_whole_document() {
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", "x = 1\n");
    a.apply_change("file:///m.craw", None, "y = 2\n");
    assert_eq!(
        a.get_document("file:///m.craw").unwrap().model.text(),
        "y = 2\n"
    );
}

#[test]
fn config_defaults_to_reporting_everything() {
    let mut a = LspAnalyzer::new();
    let diags = a.update_document("file:///m.craw", "x = y\n");
    assert!(diags.iter().any(|d| code(d) == "E0100"));
}

#[test]
fn disabling_unresolved_suppresses_e0100_but_not_other_codes() {
    let mut a = LspAnalyzer::new();
    a.set_config(ServerConfig {
        unresolved: false,
        unused: true,
        ..Default::default()
    });
    let diags = a.update_document("file:///m.craw", "x = y\ndef f() = 1\ndef f() = 2\n");
    assert!(!diags.iter().any(|d| code(d) == "E0100"));
    assert!(diags.iter().any(|d| code(d) == "E0101"));
}

#[test]
fn disabling_unused_suppresses_w0200() {
    let mut a = LspAnalyzer::new();
    a.set_config(ServerConfig {
        unresolved: true,
        unused: false,
        ..Default::default()
    });
    let diags = a.update_document("file:///m.craw", "def f():\n    unused = 1\n    return 2\n");
    assert!(!diags.iter().any(|d| code(d) == "W0200"));
}

#[test]
fn diagnostics_for_recomputes_without_reparsing() {
    let mut a = LspAnalyzer::new();
    a.update_document("file:///m.craw", "x = y\n");
    a.set_config(ServerConfig {
        unresolved: false,
        unused: true,
        ..Default::default()
    });
    let diags = a.diagnostics_for("file:///m.craw").unwrap();
    assert!(!diags.iter().any(|d| code(d) == "E0100"));
}
