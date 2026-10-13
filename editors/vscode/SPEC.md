# Craw for VS Code — Feature Specification

This document specifies what the Craw extension and its language server
(`craw-lsp`) provide, and how each requirement is verified. Every row names
the test file that enforces it; a requirement without a test is not
considered implemented.

Test suites:

| Suite | Runs | What it proves |
| --- | --- | --- |
| `tests/ide_features_test.rs` | `cargo test` | Each server feature, directly against the analyzer |
| `tests/lsp_protocol_test.rs` | `cargo test` | The real `craw-lsp` binary over stdio JSON-RPC: every capability is advertised *and* served |
| `tests/lsp_features_test.rs`, `lsp_analyzer_test.rs`, `semantic_diagnostics_test.rs` | `cargo test` | Pre-existing completion / hover / diagnostics behavior |
| `src/test/unit/*.test.ts` | `npm test` | Client logic, manifest consistency, activation against a mocked `vscode` API, grammar tokenization |
| `src/test/integration/server.test.ts` | `npm test` | The extension's settings and custom requests against the real server binary (skipped if it isn't built) |

## 1. Language server

### 1.1 Document sync and diagnostics

| Requirement | Tests |
| --- | --- |
| Incremental sync; each change in a notification applied in order | `lsp_protocol_test::incremental_changes_update_diagnostics_and_answers` |
| Syntax errors (`E0001`) with precise ranges; semantic diagnostics: unresolved (`E0100`), duplicate (`E0101`), arity (`E0102`), misplaced `return`/`break`/`continue`/`nonlocal` (`E0103`–`E0105`), unused locals (`W0200`, tagged *unnecessary*) | `semantic_diagnostics_test`, `lsp_protocol_test::every_request_is_served_over_the_wire` |
| `craw.diagnostics.unresolved` / `.unused` filter diagnostics and republish on change | `lsp_protocol_test::configuration_changes_filter_diagnostics_and_hints` |
| No source text is silently dropped by the lexer: code after `rust:` on its line is Rust; a bare `!` is a positioned syntax error with a hint | `lexer_fixes_test` |
| Untitled (unsaved) documents are fully served; closing a document drops its state | `lsp_protocol_test::closing_a_document_clears_its_state` |

### 1.2 Navigation

| Requirement | Tests |
| --- | --- |
| Go to definition, find references, document highlight (read/write) | `lsp_features_test`, `lsp_protocol_test` |
| Go to **type** definition: a variable whose type is inferred as a `data`/`class` jumps to that type | `ide_features_test::type_definition_*` |
| Document symbols (nested) and workspace symbols | `lsp_protocol_test` |
| **Call hierarchy**: incoming calls grouped by caller (module-level code appears as the file); outgoing calls de-duplicated per callee; calls inside nested functions belong to them | `ide_features_test::call_hierarchy_*`, `incoming_calls_*`, `outgoing_calls_*` |

### 1.3 Hover, completion, signature help

| Requirement | Tests |
| --- | --- |
| Hover on keywords shows their syntax and meaning | `ide_features_test::hover_on_keywords_shows_their_syntax`, `keyword_docs::every_keyword_in_the_lexer_table_is_documented` |
| Hover on a variable/parameter shows its declared or **inferred** type (`x: Int`, `p: Point`) | `ide_features_test::hover_shows_*`, `lsp_features_test::hover_on_a_plain_variable_names_its_scope` |
| Completion items carry the kind of what they name (function, class, variable, ...), a signature or inferred type as detail, and doc comments | `ide_features_test::completion_kinds_reflect_what_a_local_is` |
| Expression positions also offer expression keywords (`not`, `lambda`, `if` …), never statement keywords, never duplicate labels | `ide_features_test::completion_offers_expression_keywords_after_locals` |
| Member completion narrows by inferred receiver type | `lsp_features_test::member_completion_*` |
| Signature help for builtins lists real parameters; the active parameter follows commas **and** `name=` keyword arguments; documentation is attached | `ide_features_test::signature_help_*` |

### 1.3a Modules (cross-file)

Imports follow Python's semantics and resolve exactly as the compiler
resolves them (`crate::modules`): absolute imports from the workspace
folders (and a file's own non-package directory), relative imports from
the importing file's package, packages via `__init__.craw` (or none).
Open buffers win over disk; on-disk models are cached and invalidated by
file-watcher events, and every open document's diagnostics are republished
when any file changes.

| Requirement | Tests |
| --- | --- |
| Broken imports are diagnosed where they are written: missing module (`E0106`, including a non-`std` `from` import of nothing), missing name (`E0107`, including `a.b.name` chains and a submodule that isn't imported), circular import (`E0108`), import below top level (`E0109`), module used as a value (`E0110`); import paths and aliased originals are never "unresolved names", nor are names from `import *` | `lsp_modules_test::a_missing_module_*`, `missing_names_*`, `an_unimported_submodule_*`, `circular_nested_*`, `star_imported_names_*` |
| `from std.x import y` warns (`W0201`) with a quick fix to `use std::x::y;` | `lsp_modules_test::native_std_imports_warn_and_offer_the_use_form` |
| Go to definition from an imported name, an alias, `a.b.name`, a name inside the import statement, a module path, a relative import, or a `*`-imported name; follows re-exports | `lsp_modules_test::definition_*`, `a_b_name_chains_*`, `relative_imports_navigate_and_complete`, `star_imported_names_*` |
| Hover shows the definition's signature and docs plus its module; hovering a module shows its leading comment | `lsp_modules_test::hover_*`, `a_b_name_chains_*` |
| Completion: `a.b.` lists the module's names (and a package's submodules); `from m import ` lists names and submodules; `import `, `import a.`, `from `, `from .`, `from ..` list modules | `lsp_modules_test::completion_*`, `a_package_lists_*`, `relative_imports_navigate_and_complete` |
| Signature help and parameter inlay hints for imported functions and `module.f(...)`, including mid-edit | `lsp_modules_test::signature_help_works_through_imports_even_mid_edit`, `parameter_hints_label_imported_calls` |
| Semantic tokens classify imports by what they name (`namespace` for modules) | `lsp_modules_test::semantic_tokens_classify_imports_by_what_they_name` |
| References and rename span every workspace file, through `from` imports, uses, `a.b.name` chains, `*` imports and the original name in `x as y`; renaming an alias stays local; a module binding can't be renamed in place | `lsp_modules_test::references_*`, `renaming_*`, `renames_reach_through_chains_and_star_imports` |
| Unsaved edits to an open module are seen by its importers and by **Show Generated Rust** | `lsp_modules_test::an_unsaved_module_buffer_wins_over_disk`, `lsp_protocol_test::modules_work_across_files_over_the_wire` |

### 1.4 Type inference

Inference is deliberately conservative: it answers only when the program's
own shape proves the type, so every hint and hover it produces is true.

| Requirement | Tests |
| --- | --- |
| Literals, arithmetic promotion, string/list concatenation and repetition, comparisons, ternaries, constructors, annotated parameters and returns | `lsp_features_test`, `ide_features_test::type_hints_*` |
| Calls to **unannotated** user functions are typed from their `return`s when all agree | `ide_features_test::type_hints_see_through_calls_to_unannotated_functions` |
| Recursive and mutually recursive functions terminate (and stay unknown) | `ide_features_test::inference_terminates_on_recursive_functions` |

### 1.5 Inlay hints

| Requirement | Tests |
| --- | --- |
| Parameter names at call sites of user functions and data constructors; hidden when the argument already names the parameter; stop at the first keyword argument; vararg labelled once | `ide_features_test::parameter_hints_*` |
| Inferred type after a first, unannotated assignment; hidden when the initializer is self-evident (`x = 1`, `p = Point(..)`) | `ide_features_test::type_hints_*` |
| Inferred `-> Type` on unannotated `def`s whose returns agree; none on bare `return`, disagreement, generators | `ide_features_test::return_type_hints_*` |
| Each kind toggles independently (`craw.inlayHints.*.enable`); results respect the requested range | `ide_features_test::inlay_hint_kinds_can_be_disabled_individually`, `inlay_hints_respect_the_requested_range`, integration test |

### 1.6 Semantic highlighting

| Requirement | Tests |
| --- | --- |
| Identifiers classified by what they resolve to: `parameter`, `variable` (+`global` at module scope), `function`/`method`, `class`, `struct`, `interface`, `property`, `macro`, `namespace`, `type`, `decorator`, `unresolvedReference` | `ide_features_test::semantic_tokens_*` |
| Modifiers: `declaration`+`definition` at binding sites, `modification` on writes, `defaultLibrary` on builtins | `ide_features_test::semantic_tokens_mark_definitions_and_writes`, `..._tag_builtins_as_default_library` |
| Names interpolated in f-strings are classified; raw Rust is not | `ide_features_test::semantic_tokens_cover_names_interpolated_in_fstrings`, `..._skip_rust_blocks` |
| Correct UTF-16 delta encoding; sorted, non-overlapping; `full` and `range` requests | `ide_features_test::semantic_tokens_encode_utf16_deltas`, `..._are_sorted_and_non_overlapping`, `..._range_request_only_returns_that_range` |
| Custom types/modifiers are declared in `package.json` | `manifest.test.ts > semantic tokens` |

### 1.7 Structure

| Requirement | Tests |
| --- | --- |
| **Folding**: indented blocks (to their last line of content), multi-line brackets (closer stays visible), comment runs, `# region`/`# endregion`, consecutive imports, `rust:` blocks; one range per start line | `ide_features_test::folding_*` |
| **Selection ranges** ("Expand Selection"): token → bracket contents → brackets → statement → block body → block with header → document, strictly growing, one result per requested position | `ide_features_test::selection_*` |

### 1.8 Code actions

| Requirement | Tests |
| --- | --- |
| Quick fix `W0200`: prefix the unused binding with `_` everywhere (preferred) | `ide_features_test::quick_fix_prefixes_an_unused_local_everywhere` |
| Quick fix `E0100`: up to three closest *visible* names or builtins by edit distance; preferred only when one is strictly closest | `ide_features_test::quick_fix_*` |
| Refactor: add the inferred type annotation (`x: Int = ...`) | `ide_features_test::refactor_adds_an_inferred_type_annotation` |
| Refactor: extract the selected expression into a fresh variable before the statement; refused for partial tokens, unbalanced selections, lambda/comprehension bodies, whole statements | `ide_features_test::refactor_extract_*` |
| Refactor: inline a variable (from its definition or a use), parenthesizing multi-token initializers; refused when reassigned, when the initializer calls anything, or reads a reassigned name | `ide_features_test::refactor_inline_*` |
| `context.only` filters by kind hierarchy (`refactor` matches `refactor.extract`) | `lsp_protocol_test::every_request_is_served_over_the_wire` |

### 1.9 Formatting

A conservative, token-driven formatter (`textDocument/formatting` and
`rangeFormatting`). It never reflows code.

| Requirement | Tests |
| --- | --- |
| One space around binary, comparison, pipe and compose operators and `=`; none around `=` inside brackets (keyword args, defaults); unary `-`/`*`/`**` stay tight; one space after `,`/`:`/`;`; none inside brackets, before call/index brackets, around `.` | `ide_features_test::format_*` |
| Generic angles (`List<Int>`) and slice colons keep their spacing | `format_preserves_generic_type_spacing`, `format_spaces_dict_colons_but_preserves_slices` |
| Comments preserved (two spaces before a trailing `#`); strings untouched | `format_keeps_comments_with_two_spaces_before_them` |
| Trailing whitespace stripped; >2 blank lines collapsed; leading blank lines dropped; exactly one final newline; CRLF preserved | `format_cleans_whitespace_and_blank_lines`, `format_preserves_crlf_line_endings` |
| `rust:` blocks and lines inside multi-line brackets are never respaced | `format_leaves_rust_blocks_and_multiline_brackets_alone` |
| **Safety**: the token stream is re-lexed and must be unchanged; a line whose respacing would change it gets whitespace-only fixes. Verified token-identical, comment-preserving and idempotent on every `.craw` file in the repo | `format_never_changes_tokens_and_is_idempotent_on_every_sample` |
| Range formatting touches only the selected lines; edits are minimal and per line | `range_formatting_only_touches_the_selected_lines` |

### 1.10 Custom requests

| Request | Result | Tests |
| --- | --- | --- |
| `craw/syntaxTree {textDocument}` | The error-recovered AST as text, plus syntax errors | `ide_features_test::syntax_tree_dumps_the_ast_and_errors`, `lsp_protocol_test` |
| `craw/transpile {textDocument}` | `{rust}` for the *unsaved* buffer, or a request error listing `line:col: message` syntax errors. Runs in a child process so a transpiler panic can't crash the server (release builds abort on panic) | `lsp_protocol_test::transpile_reports_syntax_errors_as_request_errors` |

### 1.11 Robustness

| Requirement | Tests |
| --- | --- |
| No feature panics on broken or partial input, at any cursor position | `ide_features_test::features_never_panic_on_broken_or_partial_input` |
| Unknown documents return `null`, never an error | `ide_features_test::features_return_none_for_unknown_documents` |
| `serverInfo` reports name and version | `lsp_protocol_test::initialize_advertises_every_capability` |

## 2. Extension (client)

| Requirement | Tests |
| --- | --- |
| Activates on Craw files and on workspaces containing them | `manifest.test.ts` |
| Serves `file` **and** `untitled` Craw documents | `activation.test.ts` |
| Server discovery: `craw.server.path` → `$CRAW_LSP_PATH` → workspace build (when `craw.server.useWorkspaceBuild`) → bundled → `PATH`. Workspace builds beat the bundled binary (a stale bundled server must not shadow a fresh build) | `serverPath.test.ts` |
| Path settings expand `${workspaceFolder}`, `${userHome}`, `${env:NAME}`, `~`; relative paths resolve against the workspace | `config.test.ts`, `activation.test.ts` |
| Settings sent to the server are a plain object in the shape `craw-lsp` parses, and match it key for key | `config.test.ts`, `manifest.test.ts > server-side settings match`, integration test |
| A missing server is reported with a link to the setting; user-initiated stops aren't reported as crashes; start is idempotent; server settings changes offer a restart | `activation.test.ts` |
| Status bar shows server state and version | `status.test.ts` |
| Run / Run (Release) / Build / Transpile / Run in Interpreter: dirty files are saved first (`craw.run.saveBeforeRun`), untitled files prompt to save, runs start in the file's directory, `$craw`/`$craw-rustc` problem matchers | `activation.test.ts > run commands` |
| REPL opens in an interactive terminal with `craw.interpreter.path` | `activation.test.ts` |
| **Show Syntax Tree** and **Show Generated Rust** open live, read-only views beside the editor that refresh (debounced) as the source changes; server errors render as comments | `activation.test.ts > views and lenses`, `virtualDocs.test.ts` |
| Run / Build / Show Rust code lenses on the first line of code (`craw.lens.run.enable`) | `codeLens.test.ts`, `activation.test.ts` |
| Task provider for `run`/`build`/`transpile`, `${workspaceFolder}` in `file`, unknown commands rejected | `activation.test.ts` |
| Every contributed command is registered and vice versa; menus/keybindings reference real commands; editor-only commands hidden from the palette elsewhere; keybindings don't shadow built-ins (`Ctrl+Shift+R` is Refactor); every setting read is declared and every declared setting is used | `manifest.test.ts` |
| TextMate grammar matches the lexer: every keyword, operator family, f-string interpolation, numbers, strings, decorators, builtin functions/types, embedded Rust (including code on the `rust:` line) | `grammar.test.ts` |
| `craw.yaml` (micro) highlights exactly the lexer's keywords | `microSyntax.test.ts` |

## 3. Not in scope (and why)

- **Cross-file call hierarchy and type inference through imports.** Call
  hierarchy and inferred types (hover types, type hints) stop at file
  boundaries; navigation, references and rename do not.
- **Reflowing formatter.** Long-line wrapping needs a full pretty-printer
  over the AST, which today doesn't carry expression spans; the
  token-level formatter is the safe subset.
- **Debugging.** There is no debug adapter for the transpiled program.
