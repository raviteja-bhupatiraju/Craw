# Changelog

## Unreleased

- **Modules**, with Python's import semantics: `import a.b` (binds `a`),
  `import a.b as m`, `import a, b`, `from m import x as y`, parenthesized
  and `*` imports (honoring `__all__`), relative imports (`from . import
  x`), packages (`__init__.craw`), and a namespace per module, types
  included. The editor follows imports for go to definition, hover,
  completion, signature help, inlay hints and semantic highlighting; find
  references and rename work across the workspace; broken imports are
  flagged as you type, and `from std.x import y` gets a quick fix to Rust's
  `use` form. **Show Generated Rust** links imported modules, including
  unsaved edits.
- Language server: semantic tokens, inlay hints (parameter names, inferred
  types, inferred return types), code actions (unused-name and typo quick
  fixes; add type annotation, extract variable, inline variable),
  token-preserving document/range formatting, folding ranges, selection
  ranges, call hierarchy, go to type definition, keyword hover, typed hover,
  richer completion and signature help, and `craw/syntaxTree` /
  `craw/transpile` requests.
- New commands: **Show Generated Rust**, **Show Syntax Tree** (live views);
  Run / Build / Show Rust code lenses.
- Untitled Craw documents get full language support.
- Run commands save dirty files first and run from the file's directory;
  untitled files prompt to save. The REPL opens in a real terminal.
- `craw.server.useWorkspaceBuild` now takes precedence over the bundled
  binary (it previously had no effect whenever a bundled binary existed).
- Path settings accept `${workspaceFolder}`, `${userHome}`, `${env:NAME}`
  and `~`.
- New settings: `craw.interpreter.path`, `craw.run.saveBeforeRun`,
  `craw.lens.run.enable`, `craw.inlayHints.*.enable`.
- The Run keybinding moved from `Ctrl+Shift+R` (VS Code's Refactor) to
  `Ctrl+Alt+R`. Editor-only commands are hidden from the palette outside
  Craw files.
- TextMate grammar regenerated from the lexer.
- Lexer fixes: Rust code on the same line as `rust:` (`rust: panic!(...)`)
  is kept as the block's first line instead of being silently discarded;
  a bare `!` is now a syntax error ("Craw negates with `not`") instead of
  vanishing. `!=` and `.!=` are unchanged.
- `craw.yaml` (micro editor syntax) now matches the lexer's keyword and
  operator set.
- Fixed `package.json` metadata (duplicate `publisher` key, real repository
  URL, categories, `engines.vscode` bumped to `^1.85.0`).
- Added `craw.server.path`, `craw.server.extraEnv`, `craw.server.useWorkspaceBuild`,
  `craw.cli.path`, and `craw.trace.server` settings.
- Added an output channel, a status bar item (starting/running/stopped, with
  a click-to-open server menu), and `craw.restartServer` / `craw.stopServer` /
  `craw.startServer` / `craw.showServerOutput` commands.
- Server discovery now checks `craw.server.path`, `$CRAW_LSP_PATH`, the
  bundled binary, and `PATH`, and shows a clear error with an "Open Settings"
  action instead of failing silently.
- Extension is now bundled with esbuild instead of shipping `node_modules`.
- `craw-lsp --version` for diagnosing which server build is running.
- Build scripts now build `craw-lsp` for the host platform before packaging.
- Added a 128×128 PNG icon (the Marketplace rejects SVG icons).

## 0.1.0

- Initial release: syntax highlighting, hover, go to definition, document
  symbols, completion, and syntax diagnostics.
