$ErrorActionPreference = "Stop"

# Build the compiler
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Run the comprehensive feature test
Write-Host "Running feature_test.craw..."
cargo run --release --bin craw -- run tests/feature_test.craw
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Run the master showcase
Write-Host "`nRunning master_showcase.craw..."
cargo run --release --bin craw -- run samples/master_showcase.craw
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "`nEverything verified!"
