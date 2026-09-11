#!/usr/bin/env pwsh
# Starts AI Workstation Manager.
#
# Default: the Tauri desktop app in dev mode (embeds aiwm-core — same
# Loopback-API as -Headless, on :48160).
# -Headless: just the core + Loopback-API (aiwm-cored), no UI window — for
# curl/API work or when the desktop shell isn't needed.
# -Install: run the one-time dependency installs first (ui deps + sidecar venv).
#
# Usage:
#   scripts/start.ps1                 # desktop app, dev mode
#   scripts/start.ps1 -Headless       # core + API only
#   scripts/start.ps1 -Install        # install deps, then start the desktop app

param(
    [switch]$Headless,
    [switch]$Install
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root

function Has([string]$cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

try {
    if (-not (Has 'cargo')) {
        throw "cargo not found on PATH -- see docs/DEV_SETUP.md"
    }

    if ($Install) {
        Write-Host "`n=== installing dependencies ===" -ForegroundColor Cyan
        if (Has 'pnpm') {
            pnpm -C ui install
        } else {
            Write-Host 'skip: pnpm not installed' -ForegroundColor Yellow
        }
        if ((Test-Path "$root/sidecar/pyproject.toml") -and (Has 'uv')) {
            uv sync --directory sidecar
        } else {
            Write-Host 'skip: sidecar (no pyproject.toml or uv)' -ForegroundColor Yellow
        }
    }

    if ($Headless) {
        Write-Host "`n=== aiwm-cored (headless core + Loopback-API) ===" -ForegroundColor Cyan
        Write-Host "API on http://127.0.0.1:48160 -- Ctrl-C to stop`n" -ForegroundColor DarkGray
        cargo run -p aiwm-core --bin aiwm-cored
    } else {
        if (-not (Has 'pnpm')) {
            throw "pnpm not found on PATH -- see docs/DEV_SETUP.md"
        }
        if (-not (Test-Path "$root/ui/node_modules")) {
            Write-Host "ui/node_modules missing -- run 'scripts/start.ps1 -Install' first" -ForegroundColor Yellow
            exit 1
        }
        Write-Host "`n=== AI Workstation Manager (Tauri dev) ===" -ForegroundColor Cyan
        Write-Host "API on http://127.0.0.1:48160 -- close the window to stop`n" -ForegroundColor DarkGray
        pnpm -C ui exec tauri dev
    }
}
finally {
    Pop-Location
}
