$ErrorActionPreference = "Stop"

Set-Location (Join-Path $PSScriptRoot "..")

# Target platform for `vsce package --target`, e.g. win32-x64, linux-x64,
# darwin-x64, darwin-arm64. Defaults to a universal package for the host.
# -Install runs `code --install-extension` on the resulting VSIX afterward.
$Target = $null
$Install = $false
foreach ($arg in $args) {
    if ($arg -eq "--install" -or $arg -eq "-Install") {
        $Install = $true
    } else {
        $Target = $arg
    }
}

# The bundled craw-lsp is always built for the host, so a --target for any
# other platform would ship a binary that can't run there.
if ($Target) {
    $RuntimeInfo = [System.Runtime.InteropServices.RuntimeInformation]
    $OSPlatform = [System.Runtime.InteropServices.OSPlatform]
    if ($RuntimeInfo::IsOSPlatform($OSPlatform::Windows)) { $HostOs = "win32" }
    elseif ($RuntimeInfo::IsOSPlatform($OSPlatform::OSX)) { $HostOs = "darwin" }
    elseif (Test-Path "/etc/alpine-release") { $HostOs = "alpine" }
    else { $HostOs = "linux" }
    $HostArch = switch ($RuntimeInfo::OSArchitecture.ToString()) {
        "X64" { "x64" }
        "Arm64" { "arm64" }
        "Arm" { "armhf" }
        default { $_.ToLower() }
    }
    $HostTarget = "$HostOs-$HostArch"
    if ($Target -ne $HostTarget) {
        Write-Error "Target '$Target' doesn't match this host ($HostTarget); craw-lsp can only be bundled for the platform it's built on."
        exit 1
    }
}

Write-Host "Building craw-lsp (release)..."
cargo build --release --bin craw-lsp
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Clean bin/ first so a stale binary for another platform can never
# silently ride along in the VSIX.
Remove-Item -Recurse -Force "editors/vscode/bin" -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path "editors/vscode/bin" | Out-Null
Copy-Item "target/release/craw-lsp" "editors/vscode/bin/" -ErrorAction SilentlyContinue
Copy-Item "target/release/craw-lsp.exe" "editors/vscode/bin/" -ErrorAction SilentlyContinue

Push-Location "editors/vscode"
try {
    if (-not (Test-Path "node_modules")) {
        Write-Host "Installing extension dependencies..."
        npm ci
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }

    Write-Host "Compiling extension..."
    npm run compile
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "Packaging extension..."
    if ($Target) {
        $VsixName = "craw-$Target.vsix"
        npx @vscode/vsce package --target $Target --out "../../$VsixName"
    } else {
        $VsixName = "craw.vsix"
        npx @vscode/vsce package --out "../../$VsixName"
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "Success! Extension packaged as $VsixName in repo root."

    if ($Install) {
        if (-not (Get-Command code -ErrorAction SilentlyContinue)) {
            Write-Error "--install requested but 'code' is not on PATH."
            exit 1
        }
        Write-Host "Installing $VsixName..."
        code --install-extension "../../$VsixName" --force
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
}
finally {
    Pop-Location
}
