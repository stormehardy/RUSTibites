# RUSTibites integration gate.
#
# Run before declaring a milestone or merging a subsystem:
#   powershell -ExecutionPolicy Bypass -File scripts\check.ps1
#
# The same three commands are the project's CI contract (ARCHITECTURE §12).

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "== cargo fmt --check ==" -ForegroundColor Cyan
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== cargo clippy (deny warnings) ==" -ForegroundColor Cyan
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== cargo test ==" -ForegroundColor Cyan
cargo test --workspace
exit $LASTEXITCODE
