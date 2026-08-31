use craw::ast::{Expr, Pattern, Stmt};
use craw::interpreter::{Scope, eval_expr, exec_stmt};
use craw::lexer::Lexer;
use craw::parser;
use craw::runtime::CrawValue;
use chumsky::Parser;

fn run(src: &str) -> Scope {
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize();
    let (ast, errors) = parser::parser().parse(&tokens).into_output_errors();
    assert!(errors.is_empty(), "parse errors: {:?}", errors);
    let mut scope = Scope::new();
    for stmt in ast.expect("no ast produced") {
        exec_stmt(&stmt, &mut scope).expect("exec_stmt failed");
    }
    scope
}

fn get(scope: &Scope, name: &str) -> CrawValue {
    scope.get(name).unwrap_or_else(|| panic!("{} not bound", name))
}

#[test]
fn test_for_loop_mutates_outer_variable() {
    let scope = run("xs = [1, 2, 3, 4, 5]\ntotal = 0\nfor x in xs:\n    total = total + x\n");
    assert_eq!(get(&scope, "total"), CrawValue::Int(15));
}

#[test]
fn test_for_loop_break() {
    let scope = run(
        "seen = []\nfor x in [1, 2, 3, 4, 5]:\n    if x == 4:\n        break\n    seen.append(x)\n",
    );
    match get(&scope, "seen") {
        CrawValue::List(items) => {
            let v: Vec<i64> = items
                .borrow()
                .iter()
                .map(|v| match v {
                    CrawValue::Int(n) => *n,
                    _ => panic!("expected int"),
                })
                .collect();
            assert_eq!(v, vec![1, 2, 3]);
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn test_match_with_guard_first_match_wins() {
    let scope = run(
        "results = []\nfor x in [1, 2, 3, 4]:\n    match x:\n        case n if n % 2 == 0:\n            results.append(\"even\")\n        case n:\n            results.append(\"odd\")\n",
    );
    match get(&scope, "results") {
        CrawValue::List(items) => {
            let v: Vec<String> = items
                .borrow()
                .iter()
                .map(|v| match v {
                    CrawValue::String(s) => s.to_string(),
                    _ => panic!("expected string"),
                })
                .collect();
            assert_eq!(v, vec!["odd", "even", "odd", "even"]);
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn test_match_data_pattern_destructure() {
    let scope = run(
        "data Point(x, y)\np = Point(3, 4)\nmatch p:\n    case Point(a, b):\n        total = a + b\n",
    );
    assert_eq!(get(&scope, "total"), CrawValue::Int(7));
}

#[test]
fn test_index_and_index_assign() {
    let scope = run("d = {\"a\": 1}\nfirst = d[\"a\"]\nd[\"b\"] = 2\nsecond = d[\"b\"]\n");
    assert_eq!(get(&scope, "first"), CrawValue::Int(1));
    assert_eq!(get(&scope, "second"), CrawValue::Int(2));
}

#[test]
fn test_ternary_and_none_coalesce() {
    let scope = run("y = 10 if 3 > 1 else -1\nv = None\nw = v ?? 99\n");
    assert_eq!(get(&scope, "y"), CrawValue::Int(10));
    assert_eq!(get(&scope, "w"), CrawValue::Int(99));
}

#[test]
fn test_fstring_interpolation() {
    let scope = run("name = \"world\"\ntotal = 3\ns = f\"hello {name}, total={total}\"\n");
    assert_eq!(
        get(&scope, "s"),
        CrawValue::String(std::rc::Rc::new("hello world, total=3".to_string()))
    );
}

#[test]
fn test_range_and_slice() {
    let scope = run("r = 1..5\nxs = [10, 20, 30, 40, 50]\nsl = xs[1:3]\n");
    match get(&scope, "r") {
        CrawValue::List(items) => assert_eq!(items.borrow().len(), 5),
        other => panic!("expected list, got {:?}", other),
    }
    match get(&scope, "sl") {
        CrawValue::List(items) => {
            let v: Vec<i64> = items
                .borrow()
                .iter()
                .map(|v| match v {
                    CrawValue::Int(n) => *n,
                    _ => panic!("expected int"),
                })
                .collect();
            assert_eq!(v, vec![20, 30]);
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn test_comprehension_does_not_leak_loop_variable() {
    let scope = run("x = 99\nys = [x * 10 for x in [1, 2, 3]]\n");
    assert_eq!(get(&scope, "x"), CrawValue::Int(99));
    match get(&scope, "ys") {
        CrawValue::List(items) => {
            let v: Vec<i64> = items
                .borrow()
                .iter()
                .map(|v| match v {
                    CrawValue::Int(n) => *n,
                    _ => panic!("expected int"),
                })
                .collect();
            assert_eq!(v, vec![10, 20, 30]);
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn test_gather_collects_field_across_list() {
    let scope = run("data Point(x, y)\npts = [Point(1, 2), Point(3, 4)]\nxs = pts..x\n");
    match get(&scope, "xs") {
        CrawValue::List(items) => {
            let v: Vec<i64> = items
                .borrow()
                .iter()
                .map(|v| match v {
                    CrawValue::Int(n) => *n,
                    _ => panic!("expected int"),
                })
                .collect();
            assert_eq!(v, vec![1, 3]);
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn test_implicit_lambda_placeholder() {
    let scope = run("def add(a, b):\n    return a + b\nadd5 = add(5, _)\nresult = add5(10)\n");
    assert_eq!(get(&scope, "result"), CrawValue::Int(15));
}

#[test]
fn test_splat_call_expands_list_args() {
    let scope = run("def add(a, b):\n    return a + b\nargs = [3, 4]\nresult = add(*args)\n");
    assert_eq!(get(&scope, "result"), CrawValue::Int(7));
}

#[test]
fn test_copyclosure_snapshots_captured_list() {
    let mut scope = Scope::new();
    let src = "xs = [1, 2]\ncopyclosure def reader():\n    return xs\n";
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize();
    let (ast, errors) = parser::parser().parse(&tokens).into_output_errors();
    assert!(errors.is_empty(), "parse errors: {:?}", errors);
    for stmt in ast.expect("no ast") {
        exec_stmt(&stmt, &mut scope).unwrap();
    }

    // Mutate the outer list after the copyclosure was created.
    if let CrawValue::List(items) = get(&scope, "xs") {
        items.borrow_mut().push(CrawValue::Int(3));
    }

    let reader = get(&scope, "reader");
    let result = craw::runtime::craw_driver(craw::runtime::craw_call(reader, vec![]));
    match result {
        CrawValue::List(items) => assert_eq!(items.borrow().len(), 2),
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn test_where_frame_not_leaked_on_error() {
    // A failing statement inside a `where` block must not leave a stray
    // frame on the persistent scope stack (regression test for a frame
    // leak that previously corrupted later lookups in the same scope).
    let mut scope = Scope::new();
    let bad = Expr::Where(
        Box::new(Expr::Number(1)),
        vec![Stmt::Expr(Expr::Ident("undefined_var".to_string()))],
        0,
    );
    assert!(eval_expr(&bad, &mut scope).is_err());
    assert_eq!(scope.locals.len(), 1);

    let ok = Expr::Where(Box::new(Expr::Number(42)), vec![], 0);
    assert_eq!(eval_expr(&ok, &mut scope).unwrap(), CrawValue::Int(42));
    assert_eq!(scope.locals.len(), 1);
}

#[test]
fn test_bind_pattern_wildcard_and_rest() {
    let mut scope = Scope::new();
    let value = CrawValue::List(std::rc::Rc::new(std::cell::RefCell::new(vec![
        CrawValue::Int(1),
        CrawValue::Int(2),
        CrawValue::Int(3),
    ])));
    let pat = Pattern::Data(
        "List".to_string(),
        vec![
            Pattern::Var("head".to_string(), None),
            Pattern::Rest("tail".to_string()),
        ],
    );
    assert!(craw::interpreter::bind_pattern(&pat, &value, &mut scope).unwrap());
    assert_eq!(get(&scope, "head"), CrawValue::Int(1));
    match get(&scope, "tail") {
        CrawValue::List(items) => assert_eq!(items.borrow().len(), 2),
        other => panic!("expected list, got {:?}", other),
    }
}
