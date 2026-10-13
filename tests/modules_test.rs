//! The module system (`craw::modules`): Python's import semantics — module
//! and package resolution, relative imports, `import a.b` binding `a`,
//! `from m import *`, per-module namespaces including types — and the CLI
//! and interpreter running multi-file programs end to end.

use craw::modules::{self, FsLoader, LinkError};
use std::path::Path;
use std::process::Command;

/// A temporary project: `files` are `(relative path, contents)`.
fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

fn link(dir: &Path, entry: &str) -> Result<modules::Linked, Vec<LinkError>> {
    modules::link(&dir.join(entry), &[], &FsLoader::default())
}

fn source(dir: &Path, entry: &str) -> String {
    match link(dir, entry) {
        Ok(l) => l.source,
        Err(e) => panic!("link failed: {e:#?}"),
    }
}

fn errors(dir: &Path, entry: &str) -> Vec<String> {
    link(dir, entry)
        .expect_err("expected link errors")
        .into_iter()
        .map(|e| {
            let rel = e
                .file
                .strip_prefix(modules::normalize(dir))
                .unwrap_or(&e.file)
                .to_string_lossy()
                .replace('\\', "/");
            format!("{rel}:{}:{}: {}", e.line + 1, e.character + 1, e.message)
        })
        .collect()
}

/// Runs a program in the interpreter (`crawler`), returning stdout.
fn interpret(dir: &Path, entry: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_crawler"))
        .arg(dir.join(entry))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "crawler failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Compiles and runs a program with `craw run`, returning stdout.
fn compile_and_run(dir: &Path, entry: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_craw"))
        .args(["run", entry])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const GEOMETRY: &str = "\
data Point(x, y)
scale = 2
def helper(v):
    return v * scale
def area(w, h):
    return helper(w) * h
print(\"geometry loaded\")
";

// ---------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------

#[test]
fn absolute_imports_search_the_roots_only() {
    let dir = project(&[
        ("lib.craw", ""),
        ("pkg/util.craw", ""),
        ("pkg/inner.craw", ""),
    ]);
    let loader = FsLoader::default();
    let root = vec![modules::normalize(dir.path())];
    let inner = dir.path().join("pkg/inner.craw");
    // Python 3: `import util` from pkg/inner.craw does NOT find pkg/util.craw.
    assert!(modules::resolve_module(&["util".into()], &inner, &root, &loader).is_none());
    assert!(modules::resolve_module(&["lib".into()], &inner, &root, &loader).is_some());
    let hit =
        modules::resolve_module(&["pkg".into(), "util".into()], &inner, &root, &loader).unwrap();
    assert!(hit.ends_with("pkg/util.craw"));
}

#[test]
fn relative_imports_resolve_from_the_importers_package() {
    let dir = project(&[
        ("pkg/__init__.craw", ""),
        ("pkg/util.craw", ""),
        ("pkg/sub/deep.craw", ""),
        ("top.craw", ""),
    ]);
    let loader = FsLoader::default();
    let deep = dir.path().join("pkg/sub/deep.craw");
    let r = |p: &[&str]| {
        let path: Vec<String> = p.iter().map(|s| s.to_string()).collect();
        modules::resolve_module(&path, &deep, &[], &loader)
    };
    assert!(r(&[".", ".", "util"]).unwrap().ends_with("pkg/util.craw"));
    assert!(r(&[".", "."]).unwrap().ends_with("pkg/__init__.craw"));
    assert!(r(&[".", ".", ".", "top"]).unwrap().ends_with("top.craw"));
    assert!(r(&[".", "nope"]).is_none());
}

#[test]
fn packages_resolve_to_init_even_without_one() {
    let dir = project(&[("with/__init__.craw", "x = 1\n"), ("bare/mod.craw", "")]);
    let loader = FsLoader::default();
    let root = vec![modules::normalize(dir.path())];
    let main = dir.path().join("main.craw");
    let with = modules::resolve_module(&["with".into()], &main, &root, &loader).unwrap();
    assert!(with.ends_with("with/__init__.craw"));
    let bare = modules::resolve_module(&["bare".into()], &main, &root, &loader).unwrap();
    assert!(bare.ends_with("bare/__init__.craw") && !bare.exists());
    assert_eq!(modules::module_source(&loader, &bare).as_deref(), Some(""));
}

#[test]
fn crawpath_directories_are_searched_after_the_entry_directory() {
    let lib = project(&[(
        "shared.craw",
        "def hello():\n    return \"from CRAWPATH\"\n",
    )]);
    let app = project(&[("main.craw", "from shared import hello\nprint(hello())\n")]);
    let out = Command::new(env!("CARGO_BIN_EXE_crawler"))
        .arg(app.path().join("main.craw"))
        .env("CRAWPATH", lib.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "from CRAWPATH\n");
}

#[test]
fn loader_overrides_supply_unsaved_text() {
    let dir = project(&[("lib.craw", "def f():\n    return 1\n")]);
    let mut loader = FsLoader::default();
    loader.overrides.insert(
        modules::normalize(&dir.path().join("lib.craw")),
        "def g():\n    return 2\n".into(),
    );
    std::fs::write(
        dir.path().join("main.craw"),
        "from lib import g\nprint(g())\n",
    )
    .unwrap();
    assert!(modules::link(&dir.path().join("main.craw"), &[], &loader).is_ok());
}

// ---------------------------------------------------------------------
// Import forms
// ---------------------------------------------------------------------

#[test]
fn import_a_b_binds_a_and_is_used_as_a_b_name() {
    let dir = project(&[
        ("shapes/circle.craw", "def origin():\n    return 0\n"),
        ("shapes/square.craw", "def side():\n    return 4\n"),
        (
            "main.craw",
            "import shapes.circle\nimport shapes.square\nprint(shapes.circle.origin(), shapes.square.side())\n",
        ),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "0 4\n");
    let src = source(dir.path(), "main.craw");
    assert!(
        src.contains("print(circle__origin(), square__side())"),
        "{src}"
    );
}

#[test]
fn import_as_binds_the_module_itself() {
    let dir = project(&[
        ("shapes/circle.craw", "def origin():\n    return 0\n"),
        (
            "main.craw",
            "import shapes.circle as c\nprint(c.origin())\n",
        ),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "0\n");
}

#[test]
fn several_modules_in_one_import_statement() {
    let dir = project(&[
        ("a.craw", "x = 1\n"),
        ("b.craw", "y = 2\n"),
        ("main.craw", "import a, b as bee\nprint(a.x + bee.y)\n"),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "3\n");
}

#[test]
fn parenthesized_from_imports_may_span_lines() {
    let dir = project(&[
        ("geometry.craw", GEOMETRY),
        (
            "main.craw",
            "from geometry import (\n    area,\n    helper as h,\n)\nprint(area(1, 2), h(3))\n",
        ),
    ]);
    let linked = link(dir.path(), "main.craw").unwrap();
    assert_eq!(interpret(dir.path(), "main.craw"), "geometry loaded\n4 6\n");
    // The import's four lines are still four lines.
    let main_start = linked.segments[1].start_line as usize;
    assert_eq!(
        linked.source.lines().nth(main_start + 4),
        Some("print(area(1, 2), h(3))")
    );
}

#[test]
fn from_package_import_submodule() {
    let dir = project(&[
        ("pkg/__init__.craw", "version = \"1.0\"\n"),
        ("pkg/tools.craw", "def hammer():\n    return \"bang\"\n"),
        (
            "main.craw",
            "from pkg import tools, version\nprint(tools.hammer(), version)\n",
        ),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "bang 1.0\n");
}

#[test]
fn relative_imports_work_inside_packages() {
    let dir = project(&[
        ("app/__init__.craw", "from .config import name\n"),
        ("app/config.craw", "name = \"craw-app\"\n"),
        ("app/core/__init__.craw", ""),
        (
            "app/core/engine.craw",
            "from ..config import name\nfrom . import helpers\ndef start():\n    return helpers.shout(name)\n",
        ),
        (
            "app/core/helpers.craw",
            "def shout(s):\n    return s + \"!\"\n",
        ),
        (
            "main.craw",
            "import app\nfrom app.core.engine import start\nprint(app.name, start())\n",
        ),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "craw-app craw-app!\n");
}

#[test]
fn star_imports_bind_public_names_or_dunder_all() {
    let dir = project(&[
        (
            "pub.craw",
            "def one():\n    return 1\ndef _hidden():\n    return 0\ndata Pair(a, b)\n",
        ),
        (
            "listed.craw",
            "__all__ = [\"only\"]\ndef only():\n    return \"only\"\ndef other():\n    return \"other\"\n",
        ),
        (
            "main.craw",
            "from pub import *\nfrom listed import *\nprint(one(), Pair(1, 2), only())\n",
        ),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "1 Pair(1, 2) only\n");
    let hidden = project(&[
        ("pub.craw", "def _hidden():\n    return 0\n"),
        ("main.craw", "from pub import *\nprint(_hidden())\n"),
    ]);
    // `_hidden` isn't imported, so the importer's use stays unresolved.
    assert!(source(hidden.path(), "main.craw").ends_with("\nprint(_hidden())\n"));
}

#[test]
fn reexports_follow_the_chain_for_functions_and_types() {
    let dir = project(&[
        (
            "base.craw",
            "data Point(x, y)\ndef make():\n    return Point(1, 2)\n",
        ),
        ("mid.craw", "from base import make, Point\n"),
        (
            "main.craw",
            "from mid import make, Point\nmatch make():\n    case Point(a, b):\n        print(a + b)\n",
        ),
    ]);
    let src = source(dir.path(), "main.craw");
    assert!(src.contains("make = base__make"), "{src}");
    assert!(src.contains("case base__Point(a, b):"), "{src}");
    assert_eq!(interpret(dir.path(), "main.craw"), "3\n");
}

#[test]
fn native_std_from_imports_still_pass_through() {
    let dir = project(&[("main.craw", "from std.collections import HashMap\nx = 1\n")]);
    let linked = link(dir.path(), "main.craw").unwrap();
    assert!(
        linked
            .source
            .starts_with("from std.collections import HashMap\n")
    );
    assert_eq!(linked.files.len(), 1);
}

// ---------------------------------------------------------------------
// Namespaces
// ---------------------------------------------------------------------

#[test]
fn private_names_are_prefixed_and_imports_become_bindings() {
    let dir = project(&[
        ("geometry.craw", GEOMETRY),
        (
            "main.craw",
            "from geometry import area\nprint(area(1, 2))\n",
        ),
    ]);
    let src = source(dir.path(), "main.craw");
    assert!(src.contains("def geometry__helper(v):"), "{src}");
    assert!(src.contains("return geometry__helper(w) * h"), "{src}");
    assert!(src.contains("return v * geometry__scale"), "{src}");
    assert!(src.contains("data geometry__Point(x, y)"), "{src}");
    assert!(
        src.contains("area = geometry__area\nprint(area(1, 2))"),
        "{src}"
    );
}

#[test]
fn each_module_has_its_own_types() {
    let dir = project(&[
        (
            "a.craw",
            "data Point(x)\ndef make():\n    return Point(1)\n",
        ),
        (
            "b.craw",
            "data Point(x)\ndef make():\n    return Point(2)\n",
        ),
        (
            "main.craw",
            "import a, b\ndata Point(x)\nfor p in [a.make(), b.make(), Point(3)]:\n    match p:\n        case a.Point(v):\n            print(\"a\", v)\n        case b.Point(v):\n            print(\"b\", v)\n        case Point(v):\n            print(\"main\", v)\n",
        ),
    ]);
    // Printed names drop the module prefix; matching keeps them apart.
    assert_eq!(interpret(dir.path(), "main.craw"), "a 1\nb 2\nmain 3\n");
    let dir2 = project(&[
        ("a.craw", "data Point(x)\n"),
        ("main.craw", "from a import Point\nprint(Point(7))\n"),
    ]);
    assert_eq!(interpret(dir2.path(), "main.craw"), "Point(7)\n");
}

#[test]
fn a_local_shadowing_a_module_binding_is_left_alone() {
    let dir = project(&[
        ("geometry.craw", GEOMETRY),
        (
            "main.craw",
            "import geometry\ndef f(geometry):\n    return geometry.x\nprint(geometry.area(1, 1))\n",
        ),
    ]);
    let src = source(dir.path(), "main.craw");
    assert!(src.contains("return geometry.x"), "{src}");
    assert!(src.contains("print(geometry__area(1, 1))"), "{src}");
}

#[test]
fn locals_shadowing_private_names_are_not_renamed() {
    let dir = project(&[
        (
            "m.craw",
            "scale = 1\ndef f(scale):\n    return scale\ndef g():\n    return scale\n",
        ),
        ("main.craw", "from m import f, g\n"),
    ]);
    let src = source(dir.path(), "main.craw");
    assert!(
        src.contains("def m__f(scale):\n    return scale\n"),
        "{src}"
    );
    assert!(src.contains("def m__g():\n    return m__scale\n"), "{src}");
}

#[test]
fn global_declarations_and_fstrings_follow_renames() {
    let dir = project(&[
        (
            "counter.craw",
            "count = 0\ndef bump():\n    global count\n    count = count + 1\n    return f\"now {count}\"\n",
        ),
        (
            "main.craw",
            "from counter import bump\nprint(bump())\nprint(bump())\n",
        ),
    ]);
    let src = source(dir.path(), "main.craw");
    assert!(src.contains("global counter__count"), "{src}");
    assert!(src.contains("f\"now {counter__count}\""), "{src}");
    assert_eq!(interpret(dir.path(), "main.craw"), "now 1\nnow 2\n");
}

#[test]
fn private_functions_with_the_same_name_do_not_clash() {
    let dir = project(&[
        (
            "a.craw",
            "def helper():\n    return \"a\"\ndef fa():\n    return helper()\n",
        ),
        (
            "b.craw",
            "def helper():\n    return \"b\"\ndef fb():\n    return helper()\n",
        ),
        (
            "main.craw",
            "from a import fa\nfrom b import fb\ndef helper():\n    return \"main\"\nprint(fa(), fb(), helper())\n",
        ),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "a b main\n");
}

#[test]
fn modules_with_the_same_stem_get_distinct_prefixes() {
    let dir = project(&[
        ("a/util.craw", "def f():\n    return \"a\"\n"),
        ("b/util.craw", "def f():\n    return \"b\"\n"),
        (
            "main.craw",
            "from a.util import f as fa\nfrom b.util import f as fb\nprint(fa(), fb())\n",
        ),
    ]);
    assert_eq!(interpret(dir.path(), "main.craw"), "a b\n");
}

#[test]
fn modules_come_first_in_dependency_order_and_run_once() {
    let dir = project(&[
        (
            "a.craw",
            "from b import bee\nprint(\"a runs\")\ndef ay():\n    return bee()\n",
        ),
        ("b.craw", "print(\"b runs\")\ndef bee():\n    return 1\n"),
        (
            "main.craw",
            "import a\nimport b\nfrom a import ay\nprint(ay())\n",
        ),
    ]);
    let linked = link(dir.path(), "main.craw").unwrap();
    let names: Vec<_> = linked
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["b.craw", "a.craw", "main.craw"]);
    assert_eq!(interpret(dir.path(), "main.craw"), "b runs\na runs\n1\n");
}

#[test]
fn line_numbers_are_preserved_per_file() {
    let dir = project(&[
        ("lib.craw", "def f():\n    return 1\n"),
        ("main.craw", "# header\nfrom lib import f\n\nprint(f())\n"),
    ]);
    let linked = link(dir.path(), "main.craw").unwrap();
    let print_line = linked
        .source
        .lines()
        .position(|l| l == "print(f())")
        .unwrap() as u32;
    let (file, line) = linked.original_line(print_line).unwrap();
    assert!(file.ends_with("main.craw"));
    assert_eq!(line, 3);
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

#[test]
fn missing_modules_and_names_are_reported_where_they_are_written() {
    let dir = project(&[
        ("lib.craw", "def f():\n    return 1\n"),
        (
            "main.craw",
            "x = 1\nimport nowhere\nfrom lib import g\nimport lib\nprint(lib.h())\n",
        ),
    ]);
    let e = errors(dir.path(), "main.craw");
    assert!(
        e.contains(
            &"main.craw:2:1: cannot find module `nowhere` (looked for `nowhere.craw`)".to_string()
        ),
        "{e:?}"
    );
    assert!(
        e.contains(&"main.craw:3:17: module `lib` has no top-level `g`".to_string()),
        "{e:?}"
    );
    assert!(
        e.contains(&"main.craw:5:11: module `lib` has no top-level `h`".to_string()),
        "{e:?}"
    );
}

#[test]
fn a_non_std_from_import_that_names_no_module_is_an_error() {
    let dir = project(&[("main.craw", "from rand import random\n")]);
    let e = errors(dir.path(), "main.craw");
    assert_eq!(e.len(), 1);
    assert!(
        e[0].contains("cannot find module `rand`") && e[0].contains("use crate_name::item"),
        "{e:?}"
    );
}

#[test]
fn an_unimported_submodule_is_explained() {
    let dir = project(&[
        ("pkg/__init__.craw", ""),
        ("pkg/a.craw", "x = 1\n"),
        ("pkg/b.craw", "y = 2\n"),
        ("main.craw", "import pkg.a\nprint(pkg.a.x, pkg.b.y)\n"),
    ]);
    let e = errors(dir.path(), "main.craw");
    assert_eq!(
        e,
        vec!["main.craw:2:20: `pkg.b` is a module that isn't imported; add `import pkg.b`"]
    );
}

#[test]
fn relative_imports_that_find_nothing_are_errors() {
    let dir = project(&[
        ("pkg/mod.craw", "from .nope import x\n"),
        ("main.craw", "import pkg.mod\n"),
    ]);
    let e = errors(dir.path(), "main.craw");
    assert_eq!(e, vec!["pkg/mod.craw:1:1: cannot find module `.nope`"]);
}

#[test]
fn circular_imports_are_reported_with_the_cycle() {
    let dir = project(&[
        ("a.craw", "import b\n"),
        ("b.craw", "import c\n"),
        ("c.craw", "import a\n"),
        ("main.craw", "import a\n"),
    ]);
    let e = errors(dir.path(), "main.craw");
    assert_eq!(
        e,
        vec!["c.craw:1:1: circular import: a.craw → b.craw → c.craw → a.craw"]
    );
}

#[test]
fn a_module_importing_itself_is_a_cycle() {
    let dir = project(&[("main.craw", "import main\n")]);
    assert!(errors(dir.path(), "main.craw")[0].contains("circular import"));
}

#[test]
fn imports_below_the_top_level_are_rejected() {
    let dir = project(&[
        ("lib.craw", ""),
        ("main.craw", "def f():\n    import lib\n    return 1\n"),
    ]);
    assert_eq!(
        errors(dir.path(), "main.craw"),
        vec!["main.craw:2:5: imports must be at the top level of a file"]
    );
}

#[test]
fn a_macro_defined_in_two_files_is_an_error() {
    let dir = project(&[
        ("a.craw", "macro twice(x):\n    x\n    x\n"),
        ("main.craw", "import a\nmacro twice(y):\n    y\n"),
    ]);
    let e = errors(dir.path(), "main.craw");
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(
        e[0].contains("`twice` is already defined in a.craw"),
        "{e:?}"
    );
}

#[test]
fn a_module_used_as_a_value_is_an_error() {
    let dir = project(&[("lib.craw", ""), ("main.craw", "import lib\nx = lib\n")]);
    assert_eq!(
        errors(dir.path(), "main.craw"),
        vec!["main.craw:2:5: module `lib` can only be used as `lib.name`"]
    );
}

#[test]
fn syntax_errors_in_a_module_point_into_that_module() {
    let dir = project(&[
        ("broken.craw", "x = 1\ny = (\n"),
        ("main.craw", "import broken\n"),
    ]);
    let e = errors(dir.path(), "main.craw");
    assert!(e.iter().all(|m| m.starts_with("broken.craw:")), "{e:?}");
}

#[test]
fn reexporting_a_native_import_is_explained() {
    let dir = project(&[
        ("lib.craw", "from std.collections import HashMap\n"),
        ("main.craw", "from lib import HashMap\n"),
    ]);
    let e = errors(dir.path(), "main.craw");
    assert!(e[0].contains("native Rust import"), "{e:?}");
}

// ---------------------------------------------------------------------
// End to end
// ---------------------------------------------------------------------

const MAIN: &str = "\
import geometry
import shapes.circle
from geometry import area as a, Point
print(geometry.area(3, 4))
print(a(1, 1))
print(shapes.circle.origin())
print(shapes.circle.helper(1))
match shapes.circle.origin():
    case Point(x, y):
        print(\"matched\", x, y)
def helper(v):
    return \"main helper\"
print(helper(1))
";

const CIRCLE: &str = "\
from geometry import Point
def origin():
    return Point(0, 0)
def helper(v):
    return v + 1000
";

const EXPECTED: &str = "geometry loaded\n24\n2\nPoint(0, 0)\n1001\nmatched 0 0\nmain helper\n";

fn full_project() -> tempfile::TempDir {
    project(&[
        ("geometry.craw", GEOMETRY),
        ("shapes/circle.craw", CIRCLE),
        ("main.craw", MAIN),
    ])
}

#[test]
fn the_interpreter_runs_a_multi_file_program() {
    let dir = full_project();
    assert_eq!(interpret(dir.path(), "main.craw"), EXPECTED);
}

#[test]
fn the_compiler_runs_a_multi_file_program_and_rebuilds_when_a_module_changes() {
    let dir = full_project();
    assert_eq!(compile_and_run(dir.path(), "main.craw"), EXPECTED);
    // Edit only the imported module: the cached binary must be rebuilt.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let path = dir.path().join("geometry.craw");
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("return helper(w) * h", "return helper(w) * h + 100");
    std::fs::write(&path, edited).unwrap();
    assert!(compile_and_run(dir.path(), "main.craw").contains("\n124\n"));
}

#[test]
fn the_compiler_keeps_per_module_types_apart() {
    let dir = project(&[
        (
            "a.craw",
            "data Point(x)\ndef make():\n    return Point(1)\n",
        ),
        (
            "main.craw",
            "import a\ndata Point(x)\nmatch a.make():\n    case Point(v):\n        print(\"wrong\")\n    case a.Point(v):\n        print(\"a.Point\", v, a.make())\n",
        ),
    ]);
    assert_eq!(
        compile_and_run(dir.path(), "main.craw"),
        "a.Point 1 Point(1)\n"
    );
}

#[test]
fn the_cli_reports_link_errors_relative_to_the_working_directory() {
    let dir = project(&[("main.craw", "from lib import f\n"), ("lib.craw", "")]);
    let out = Command::new(env!("CARGO_BIN_EXE_craw"))
        .args(["transpile", "main.craw", "out.rs"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("main.craw:1:17: error: module `lib` has no top-level `f`"),
        "{stderr}"
    );
}
