#!/usr/bin/env pwsh
# One-time dev-environment bootstrap for a fresh clone (Windows).
# Installs the toolchain (Node.js, Rust/rustup, uv, MSVC C++ Build Tools),
# then builds/syncs every workspace (core, ui, sidecar) and wires the git
# hooks. Safe to re-run -- every step skips what's already there.
#
# This mirrors docs/DEV_SETUP.md step by step; see that file for the manual
# version and the "why" behind each step.
#
# Usage:
#   scripts/install.ps1              # full bootstrap (toolchain + build + hooks)
#   scripts/install.ps1 -SkipWinget  # skip the winget/toolchain installs --
#                                     # just build, sync deps, wire git hooks
#                                     # (use once the toolchain is already there)

param(
    [switch]$SkipWinget
)

$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$failed = @()

function Step {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][scriptblock]$Body,
        [switch]$Optional
    )
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    & $Body
    if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) {
        if ($Optional) {
            Write-Host "skip: $Name failed (exit $LASTEXITCODE), continuing" -ForegroundColor Yellow
        } else {
            Write-Host "FAILED: $Name (exit $LASTEXITCODE)" -ForegroundColor Red
            $script:failed += $Name
        }
    }
}

function Has([string]$cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

# --- 1) toolchain (winget) --------------------------------------------------

if ($SkipWinget) {
    Write-Host 'skip: toolchain installs (-SkipWinget)' -ForegroundColor Yellow
} elseif (-not (Has 'winget')) {
    Write-Host 'winget not found -- install the toolchain by hand, see docs/DEV_SETUP.md' -ForegroundColor Yellow
} else {
    if (Has 'node') {
        Write-Host 'skip: Node.js already installed' -ForegroundColor Yellow
    } else {
        Step -Name 'winget: Node.js LTS' -Optional -Body {
            winget install --id OpenJS.NodeJS.LTS --accept-package-agreements --accept-source-agreements
        }
    }

    if (Has 'rustc') {
        Write-Host 'skip: Rust already installed' -ForegroundColor Yellow
    } else {
        Step -Name 'winget: Rust (rustup)' -Optional -Body {
            winget install --id Rustlang.Rustup --accept-package-agreements --accept-source-agreements
        }
    }

    if (Has 'uv') {
        Write-Host 'skip: uv already installed' -ForegroundColor Yellow
    } else {
        Step -Name 'winget: uv' -Optional -Body {
            winget install --id astral-sh.uv --accept-package-agreements --accept-source-agreements
        }
    }

    # Idempotent either way -- winget no-ops if the C++ workload is already there.
    Step -Name 'winget: MSVC C++ Build Tools (C++ workload)' -Optional -Body {
        winget install --id Microsoft.VisualStudio.2022.BuildTools --accept-package-agreements --accept-source-agreements `
            --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    }

    Write-Host "`nIf any of the above just installed for the first time, open a NEW shell" -ForegroundColor Yellow
    Write-Host "and re-run this script so PATH picks them up." -ForegroundColor Yellow
}

# --- 2) activate toolchains --------------------------------------------------

if (Has 'rustup') {
    Step -Name 'rustup default stable' -Optional -Body { rustup default stable }
}
if (Has 'uv') {
    Step -Name 'uv python install 3.11' -Optional -Body { uv python install 3.11 }
}

if (-not (Has 'pnpm')) {
    if (Has 'corepack') {
        Step -Name 'corepack: enable pnpm' -Optional -Body {
            corepack enable pnpm
            corepack prepare pnpm@latest --activate
        }
    }
    if ((-not (Has 'pnpm')) -and (Has 'npm')) {
        # corepack can be blocked by Program Files permissions (seen on WP-0) --
        # fall back to a plain global npm install.
        Write-Host 'corepack unavailable or blocked -- falling back to: npm install -g pnpm' -ForegroundColor Yellow
        Step -Name 'npm: install pnpm globally' -Optional -Body { npm install -g pnpm }
    }
} else {
    Write-Host 'skip: pnpm already installed' -ForegroundColor Yellow
}

# --- 3) build / sync every workspace -----------------------------------------

if (Has 'cargo') {
    Step -Name 'cargo build (core + src-tauri)' -Body { cargo build }
} else {
    Write-Host 'cargo not found -- open a new shell so PATH picks up rustup, then re-run' -ForegroundColor Red
    $failed += 'cargo build'
}

if ((Test-Path "$root/sidecar/pyproject.toml") -and (Has 'uv')) {
    Step -Name 'uv sync (sidecar)' -Body { uv sync --directory sidecar }
} else {
    Write-Host 'skip: sidecar (no pyproject.toml or uv not on PATH)' -ForegroundColor Yellow
}

if (Has 'pnpm') {
    Step -Name 'pnpm install (ui)' -Body { pnpm -C ui install }
} else {
    Write-Host 'pnpm not found -- open a new shell so PATH picks it up, then re-run' -ForegroundColor Red
    $failed += 'pnpm install'
}

# --- 4) git hooks -------------------------------------------------------------

if (Has 'git') {
    Step -Name 'git hooks (scripts/githooks)' -Body { git config core.hooksPath scripts/githooks }
}

# --- summary ------------------------------------------------------------------

Write-Host "`n=== versions ===" -ForegroundColor Cyan
foreach ($cmd in 'rustc', 'cargo', 'pnpm', 'uv', 'node') {
    if (Has $cmd) { & $cmd --version } else { Write-Host "$cmd -- not on PATH" -ForegroundColor Yellow }
}

Pop-Location
if ($failed.Count -gt 0) {
    Write-Host "`n$($failed.Count) step(s) failed: $($failed -join ', ')" -ForegroundColor Red
    Write-Host 'See docs/DEV_SETUP.md for the manual steps.' -ForegroundColor Red
    exit 1
}
Write-Host "`nReady. Try: scripts/start.ps1" -ForegroundColor Green
