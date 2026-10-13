# Craw Language Support

Language support for the **Craw** programming language, backed by `craw-lsp`.

## Features

- **Syntax highlighting** generated from the lexer: keywords, every operator
  family (pipes, composition, broadcasting, Unicode operators), f-string
  interpolation, decorators, builtins, and embedded Rust.
- **Semantic highlighting**: parameters, locals, module globals, functions,
  classes, fields, builtins and unresolved names are colored by what they
  *resolve to*, not just how they look.
- **Diagnostics**: syntax errors with precise ranges, plus semantic checks
  (unresolved names, unused locals, duplicate definitions, call-arity
  mismatches, misplaced `return`/`break`/`continue`/`nonlocal`).
- **Inlay hints**: parameter names at call sites, inferred variable types,
  and inferred return types of unannotated functions.
- **Type inference** behind hover, completion, hints and *Go to Type
  Definition* — conservative, so what it shows is always true.
- **Code actions**: prefix unused names with `_`, "did you mean" fixes for
  typos, add an inferred type annotation, extract expression into a
  variable, inline a variable.
- **Formatting** (document and selection): consistent operator and comma
  spacing, whitespace and blank-line cleanup. Token-preserving by
  construction — it re-lexes its own output and never changes meaning.
- **Completion** (context-aware, typed member completion, snippets),
  **signature help** (active parameter follows keyword arguments), **hover**
  (including keywords).
- **Navigation**: go to definition / type definition, find references,
  highlights, rename, document and workspace symbols, **call hierarchy**.
- **Modules**: `import` / `from ... import` resolve to other `.craw` files;
  definition, hover, completion, signature help, references and rename
  work across files, and broken imports are flagged as you type.
- **Folding** (blocks, brackets, comments, imports, `# region`) and
  **Expand Selection**.
- **Run / Build / Show Rust** code lenses, **Show Generated Rust** and
  **Show Syntax Tree** live views, tasks, and a REPL terminal.

See [SPEC.md](SPEC.md) for the full specification and the tests that verify
each requirement.

```python
# Plain Rust function in a Craw file
fn add(a: i64, b: i64) -> i64 {
    a + b
}

def main():
    print("Result:", add(1, 2))
```

## Requirements

- A `craw-lsp` binary. The extension bundles one for your platform; if none
  is bundled, set `craw.server.path` or put `craw-lsp` on your `PATH`.
- The `craw` CLI (for the run/build/transpile commands) and a Rust toolchain
  (to compile the code Craw generates).

## Settings

| Setting | Default | Description |
| --- | --- | --- |
| `craw.server.path` | | Path to `craw-lsp`. Supports `${workspaceFolder}`, `${userHome}`, `${env:NAME}`, `~`. |
| `craw.server.extraEnv` | `{}` | Extra environment variables for the server process. |
| `craw.server.useWorkspaceBuild` | `false` | Prefer `<workspace>/target/{release,debug}/craw-lsp` over the bundled binary (development). |
| `craw.cli.path` | `craw` | The `craw` CLI used by run/build/transpile. |
| `craw.interpreter.path` | `crawler` | The interpreter used by the REPL and "Run File in Interpreter". |
| `craw.run.saveBeforeRun` | `true` | Save the file before running it (the CLI reads the file on disk). |
| `craw.lens.run.enable` | `true` | Run / Build / Show Rust code lenses. |
| `craw.inlayHints.parameterNames.enable` | `true` | Parameter-name hints at call sites. |
| `craw.inlayHints.typeHints.enable` | `true` | Inferred-type hints on variables. |
| `craw.inlayHints.returnTypeHints.enable` | `true` | Inferred return-type hints on functions. |
| `craw.diagnostics.unresolved` | `true` | Report names that don't resolve. |
| `craw.diagnostics.unused` | `true` | Report unused locals and parameters. |
| `craw.trace.server` | `off` | LSP protocol tracing: `off`, `messages`, or `verbose`. |

Server discovery order: `craw.server.path` → `$CRAW_LSP_PATH` → the
workspace build (if enabled) → the binary bundled with the extension →
`craw-lsp` on `PATH`.

## Commands

- **Craw: Run File** (`Ctrl+Alt+R`), **Run File (Release)**, **Build File**,
  **Transpile File**, **Run File in Interpreter**, **Open REPL**, **New Project**
- **Craw: Show Generated Rust**, **Craw: Show Syntax Tree** (live views)
- **Craw: Restart / Stop / Start Language Server**, **Show Language Server
  Output**, **Server Menu** (also on the status bar item)

## Development

```sh
cargo build --bin craw-lsp     # the integration tests use target/debug/craw-lsp
cd editors/vscode
npm test                       # unit, manifest, activation, grammar, integration
npm run typecheck && npm run lint
```

## Installation

1. Open the **Extensions** view in VS Code (`Ctrl+Shift+X`).
2. Search for "Craw".
3. Click **Install**.

*The VSIX is built per-platform (`scripts/build_extension.sh`/`.ps1`) and
bundles the matching `craw-lsp` binary.*

## License

MIT License. See [LICENSE](LICENSE) for details.
