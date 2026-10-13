use craw::semantic::SemanticModel;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString};

fn code_str(d: &Diagnostic) -> String {
    match d.code.as_ref().unwrap() {
        NumberOrString::Number(n) => n.to_string(),
        NumberOrString::String(s) => s.clone(),
    }
}

fn codes(src: &str) -> Vec<(String, u32)> {
    let m = SemanticModel::build(src);
    m.semantic_diagnostics()
        .iter()
        .filter(|d| d.source.as_deref() == Some("craw"))
        .map(|d| (code_str(d), d.range.start.line))
        .collect()
}

#[test]
fn unresolved_name() {
    assert_eq!(codes("x = y\n"), vec![("E0100".into(), 0)]);
}

#[test]
fn unused_local_is_hint() {
    let m = SemanticModel::build("def f():\n    unused = 1\n    return 2\n");
    let d = &m.semantic_diagnostics()[0];
    assert_eq!(code_str(d), "W0200");
    assert_eq!(d.severity, Some(DiagnosticSeverity::HINT));
    assert_eq!(d.tags, Some(vec![DiagnosticTag::UNNECESSARY]));
}

#[test]
fn underscore_prefixed_is_not_unused() {
    assert!(codes("def f(_a):\n    return 1\n").is_empty());
}

#[test]
fn duplicate_def_in_scope() {
    assert_eq!(
        codes("def f() = 1\ndef f() = 2\n"),
        vec![("E0101".into(), 1)]
    );
}

#[test]
fn addpattern_is_not_duplicate() {
    assert!(codes("def f(0) = 1\naddpattern def f(n) = n\n").is_empty());
}

#[test]
fn arity_mismatch_for_known_def() {
    assert_eq!(
        codes("def add(a, b) = a + b\nadd(1)\nadd(1, 2, 3)\n"),
        vec![("E0102".into(), 1), ("E0102".into(), 2)]
    );
    assert!(codes("def f(a, b=1) = a + b\nf(1)\nf(1, 2)\n").is_empty());
    assert!(codes("def f(*xs) = xs\nf()\nf(1, 2, 3)\n").is_empty());
}

#[test]
fn misplaced_keywords() {
    assert_eq!(codes("def f():\n    yield 1\n"), vec![]); // parser marks it a generator; fine
    assert_eq!(codes("return 1\n"), vec![("E0103".into(), 0)]);
    assert_eq!(codes("break\n"), vec![("E0104".into(), 0)]);
    assert_eq!(codes("nonlocal q\n"), vec![("E0105".into(), 0)]);
}

#[test]
fn rust_interop_info() {
    let m = SemanticModel::build("rust:\n    let x = 1;\n");
    assert!(
        m.semantic_diagnostics()
            .iter()
            .any(|d| code_str(d) == "I0300" && d.severity == Some(DiagnosticSeverity::INFORMATION))
    );
}

#[test]
fn nothing_reported_inside_blanked_regions() {
    assert_eq!(
        codes("x = \ny = 1\n")
            .iter()
            .filter(|(c, _)| c == "E0100")
            .count(),
        0
    );
}

// THE GATE: no false positives on real programs.
#[test]
fn samples_produce_no_errors_or_warnings() {
    for path in [
        "samples/showcase.craw",
        "samples/master_showcase.craw",
        "samples/macro_blocks.craw",
        "tests/feature_test.craw",
        "tests/parity_dict.craw",
        "tests/parity_list.craw",
        "tests/parity_string.craw",
    ] {
        let src = std::fs::read_to_string(path).unwrap();
        let m = SemanticModel::build(&src);
        let bad: Vec<_> = m
            .semantic_diagnostics()
            .iter()
            .filter(|d| {
                matches!(
                    d.severity,
                    Some(DiagnosticSeverity::ERROR | DiagnosticSeverity::WARNING)
                )
            })
            .map(|d| {
                format!(
                    "{}:{}:{} {} {}",
                    path,
                    d.range.start.line + 1,
                    d.range.start.character,
                    code_str(d),
                    d.message
                )
            })
            .collect();
        assert!(bad.is_empty(), "false positives:\n{}", bad.join("\n"));
    }
}

#[test]
fn keyword_argument_name_is_not_unresolved() {
    assert!(codes("def f(a, b) = a + b\nf(1, b=2)\n").is_empty());
}

#[test]
fn keyword_argument_shorthand_still_resolves_its_value() {
    assert_eq!(
        codes("def f(a, b) = a + b\nf(1, b=)\n"),
        vec![("E0100".into(), 1)]
    );
}
