use craw::lsp_analyzer::LspAnalyzer;
use tower_lsp::lsp_types::{HoverContents, MarkedString};

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

    if let HoverContents::Scalar(MarkedString::LanguageString(s)) = hover.contents {
        assert_eq!(s.language, "craw");
        assert_eq!(s.value, "(variable) target");
    } else {
        panic!("Expected HoverContents::Scalar(MarkedString::LanguageString)");
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
        .unwrap();

    // We expect the workspace edit to contain changes for file:///main.crw
    let changes = edit.changes.unwrap();
    let uri = tower_lsp::lsp_types::Url::parse("file:///main.crw").unwrap();
    let edits = changes.get(&uri).unwrap();
    assert_eq!(edits.len(), 3); // 1 definition, 2 usages
}

#[test]
fn test_find_references_bug() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "x = 1;\n# this is x\ny = \"x\";");
    let refs = analyzer.find_references("file:///main.crw", 0, 0).unwrap();
    assert_eq!(refs.len(), 1); // Only the definition should be found!
}

#[test]
fn test_find_references_bug2() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "x = 1;\n# this is x\ny = \"x\";");
    let refs = analyzer.find_references("file:///main.crw", 0, 0).unwrap();
    assert_eq!(refs.len(), 1);
}

#[test]
fn test_find_references_bug3() {
    let mut analyzer = LspAnalyzer::new();
    let diags = analyzer.update_document("file:///main.crw", "x = 1;\n# this is x\ny = \"x\";");
    assert!(diags.is_empty());
}

#[test]
fn test_find_references_bug4() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "x = 1;\ny = \"x\";");
    let refs = analyzer.find_references("file:///main.crw", 0, 0).unwrap();
    assert_eq!(refs.len(), 1);
}

#[test]
fn test_find_references_bug5() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "x = 1;\ny = user.x;");
    let refs = analyzer.find_references("file:///main.crw", 0, 0).unwrap();
    assert_eq!(refs.len(), 1);
}

#[test]
fn test_scope_builder_bug() {
    let mut analyzer = LspAnalyzer::new();
    analyzer.update_document("file:///main.crw", "let a = \"b\";\nlet b = 2;");
    let loc = analyzer.get_definition("file:///main.crw", 1, 4).unwrap();
    // BUG: ScopeBuilder currently finds 'b' inside the string on line 0
    assert_eq!(loc.range.start.line, 0);
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
