#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Target platform for `vsce package --target`, e.g. linux-x64, win32-x64,
# darwin-x64, darwin-arm64. Defaults to a universal (no --target) package
# built for the host platform. --install runs `code --install-extension`
# on the resulting VSIX afterward (order doesn't matter: both
# `build_extension.sh --install` and `build_extension.sh linux-x64
# --install` work).
TARGET=""
INSTALL=false
for arg in "$@"; do
    case "$arg" in
        --install) INSTALL=true ;;
        *) TARGET="$arg" ;;
    esac
done

# The bundled craw-lsp is always built for the host, so a --target for any
# other platform would ship a binary that can't run there.
if [ -n "$TARGET" ]; then
    case "$(uname -s)" in
        Linux) [ -f /etc/alpine-release ] && HOST_OS=alpine || HOST_OS=linux ;;
        Darwin) HOST_OS=darwin ;;
        MINGW* | MSYS* | CYGWIN*) HOST_OS=win32 ;;
        *) HOST_OS="$(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64 | amd64) HOST_ARCH=x64 ;;
        aarch64 | arm64) HOST_ARCH=arm64 ;;
        armv7l | armv7*) HOST_ARCH=armhf ;;
        *) HOST_ARCH="$(uname -m)" ;;
    esac
    if [ "$TARGET" != "$HOST_OS-$HOST_ARCH" ]; then
        echo "Error: target '$TARGET' doesn't match this host ($HOST_OS-$HOST_ARCH); craw-lsp can only be bundled for the platform it's built on." >&2
        exit 1
    fi
fi

echo "Building craw-lsp (release)..."
cargo build --release --bin craw-lsp

# Clean bin/ first so a stale binary for another platform (e.g. left over
# from an earlier manual copy) can never silently ride along in the VSIX.
rm -rf editors/vscode/bin
mkdir -p editors/vscode/bin
cp target/release/craw-lsp editors/vscode/bin/ 2>/dev/null || true
cp target/release/craw-lsp.exe editors/vscode/bin/ 2>/dev/null || true

cd editors/vscode

if [ ! -d node_modules ]; then
    echo "Installing extension dependencies..."
    npm ci
fi

echo "Compiling extension..."
npm run compile

echo "Packaging extension..."
if [ -n "$TARGET" ]; then
    VSIX_NAME="craw-$TARGET.vsix"
else
    VSIX_NAME="craw.vsix"
fi
VSIX_PATH="../../$VSIX_NAME"
if [ -n "$TARGET" ]; then
    npx @vscode/vsce package --target "$TARGET" --out "$VSIX_PATH"
else
    npx @vscode/vsce package --out "$VSIX_PATH"
fi
echo "Success! Extension packaged as $VSIX_NAME in repo root."

if [ "$INSTALL" = true ]; then
    if ! command -v code >/dev/null 2>&1; then
        echo "Error: --install requested but 'code' is not on PATH." >&2
        exit 1
    fi
    echo "Installing $VSIX_NAME..."
    code --install-extension "$VSIX_PATH" --force
fi
