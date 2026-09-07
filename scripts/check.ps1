#!/usr/bin/env pwsh
# Quality gate. Run before every commit; CI runs the same steps.
# Skips UI / sidecar steps when their toolchain or folder is not present yet.

$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$failed = @()

function Step([string]$name, [scriptblock]$body) {
    Write-Host "`n=== $name ===" -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) {
        Write-Host "FAILED: $name (exit $LASTEXITCODE)" -ForegroundColor Red
        $script:failed += $name
    }
}

function Has([string]$cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

if (Has 'cargo') {
    Step 'cargo fmt'    { cargo fmt --all -- --check }
    Step 'cargo clippy' { cargo clippy --workspace --all-targets -- -D warnings }
    Step 'cargo test'   { cargo test --workspace }
} else { Write-Host 'skip: cargo not installed' -ForegroundColor Yellow }

if ((Test-Path "$root/ui/package.json") -and (Has 'pnpm')) {
    Step 'ui typecheck' { pnpm -C ui run typecheck }
    Step 'ui lint'      { pnpm -C ui run lint }
} else { Write-Host 'skip: ui (no package.json or pnpm)' -ForegroundColor Yellow }

if ((Test-Path "$root/sidecar/pyproject.toml") -and (Has 'uv')) {
    Step 'sidecar ruff'   { uv run --directory sidecar ruff check . }
    Step 'sidecar pytest' { uv run --directory sidecar pytest }
} else { Write-Host 'skip: sidecar (no pyproject or uv)' -ForegroundColor Yellow }

Pop-Location
if ($failed.Count -gt 0) {
    Write-Host "`n$($failed.Count) step(s) failed: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host "`nAll checks passed." -ForegroundColor Green
