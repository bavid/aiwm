# Dev-Setup (Windows)

Einmalige Einrichtung der Toolchain. Danach ist das Projekt offline baubar.

## Voraussetzungen installieren

```powershell
# 1) MSVC C++ Build Tools (C++-Workload zu den vorhandenen VS Build Tools 2022)
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
#    Alternative: "Visual Studio Installer" -> Build Tools 2022 -> Ändern ->
#    "Desktopentwicklung mit C++" anhaken.

# 2) Rust (MSVC-Toolchain)
winget install --id Rustlang.Rustup

# 3) pnpm über corepack (bereits vorhanden)
corepack enable pnpm
corepack prepare pnpm@latest --activate

# 4) uv (managt auch Python 3.11 für den Sidecar)
winget install --id astral-sh.uv
```

Neue Shell öffnen, dann:

```powershell
rustup default stable
uv python install 3.11
```

## Verifizieren

```powershell
rustc --version      # >= 1.80
cargo --version
pnpm --version
uv --version
node --version       # v20+ ok
```

## Bauen

```powershell
cargo build                       # core + (später) src-tauri
uv sync --directory sidecar       # Sidecar-venv
pnpm -C ui install                # sobald ui/ existiert (nächster WP-0-Schritt)
```

## Quality Gate

```powershell
pwsh scripts/check.ps1
```

Läuft fmt / clippy / test für Rust, typecheck / lint für die UI, ruff / pytest
für den Sidecar. Schritte, deren Toolchain fehlt, werden übersprungen.

## Bereits vorhanden auf dieser Maschine

git 2.47 · Node v20.11 · corepack 0.23 · WebView2 · winget 1.29 · Python 3.10
(Sidecar nutzt via uv ein separates 3.11).
