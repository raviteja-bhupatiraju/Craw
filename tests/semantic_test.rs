use craw::semantic::{RefKind, Resolution, ScopeKind, SemanticModel, SymbolKind};

fn model(src: &str) -> SemanticModel {
    SemanticModel::build(src)
}

#[test]
fn function_definition_has_exact_name_span() {
    let m = model("def add(a, b) = a + b\n");
    let f = m.symbol_named("add").expect("add defined");
    assert_eq!(f.kind, SymbolKind::Function);
    assert_eq!(&m.text()[f.name_span.start..f.name_span.end], "add");
    assert_eq!(f.name_span.start, 4);
}

#[test]
fn definition_is_not_confused_by_same_name_in_a_string() {
    // The old analyzer found `b` inside the string literal on line 0.
    let m = model("a = \"b\"\nb = 2\n");
    let b = m.symbol_named("b").unwrap();
    assert_eq!(m.line_index().position(b.name_span.start).line, 1);
}

#[test]
fn all_definition_kinds_are_collected() {
    let src = "data P(x, y)\nstruct S:\n    v: Int\ntrait T:\n    def m(self) = 1\nclass C(n):\n    def get(self) = self.n\ntemplate tp(body):\n    body\nmacro mc(x):\n    x\ngen g():\n    yield 1\nk = 1\n";
    let m = model(src);
    let kinds: Vec<_> = ["P", "S", "T", "C", "tp", "mc", "g", "k"]
        .iter()
        .map(|n| m.symbol_named(n).unwrap_or_else(|| panic!("{n}")).kind)
        .collect();
    use SymbolKind::*;
    assert_eq!(
        kinds,
        vec![
            DataClass, Struct, Trait, Class, Template, Macro, Generator, Variable
        ]
    );
    // fields and methods are children
    let p = m.symbol_named("P").unwrap();
    assert_eq!(
        m.children(p.id)
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["x", "y"]
    );
    let c = m.symbol_named("C").unwrap();
    assert!(
        m.children(c.id)
            .iter()
            .any(|s| s.name == "get" && s.kind == SymbolKind::Method)
    );
}

#[test]
fn full_span_covers_the_whole_block() {
    let src = "def f():\n    a = 1\n    return a\nx = 2\n";
    let m = model(src);
    let f = m.symbol_named("f").unwrap();
    let end_line = m.line_index().position(f.full_span.end).line;
    assert_eq!(end_line, 2, "def body ends on line 2, x is on line 3");
}

#[test]
fn parameters_and_locals_bind_in_function_scope() {
    let m = model("def f(a, b=2, *rest):\n    c = a\n    return c\nd = 1\n");
    let f = m.symbol_named("f").unwrap();
    let scope = m.scope_of(f.id).unwrap(); // the scope the def introduces
    let mut names: Vec<_> = m.scopes()[scope.0 as usize]
        .bindings
        .keys()
        .cloned()
        .collect();
    names.sort();
    assert_eq!(names, vec!["a", "b", "c", "rest"]);
    assert!(m.symbol_named("d").unwrap().scope == m.module_scope());
    assert_eq!(m.lookup("c", m.module_scope()), None, "c is local to f");
}

#[test]
fn pattern_binders_are_collected() {
    let src = "(a, b) = (1, 2)\n[h, *t] = [1, 2]\nfor (i, j) in pairs:\n    pass_it(i)\nmatch v:\n    case P(x, y) if x > y:\n        y\n    case n:\n        n\n";
    let m = model(src);
    for n in ["a", "b", "h", "t", "i", "j", "x", "y", "n"] {
        assert!(m.symbol_named(n).is_some(), "{n} should be bound");
    }
}

#[test]
fn match_arm_patterns_resolve_to_their_own_case_line() {
    let m = model("match v:\n    case x:\n        print(x)\n    case x:\n        print(x)\n");
    let xs: Vec<_> = m
        .symbols()
        .iter()
        .filter(|s| s.name == "x")
        .map(|s| m.line_index().position(s.name_span.start).line)
        .collect();
    assert_eq!(xs, vec![1, 3]);
}

#[test]
fn lambda_comprehension_where_query_scopes() {
    let src = "f = x => x + 1\nsq = [n * n for n in range(3)]\nr = g(a) where:\n    a = 1\nq = from p in ps where p > 1 select p\n";
    let m = model(src);
    for (n, kind) in [
        ("x", ScopeKind::Lambda),
        ("n", ScopeKind::Comprehension),
        ("a", ScopeKind::Where),
        ("p", ScopeKind::Query),
    ] {
        let s = m.symbol_named(n).unwrap_or_else(|| panic!("{n}"));
        assert_eq!(m.scopes()[s.scope.0 as usize].kind, kind, "{n}");
    }
}

#[test]
fn global_nonlocal_imports_and_class_self() {
    let src = "from std.math import sqrt\ncounter = 0\ndef inc():\n    global counter\n    counter = counter + 1\nclass C(n):\n    def get(self) = self.n\n";
    let m = model(src);
    assert_eq!(m.symbol_named("sqrt").unwrap().kind, SymbolKind::Import);
    let inc = m.symbol_named("inc").unwrap();
    let inc_scope = m.scope_of(inc.id).unwrap();
    // `global counter` must NOT create a local `counter`; lookup resolves to the module symbol
    assert_eq!(
        m.lookup("counter", inc_scope),
        Some(m.symbol_named("counter").unwrap().id)
    );
    let get = m.symbols().iter().find(|s| s.name == "get").unwrap();
    assert!(m.lookup("self", m.scope_of(get.id).unwrap()).is_some());
}

#[test]
fn references_resolve_to_innermost_binding() {
    let src = "x = 1\ndef f(x):\n    return x\nprint(x)\n";
    let m = model(src);
    let refs: Vec<_> = m
        .references_named("x")
        .iter()
        .map(|r| (m.line_index().position(r.span.start).line, r.target.clone()))
        .collect();
    let outer = m.symbol_named("x").unwrap().id;
    let inner = m
        .symbols()
        .iter()
        .find(|s| s.name == "x" && s.kind == SymbolKind::Parameter)
        .unwrap()
        .id;
    assert_eq!(
        refs,
        vec![
            (0, Resolution::Symbol(outer)),
            (1, Resolution::Symbol(inner)),
            (2, Resolution::Symbol(inner)),
            (3, Resolution::Symbol(outer)),
        ]
    );
    assert_eq!(m.references_to(outer).len(), 2);
}

#[test]
fn attribute_names_are_field_references_not_variables() {
    let m = model("data P(x)\np = P(1)\nprint(p.x)\n");
    let last_x = m.references_named("x").last().unwrap().target.clone();
    assert!(matches!(last_x, Resolution::Field(_)));
    let field = m.children(m.symbol_named("P").unwrap().id)[0].id;
    assert_eq!(last_x, Resolution::Field(vec![field]));
}

#[test]
fn builtins_and_types_resolve() {
    let m = model("def f(a) -> Int = len(a)\n");
    assert!(matches!(
        m.references_named("len")[0].target,
        Resolution::Builtin(_)
    ));
    assert!(matches!(
        m.references_named("Int")[0].target,
        Resolution::BuiltinType(_)
    ));
}

#[test]
fn unresolved_names_are_flagged_but_nothing_in_rust_regions_is() {
    let src = "y = zzz\nrust:\n    let qqq = 1;\nfn helper(v: i64) -> i64 { v }\n";
    let m = model(src);
    assert!(matches!(
        m.references_named("zzz")[0].target,
        Resolution::Unresolved
    ));
    assert!(m.references_named("qqq").is_empty());
    assert!(m.references_named("v").is_empty());
}

#[test]
fn references_inside_fstrings_resolve() {
    let m = model("name = \"a\"\ns = f\"hi {name}\"\n");
    let n = m.symbol_named("name").unwrap().id;
    assert_eq!(m.references_to(n).len(), 2);
}

#[test]
fn definition_site_is_a_write_reference() {
    let m = model("x = 1\nx = x + 1\n");
    let x = m.symbol_named("x").unwrap().id;
    let kinds: Vec<_> = m.references_to(x).iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        vec![RefKind::Definition, RefKind::Write, RefKind::Read]
    );
}
