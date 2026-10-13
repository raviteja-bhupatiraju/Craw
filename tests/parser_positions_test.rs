use craw::ast::Stmt;
use craw::lexer::Lexer;
use craw::parser;

fn parse(src: &str) -> Vec<Stmt> {
    parser::parse(&Lexer::new(src).tokenize()).unwrap()
}

#[test]
fn assign_records_start_token() {
    let ast = parse("x = 1\ny = 2\n");
    let Stmt::Assign(_, _, pos0) = &ast[0] else {
        panic!("expected Assign, got {:?}", ast[0])
    };
    let Stmt::Assign(_, _, pos1) = &ast[1] else {
        panic!("expected Assign, got {:?}", ast[1])
    };
    assert_eq!(*pos0, 0);
    assert_eq!(*pos1, 4); // Ident x, Assign, Number, Newline, Ident y
}

#[test]
fn nested_statements_record_positions() {
    let src = "def f(a):\n    b = a\n    for i in b:\n        global g\n        nonlocal n\n    match a:\n        case 1:\n            c = 2\n";
    let ast = parse(src);
    let Stmt::FunctionDef { body, id, .. } = &ast[0] else {
        panic!("expected FunctionDef, got {:?}", ast[0])
    };
    assert_eq!(*id, 0);
    let Stmt::Assign(_, _, p_b) = &body[0] else {
        panic!("expected Assign, got {:?}", body[0])
    };
    let Stmt::MatchFor(_, _, for_body, p_for) = &body[1] else {
        panic!("expected MatchFor, got {:?}", body[1])
    };
    let Stmt::Global(_, p_g) = &for_body[0] else {
        panic!("expected Global, got {:?}", for_body[0])
    };
    let Stmt::Nonlocal(_, p_n) = &for_body[1] else {
        panic!("expected Nonlocal, got {:?}", for_body[1])
    };
    let Stmt::Match(_, _, p_m) = &body[2] else {
        panic!("expected Match, got {:?}", body[2])
    };
    assert!(*p_b < *p_for && *p_for < *p_g && *p_g < *p_n && *p_n < *p_m);
}

#[test]
fn import_use_passthrough_macrodef_positions() {
    let src = "from std.math import sqrt\nuse std::thread\nmacro m(x):\n    x\n";
    let ast = parse(src);
    assert!(matches!(ast[0], Stmt::FromImport(_, _, 0)));
    // A column-0 `use …` line is captured by the lexer as inline Rust
    // (`Passthrough`), so accept either form here.
    assert!(matches!(ast[1], Stmt::Use(_, p) | Stmt::Passthrough(_, p) if p > 0));
    assert!(matches!(ast[2], Stmt::MacroDef { pos, .. } if pos > 0));
}
