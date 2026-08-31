Write-Host "Running samples/showcase.craw..."
cargo run --bin craw -- run "samples/showcase.craw"
if ($LASTEXITCODE -ne 0) {
    Write-Host "FAIL: samples/showcase.craw"
    exit 1
} else {
    Write-Host "SUCCESS: samples/showcase.craw"
}
