//! Cross-file language server features for Craw modules: diagnostics for
//! broken imports, go to definition / hover / completion / signature help
//! / inlay hints / semantic tokens through imports, and project-wide
//! references and rename.

use craw::lsp_analyzer::LspAnalyzer;
use std::collections::HashMap;
use std::path::Path;
use tower_lsp::lsp_types::*;

const GEOMETRY: &str = "\
# Shapes and measurements.
data Point(x, y)
# Area of a rectangle.
def area(w, h):
    return w * h
def helper():
    return 1
";

const CIRCLE: &str = "\
from geometry import Point
def origin():
    return Point(0, 0)
";

struct Project {
    dir: tempfile::TempDir,
    analyzer: LspAnalyzer,
}

impl Project {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut analyzer = LspAnalyzer::new();
        analyzer.set_roots(vec![dir.path().to_path_buf()]);
        Project { dir, analyzer }
    }

    fn uri(&self, rel: &str) -> String {
        let p = craw::modules::normalize(&self.dir.path().join(rel));
        Url::from_file_path(p).unwrap().to_string()
    }

    /// Opens `rel` with `src` (a `$0` marks the cursor), returning the
    /// diagnostics and the cursor position.
    fn open(&mut self, rel: &str, src: &str) -> (Vec<Diagnostic>, Position) {
        let (text, pos) = match src.find("$0") {
            Some(i) => {
                let text = format!("{}{}", &src[..i], &src[i + 2..]);
                let before = &src[..i];
                let line = before.matches('\n').count() as u32;
                let col = before[before.rfind('\n').map_or(0, |i| i + 1)..]
                    .encode_utf16()
                    .count() as u32;
                (text, Position::new(line, col))
            }
            None => (src.to_string(), Position::new(0, 0)),
        };
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        let uri = self.uri(rel);
        (self.analyzer.update_document(&uri, &text), pos)
    }

    fn file_of(&self, loc: &Location) -> String {
        let p = loc.uri.to_file_path().unwrap();
        p.strip_prefix(craw::modules::normalize(self.dir.path()))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    }
}

fn codes(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .filter_map(|d| match &d.code {
            Some(NumberOrString::String(c)) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

fn geometry() -> Project {
    Project::new(&[("geometry.craw", GEOMETRY), ("shapes/circle.craw", CIRCLE)])
}

// ---------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------

#[test]
fn valid_imports_produce_no_diagnostics() {
    let mut p = geometry();
    let (d, _) = p.open(
        "main.craw",
        "import geometry\nimport shapes.circle\nfrom geometry import area as a, Point\nprint(geometry.area(1, 2), a(1, 1), shapes.circle.origin(), Point(1, 2))\n",
    );
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn a_missing_module_is_reported_on_its_path() {
    let mut p = geometry();
    let (d, _) = p.open("main.craw", "x = 1\nimport shapes.square\n");
    assert_eq!(codes(&d), vec!["E0106"]);
    assert_eq!(d[0].range.start, Position::new(1, 7));
    assert_eq!(d[0].range.end, Position::new(1, 20));
    assert!(d[0].message.contains("shapes/square.craw"));
}

#[test]
fn missing_names_are_reported_on_the_name() {
    let mut p = geometry();
    let (d, _) = p.open(
        "main.craw",
        "from geometry import area, volume\nimport geometry\nprint(geometry.perimeter())\n",
    );
    assert_eq!(codes(&d), vec!["E0107", "E0107"], "{d:#?}");
    assert_eq!(d[0].range.start, Position::new(0, 27));
    assert!(d[0].message.contains("`volume`"));
    assert_eq!(d[1].range.start, Position::new(2, 15));
}

#[test]
fn circular_nested_and_value_uses_of_modules_are_reported() {
    let mut p = Project::new(&[("a.craw", "import main\n"), ("lib.craw", "")]);
    let (d, _) = p.open(
        "main.craw",
        "import a\nimport lib\nx = lib\ndef f():\n    import lib\n    return 1\n",
    );
    let c = codes(&d);
    assert!(c.contains(&"E0108".to_string()), "{d:#?}");
    assert!(c.contains(&"E0110".to_string()), "{d:#?}");
    assert!(c.contains(&"E0109".to_string()), "{d:#?}");
}

#[test]
fn import_paths_and_aliased_names_are_never_unresolved_names() {
    let mut p = geometry();
    let (d, _) = p.open(
        "main.craw",
        "import shapes.circle\nfrom geometry import area as a\nprint(a)\n",
    );
    assert!(!codes(&d).contains(&"E0100".to_string()), "{d:#?}");
}

#[test]
fn an_unsaved_module_buffer_wins_over_disk() {
    let mut p = geometry();
    let (d, _) = p.open("main.craw", "from geometry import volume\n");
    assert_eq!(codes(&d), vec!["E0107"]);
    // Add `volume` in the open (unsaved) geometry buffer.
    let uri = p.uri("geometry.craw");
    p.analyzer
        .update_document(&uri, &format!("{GEOMETRY}def volume():\n    return 3\n"));
    let d = p.analyzer.diagnostics_for(&p.uri("main.craw")).unwrap();
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn a_file_changed_on_disk_is_seen_after_invalidation() {
    let mut p = geometry();
    p.open("main.craw", "from geometry import volume\n");
    let geo = p.dir.path().join("geometry.craw");
    std::fs::write(&geo, format!("{GEOMETRY}def volume():\n    return 3\n")).unwrap();
    p.analyzer.file_changed(&geo);
    let d = p.analyzer.diagnostics_for(&p.uri("main.craw")).unwrap();
    assert!(d.is_empty(), "{d:#?}");
}

// ---------------------------------------------------------------------
// Navigation and hover
// ---------------------------------------------------------------------

fn definition(p: &mut Project, src: &str) -> (String, Position) {
    let (_, pos) = p.open("main.craw", src);
    let loc = p
        .analyzer
        .get_definition(&p.uri("main.craw"), pos.line, pos.character)
        .expect("a definition");
    (p.file_of(&loc), loc.range.start)
}

#[test]
fn definition_follows_imported_names_aliases_and_members() {
    let mut p = geometry();
    let area = ("geometry.craw".to_string(), Position::new(3, 4));
    assert_eq!(
        definition(&mut p, "from geometry import area\nprint($0area(1, 2))\n"),
        area
    );
    assert_eq!(
        definition(&mut p, "from geometry import area as a\nprint($0a(1, 2))\n"),
        area
    );
    assert_eq!(
        definition(&mut p, "import geometry\nprint(geometry.$0area(1, 2))\n"),
        area
    );
    assert_eq!(
        definition(&mut p, "from geometry import $0area as a\n"),
        area
    );
}

#[test]
fn definition_on_a_module_path_opens_the_module() {
    let mut p = geometry();
    assert_eq!(
        definition(&mut p, "import shapes.$0circle\n"),
        ("shapes/circle.craw".to_string(), Position::new(0, 0))
    );
    assert_eq!(
        definition(&mut p, "from $0geometry import area\n"),
        ("geometry.craw".to_string(), Position::new(0, 0))
    );
    assert_eq!(
        definition(&mut p, "import geometry\nprint($0geometry.area(1, 1))\n"),
        ("geometry.craw".to_string(), Position::new(0, 0))
    );
}

#[test]
fn definition_follows_reexports_to_the_original() {
    let mut p = Project::new(&[
        ("base.craw", "def make():\n    return 1\n"),
        ("mid.craw", "from base import make\n"),
    ]);
    assert_eq!(
        definition(&mut p, "from mid import make\nprint($0make())\n"),
        ("base.craw".to_string(), Position::new(0, 4))
    );
}

fn hover(p: &mut Project, src: &str) -> String {
    let (_, pos) = p.open("main.craw", src);
    match p
        .analyzer
        .get_hover_info(&p.uri("main.craw"), pos.line, pos.character)
        .expect("a hover")
        .contents
    {
        HoverContents::Markup(m) => m.value,
        _ => panic!(),
    }
}

#[test]
fn hover_on_an_imported_function_shows_its_signature_docs_and_module() {
    let mut p = geometry();
    let md = hover(&mut p, "import geometry\nprint(geometry.$0area(1, 2))\n");
    assert!(md.starts_with("```craw\ndef area(w, h)\n```"), "{md}");
    assert!(md.contains("Area of a rectangle."), "{md}");
    assert!(md.ends_with("*from `geometry`*"), "{md}");
}

#[test]
fn hover_on_a_module_shows_its_leading_comment() {
    let mut p = geometry();
    let md = hover(&mut p, "import $0geometry\n");
    assert!(md.starts_with("```craw\nmodule geometry\n```"), "{md}");
    assert!(md.contains("Shapes and measurements."), "{md}");
}

// ---------------------------------------------------------------------
// Completion, signature help, inlay hints, semantic tokens
// ---------------------------------------------------------------------

fn complete(p: &mut Project, src: &str) -> Vec<CompletionItem> {
    let (_, pos) = p.open("main.craw", src);
    p.analyzer
        .get_completions(&p.uri("main.craw"), pos.line, pos.character)
}

fn labels(items: &[CompletionItem]) -> Vec<&str> {
    items.iter().map(|i| i.label.as_str()).collect()
}

#[test]
fn completion_after_a_module_lists_its_top_level_names() {
    let mut p = geometry();
    let items = complete(&mut p, "import geometry\nx = 1\ngeometry.$0\n");
    assert_eq!(labels(&items), vec!["Point", "area", "helper"]);
    let area = items.iter().find(|i| i.label == "area").unwrap();
    assert_eq!(area.kind, Some(CompletionItemKind::FUNCTION));
    assert_eq!(area.detail.as_deref(), Some("def area(w, h)"));
    let point = items.iter().find(|i| i.label == "Point").unwrap();
    assert_eq!(point.kind, Some(CompletionItemKind::CLASS));
}

#[test]
fn completion_filters_while_typing_a_member() {
    let mut p = geometry();
    let items = complete(&mut p, "import geometry\ngeometry.ar$0\n");
    assert!(labels(&items).contains(&"area"));
}

#[test]
fn completion_in_from_import_lists_the_modules_names() {
    let mut p = geometry();
    let items = complete(&mut p, "from geometry import $0");
    assert_eq!(labels(&items), vec!["Point", "area", "helper"]);
    let items = complete(&mut p, "from geometry import area, $0");
    assert!(labels(&items).contains(&"Point"));
}

#[test]
fn completion_after_import_lists_workspace_modules() {
    let mut p = geometry();
    let items = complete(&mut p, "import $0");
    let l = labels(&items);
    assert!(
        l.contains(&"geometry") && l.contains(&"shapes.circle"),
        "{l:?}"
    );
    assert!(!l.contains(&"main"), "a file doesn't import itself: {l:?}");
    let items = complete(&mut p, "import shapes.$0");
    assert_eq!(labels(&items), vec!["circle"]);
    let items = complete(&mut p, "from $0");
    let l = labels(&items);
    assert!(l.contains(&"std.math") && l.contains(&"geometry"), "{l:?}");
}

#[test]
fn signature_help_works_through_imports_even_mid_edit() {
    let mut p = geometry();
    for src in [
        "from geometry import area\narea(1, $0\n",
        "import geometry\ngeometry.area(1, $0\n",
    ] {
        let (_, pos) = p.open("main.craw", src);
        let s = p
            .analyzer
            .get_signature_help(&p.uri("main.craw"), pos.line, pos.character)
            .unwrap_or_else(|| panic!("no signature help for {src:?}"));
        assert_eq!(s.signatures[0].label, "area(w, h)");
        assert_eq!(s.active_parameter, Some(1));
    }
}

#[test]
fn parameter_hints_label_imported_calls() {
    let mut p = geometry();
    let (_, _) = p.open(
        "main.craw",
        "import geometry\nfrom geometry import area\nprint(area(1, 2), geometry.area(3, 4))\n",
    );
    let hints = p
        .analyzer
        .inlay_hints(
            &p.uri("main.craw"),
            Range::new(Position::new(0, 0), Position::new(3, 0)),
        )
        .unwrap();
    let labels: Vec<String> = hints
        .into_iter()
        .map(|h| match h.label {
            InlayHintLabel::String(s) => s,
            _ => panic!(),
        })
        .collect();
    assert_eq!(
        labels.iter().filter(|l| *l == "w:").count(),
        2,
        "{labels:?}"
    );
    assert_eq!(
        labels.iter().filter(|l| *l == "h:").count(),
        2,
        "{labels:?}"
    );
}

#[test]
fn semantic_tokens_classify_imports_by_what_they_name() {
    let mut p = geometry();
    let text = "import geometry\nfrom geometry import area, Point\nprint(area(1, 1), geometry.Point(1, 2))\n";
    p.open("main.craw", text);
    let data = p
        .analyzer
        .semantic_tokens(&p.uri("main.craw"), None)
        .unwrap();
    let types = craw::ide::semantic_tokens::TOKEN_TYPES;
    // Decode into (line, col, type).
    let (mut line, mut col) = (0, 0);
    let mut toks = Vec::new();
    for t in data {
        if t.delta_line > 0 {
            line += t.delta_line;
            col = t.delta_start;
        } else {
            col += t.delta_start;
        }
        toks.push((line, col, types[t.token_type as usize]));
    }
    let at = |l: u32, c: u32| toks.iter().find(|t| t.0 == l && t.1 == c).map(|t| t.2);
    assert_eq!(at(0, 7), Some("namespace"), "{toks:?}"); // geometry
    assert_eq!(at(2, 6), Some("function"), "{toks:?}"); // area(...)
    assert_eq!(at(2, 18), Some("namespace"), "{toks:?}"); // geometry.
    assert_eq!(at(2, 27), Some("class"), "{toks:?}"); // .Point
    assert_eq!(at(1, 27), Some("class"), "{toks:?}"); // imported Point
}

// ---------------------------------------------------------------------
// References and rename across files
// ---------------------------------------------------------------------

fn by_file(p: &Project, locs: &[Location]) -> HashMap<String, Vec<(u32, u32)>> {
    let mut m: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
    for l in locs {
        m.entry(p.file_of(l))
            .or_default()
            .push((l.range.start.line, l.range.start.character));
    }
    m
}

const USES: &str = "\
import geometry
from geometry import area
from geometry import area as a
print(area(1, 2), a(3, 4), geometry.area(5, 6))
";

#[test]
fn references_span_every_file_that_uses_the_function() {
    let mut p = geometry();
    p.open("main.craw", USES);
    // From the definition, in geometry.craw (opened so it has a model).
    p.open("geometry.craw", GEOMETRY);
    let refs = p
        .analyzer
        .find_references(&p.uri("geometry.craw"), 3, 5)
        .unwrap();
    let m = by_file(&p, &refs);
    assert_eq!(m["geometry.craw"], vec![(3, 4)]);
    assert_eq!(
        m["main.craw"],
        vec![(1, 21), (2, 21), (2, 29), (3, 6), (3, 18), (3, 36)],
        "binding, aliased original, alias, alias use, member, use"
    );
}

#[test]
fn references_from_a_use_site_find_the_same_set() {
    let mut p = geometry();
    let (_, pos) = p.open("main.craw", &USES.replace("print(area", "print($0area"));
    let refs = p
        .analyzer
        .find_references(&p.uri("main.craw"), pos.line, pos.character)
        .unwrap();
    let m = by_file(&p, &refs);
    assert_eq!(m["geometry.craw"], vec![(3, 4)]);
    assert_eq!(m["main.craw"].len(), 6);
}

fn rename(p: &mut Project, src: &str, new: &str) -> HashMap<String, String> {
    let (_, pos) = p.open("main.craw", src);
    let edit = p
        .analyzer
        .rename_symbol(
            &p.uri("main.craw"),
            pos.line,
            pos.character,
            new.to_string(),
        )
        .expect("rename allowed")
        .expect("an edit");
    let mut out = HashMap::new();
    for (uri, edits) in edit.changes.unwrap() {
        let path = uri.to_file_path().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let rel = p.file_of(&Location {
            uri: uri.clone(),
            range: Range::default(),
        });
        out.insert(rel, apply(&text, &edits));
    }
    out
}

fn apply(text: &str, edits: &[TextEdit]) -> String {
    let li = craw::text::LineIndex::new(text);
    let mut spans: Vec<_> = edits
        .iter()
        .map(|e| {
            (
                li.offset(e.range.start.into()).unwrap(),
                li.offset(e.range.end.into()).unwrap(),
                e.new_text.clone(),
            )
        })
        .collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut out = text.to_string();
    for (s, e, t) in spans {
        out.replace_range(s..e, &t);
    }
    out
}

#[test]
fn renaming_an_imported_function_edits_its_module_and_every_importer() {
    let mut p = geometry();
    let files = rename(
        &mut p,
        &USES.replace("print(area", "print($0area"),
        "surface",
    );
    assert!(files["geometry.craw"].contains("def surface(w, h):"));
    assert_eq!(
        files["main.craw"],
        "import geometry\nfrom geometry import surface\nfrom geometry import surface as a\nprint(surface(1, 2), a(3, 4), geometry.surface(5, 6))\n"
    );
}

#[test]
fn renaming_from_the_definition_reaches_importers_too() {
    let mut p = geometry();
    p.open("main.craw", USES);
    let (_, pos) = p.open("geometry.craw", &GEOMETRY.replace("def area", "def $0area"));
    let edit = p
        .analyzer
        .rename_symbol(
            &p.uri("geometry.craw"),
            pos.line,
            pos.character,
            "surface".into(),
        )
        .unwrap()
        .unwrap();
    let changes = edit.changes.unwrap();
    assert_eq!(changes.len(), 2, "both files are edited");
}

#[test]
fn renaming_an_alias_stays_in_this_file() {
    let mut p = geometry();
    let files = rename(
        &mut p,
        "from geometry import area as a\nprint($0a(1, 2))\n",
        "rect",
    );
    assert_eq!(files.len(), 1);
    assert_eq!(
        files["main.craw"],
        "from geometry import area as rect\nprint(rect(1, 2))\n"
    );
}

#[test]
fn a_module_name_cannot_be_renamed_in_place() {
    let mut p = geometry();
    let (_, pos) = p.open("main.craw", "import $0geometry\n");
    let uri = p.uri("main.craw");
    assert!(
        p.analyzer
            .prepare_rename(&uri, pos.line, pos.character)
            .is_none()
    );
    assert!(
        p.analyzer
            .rename_symbol(&uri, pos.line, pos.character, "geo".into())
            .is_err()
    );
}

#[test]
fn locals_still_rename_within_their_file() {
    let mut p = geometry();
    let files = rename(&mut p, "def f():\n    $0t = 1\n    return t\n", "u");
    assert_eq!(files.len(), 1);
    assert_eq!(files["main.craw"], "def f():\n    u = 1\n    return u\n");
}

#[test]
fn project_features_are_inert_for_untitled_documents() {
    let mut a = LspAnalyzer::new();
    let uri = "untitled:Untitled-1";
    let d = a.update_document(uri, "import geometry\nprint(geometry.area(1))\n");
    assert!(
        d.iter()
            .all(|d| d.code != Some(NumberOrString::String("E0106".into())))
    );
    assert!(a.get_definition(uri, 1, 16).is_none());
    assert!(
        a.find_references(uri, 1, 16).is_none()
            || a.find_references(uri, 1, 16).unwrap().is_empty()
    );
}

#[allow(dead_code)]
fn exists(p: &Path) -> bool {
    p.exists()
}

// ---------------------------------------------------------------------
// Python import forms
// ---------------------------------------------------------------------

#[test]
fn a_b_name_chains_resolve_through_packages() {
    let mut p = geometry();
    assert_eq!(
        definition(
            &mut p,
            "import shapes.circle\nprint(shapes.circle.$0origin())\n"
        ),
        ("shapes/circle.craw".to_string(), Position::new(1, 4))
    );
    let md = hover(
        &mut p,
        "import shapes.circle\nprint(shapes.$0circle.origin())\n",
    );
    assert!(md.starts_with("```craw\nmodule circle\n```"), "{md}");
    let items = complete(&mut p, "import shapes.circle\nshapes.circle.$0\n");
    assert_eq!(labels(&items), vec!["Point", "origin"]);
}

#[test]
fn a_package_lists_its_submodules_after_a_dot() {
    let mut p = Project::new(&[
        ("pkg/__init__.craw", "version = 1\n"),
        ("pkg/tools.craw", "def hammer():\n    return 1\n"),
    ]);
    let items = complete(&mut p, "import pkg\npkg.$0\n");
    let l = labels(&items);
    assert!(l.contains(&"version") && l.contains(&"tools"), "{l:?}");
    let items = complete(&mut p, "from pkg import $0");
    let l = labels(&items);
    assert!(l.contains(&"version") && l.contains(&"tools"), "{l:?}");
}

#[test]
fn relative_imports_navigate_and_complete() {
    let mut p = Project::new(&[
        ("app/__init__.craw", ""),
        ("app/config.craw", "name = \"craw\"\n"),
        ("app/core/__init__.craw", ""),
        ("app/core/helpers.craw", "def shout(s):\n    return s\n"),
    ]);
    let (_, pos) = p.open(
        "app/core/engine.craw",
        "from ..config import name\nfrom . import helpers\nprint(name, helpers.$0shout(1))\n",
    );
    let uri = p.uri("app/core/engine.craw");
    let d = p.analyzer.diagnostics_for(&uri).unwrap();
    assert!(d.is_empty(), "{d:#?}");
    let loc = p
        .analyzer
        .get_definition(&uri, pos.line, pos.character)
        .unwrap();
    assert_eq!(p.file_of(&loc), "app/core/helpers.craw");
    let (_, pos) = p.open("app/core/engine.craw", "from .$0");
    let items = p.analyzer.get_completions(&uri, pos.line, pos.character);
    assert!(labels(&items).contains(&"helpers"), "{:?}", labels(&items));
    let (_, pos) = p.open("app/core/engine.craw", "from ..$0");
    let items = p.analyzer.get_completions(&uri, pos.line, pos.character);
    let l = labels(&items);
    assert!(l.contains(&"config") && l.contains(&"core"), "{l:?}");
}

#[test]
fn star_imported_names_are_not_unresolved_and_navigate() {
    let mut p = geometry();
    let (d, pos) = p.open(
        "main.craw",
        "from geometry import *\nprint($0area(1, 2), Point(1, 2))\n",
    );
    assert!(d.is_empty(), "{d:#?}");
    let loc = p
        .analyzer
        .get_definition(&p.uri("main.craw"), pos.line, pos.character)
        .unwrap();
    assert_eq!(p.file_of(&loc), "geometry.craw");
}

#[test]
fn native_std_imports_warn_and_offer_the_use_form() {
    let mut p = geometry();
    let (d, _) = p.open(
        "main.craw",
        "from std.collections import HashMap, HashSet as Set\n",
    );
    assert_eq!(codes(&d), vec!["W0201"]);
    assert_eq!(d[0].severity, Some(DiagnosticSeverity::WARNING));
    let actions = p
        .analyzer
        .code_actions(
            &p.uri("main.craw"),
            Range::new(Position::new(0, 3), Position::new(0, 3)),
        )
        .unwrap();
    let fix = actions
        .iter()
        .find(|a| a.title == "Use Rust `use` syntax")
        .expect("a quick fix");
    let edit = &fix.edit.as_ref().unwrap().changes.as_ref().unwrap()
        [&Url::parse(&p.uri("main.craw")).unwrap()][0];
    assert_eq!(
        edit.new_text,
        "use std::collections::{HashMap, HashSet as Set};"
    );
}

#[test]
fn a_non_std_from_import_of_nothing_is_an_error() {
    let mut p = geometry();
    let (d, _) = p.open("main.craw", "from rand import random\n");
    assert_eq!(codes(&d), vec!["E0106"]);
}

#[test]
fn an_unimported_submodule_is_diagnosed_like_the_compiler_does() {
    let mut p = Project::new(&[
        ("pkg/__init__.craw", ""),
        ("pkg/a.craw", "x = 1\n"),
        ("pkg/b.craw", "y = 2\n"),
    ]);
    let (d, _) = p.open("main.craw", "import pkg.a\nprint(pkg.a.x, pkg.b.y)\n");
    assert_eq!(codes(&d), vec!["E0107"], "{d:#?}");
    assert!(d[0].message.contains("add `import pkg.b`"));
}

#[test]
fn several_modules_per_import_and_parenthesized_items_are_understood() {
    let mut p = geometry();
    let (d, _) = p.open(
        "main.craw",
        "import geometry, shapes.circle as c\nfrom geometry import (\n    area,\n    Point,\n)\nprint(geometry.area(1, 1), c.origin(), area(2, 2), Point(1, 2))\n",
    );
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn renames_reach_through_chains_and_star_imports() {
    let mut p = geometry();
    p.open("other.craw", "from geometry import *\nprint(area(1, 2))\n");
    let files = rename(
        &mut p,
        "import geometry\nprint(geometry.$0area(1, 2))\n",
        "surface",
    );
    assert_eq!(
        files["main.craw"],
        "import geometry\nprint(geometry.surface(1, 2))\n"
    );
    assert!(files["geometry.craw"].contains("def surface(w, h):"));
    assert_eq!(
        files["other.craw"],
        "from geometry import *\nprint(surface(1, 2))\n"
    );
}
