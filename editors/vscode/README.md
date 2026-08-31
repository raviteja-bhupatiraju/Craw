# Craw Language Support

The official Visual Studio Code extension for the **Craw** programming language.

## Features

- **Syntax Highlighting**: Full highlighting for Craw keywords, expressions, and structures.
- **Seamless Rust Integration**: Highlighting and LSP support for inline Rust code (`fn`, `struct`, `impl`, etc.) without special blocks.
- **Go to Definition**: Instantly jump to where a variable or function is defined.
- **Hover Information**: See type and signature info when hovering over symbols.
- **Document Symbols**: Navigate your code easily with the symbol tree (functions, classes, traits).
- **Auto-completion**: Intelligent suggestions for variables, keywords, and functions.
- **Diagnostics**: Real-time syntax error reporting.

## Seamless Rust Integration

Craw allows you to write standard Rust code directly in your files. The extension automatically detects and highlights these blocks:

```python
# Plain Rust function in a Craw file
fn add(a: i64, b: i64) -> i64 {
    a + b
}

def main():
    print("Result:", add(1, 2))
```

## Installation

1. Open the **Extensions** view in VS Code (`Ctrl+Shift+X`).
2. Search for "Craw".
3. Click **Install**.

*Note: The extension bundles a pre-compiled Language Server (LSP) binary for both Linux and Windows.*

## Requirements

- **Rust Toolchain**: Recommended for compiling the generated Rust code.
- **Craw CLI**: Used for transpiling and running `.craw` files.

## Extension Settings

- `craw.lsp.path`: (Optional) Specify a custom path to the `craw-lsp` binary.

## License

MIT License. See [LICENSE](LICENSE) for details.
