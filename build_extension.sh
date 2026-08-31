#!/bin/bash
set -e

# Configuration
EXT_DIR="editors/vscode"
VSIX_NAME="craw-0.1.0.vsix"

echo "Checking for LSP binaries..."
if [ ! -f "$EXT_DIR/bin/craw-lsp" ] || [ ! -f "$EXT_DIR/bin/craw-lsp.exe" ]; then
    echo "Error: LSP binaries not found in $EXT_DIR/bin/"
    echo "Please build the Rust LSP first."
    exit 1
fi

echo "Moving to $EXT_DIR..."
cd "$EXT_DIR"

if [ ! -d "node_modules" ]; then
    echo "Installing dependencies..."
    npm install
fi

echo "Compiling TypeScript..."
npm run compile

echo "Packaging extension..."
# Use npx to ensure vsce is available
npx @vscode/vsce package --out "../../$VSIX_NAME"

echo "Success! Extension packaged as $VSIX_NAME in root."
