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
        # Invoke the tauri CLI binary directly with cwd = repo root, NOT via
        # `pnpm -C ui exec` (that sets the child process's cwd to ui/, and the
        # tauri CLI locates the project by finding `src-tauri/tauri.conf.json`
        # in a SUBFOLDER of cwd -- from ui/ that search fails since src-tauri
        # is ui's *sibling*, not its child: "Couldn't recognize the current
        # folder as a Tauri project"). tauri.conf.json's own
        # `build.beforeDevCommand` (`pnpm dev`, cwd `../ui`) still starts the
        # Vite dev server correctly regardless of where the CLI itself runs.
        $tauriBin = Join-Path $root 'ui\node_modules\.bin\tauri.cmd'
        if (-not (Test-Path $tauriBin)) {
            Write-Host "$tauriBin missing -- run 'scripts/start.ps1 -Install' first" -ForegroundColor Yellow
            exit 1
        }
        Write-Host "`n=== AI Workstation Manager (Tauri dev) ===" -ForegroundColor Cyan
        Write-Host "API on http://127.0.0.1:48160 -- close the window to stop`n" -ForegroundColor DarkGray
        & $tauriBin dev
    }
}
finally {
    Pop-Location
}
