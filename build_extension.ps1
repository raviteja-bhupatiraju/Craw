$ErrorActionPreference = "Stop"

# Configuration
$ExtDir = "editors/vscode"
$VsixName = "craw-0.1.0.vsix"

Write-Host "Checking for LSP binaries..."
if (-not (Test-Path "$ExtDir/bin/craw-lsp") -or -not (Test-Path "$ExtDir/bin/craw-lsp.exe")) {
    Write-Host "Error: LSP binaries not found in $ExtDir/bin/"
    Write-Host "Please build the Rust LSP first."
    exit 1
}

Write-Host "Moving to $ExtDir..."
Push-Location $ExtDir
try {
    if (-not (Test-Path "node_modules")) {
        Write-Host "Installing dependencies..."
        npm install
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

    Write-Host "Compiling TypeScript..."
    npm run compile
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "Packaging extension..."
    npx @vscode/vsce package --out "../../$VsixName"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "Success! Extension packaged as $VsixName in root."
}
finally {
    Pop-Location
}
