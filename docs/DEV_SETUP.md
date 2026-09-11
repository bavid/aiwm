# Dev-Setup (Windows)

Einmalige Einrichtung der Toolchain. Danach ist das Projekt offline baubar.

## Kurzform: `scripts/install.ps1`

```powershell
scripts/install.ps1              # Toolchain (winget) + build/sync + git hooks
scripts/install.ps1 -SkipWinget  # nur build/sync/hooks (Toolchain schon da)
```

Idempotent — jeder Schritt überspringt, was schon installiert ist; einzelne
fehlgeschlagene Toolchain-Installs (`-Optional`) brechen den Lauf nicht ab, nur
die Build-Schritte (`cargo build`/`pnpm install`) sind hart. Deckt Node.js,
Rust/rustup, uv, die MSVC-C++-Build-Tools, `rustup default stable`,
`uv python install 3.11`, pnpm (`corepack`, mit `npm install -g pnpm`-Fallback
falls corepack an Programm-Verzeichnis-Rechten scheitert — siehe unten), sowie
`cargo build` / `uv sync` / `pnpm install` / die Git-Hooks in einem Lauf ab.
**Nach einem frischen Node/Rust/uv-Install eine neue Shell öffnen und
erneut laufen lassen** (PATH).

Von Hand, Schritt für Schritt (was das Skript automatisiert):

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
cargo build                       # core + src-tauri
uv sync --directory sidecar       # Sidecar-venv
pnpm -C ui install                # UI-Abhängigkeiten
```

## Ausführen

```powershell
scripts/start.ps1              # Desktop-App, Dev-Modus (Kurzform)
scripts/start.ps1 -Install     # einmalig: Deps installieren, dann starten
scripts/start.ps1 -Headless    # nur Core + Loopback-API, kein UI-Fenster
```

Ohne das Skript, per Hand (aus `E:\AI` ausführen):

```powershell
cargo run -p aiwm-core --bin aiwm-cored   # headless core + Loopback-API, Ctrl-C beendet
.\ui\node_modules\.bin\tauri.cmd dev      # Desktop-App
```

**Nicht** `pnpm -C ui exec tauri dev` — das setzt das Arbeitsverzeichnis des
Kindprozesses auf `ui/`, und die Tauri-CLI sucht `src-tauri/tauri.conf.json`
nur in **Unterordnern** des cwd. Von `ui/` aus ist `src-tauri` aber ein
Geschwister-, kein Kindordner → Panic „Couldn't recognize the current folder
as a Tauri project". Die CLI direkt aus `E:\AI` starten (`src-tauri` liegt
dort als Unterordner); `tauri.conf.json`s `beforeDevCommand` (`pnpm dev`,
`cwd: "../ui"`) startet den Vite-Dev-Server unabhängig davon richtig.

Die Loopback-API läuft bei allen Varianten auf `http://127.0.0.1:48160`
(`GET /about /telemetry /jobs /runtimes /logs`, `GET /ws` Telemetrie-Stream).

**Datenablage ist portabel (ADR-026):** `config.toml`, `aiwm.db`, `logs/`,
generierte Bilder/Videos, verwaltete Laufzeit-Installationen (llama.cpp,
ComfyUI) und der Registry-Cache landen standardmäßig unter `<repo>\data\` —
läuft die App aus `E:\AI`, bleibt alles auf `E:`, kein `%APPDATA%`. Override
für alles zusammen: `AIWM_DATA_DIR`. Einzeln überschreibbar (Settings → „Data
locations" oder `config.toml`s `[paths]`-Tabelle): `outputs_path`,
`runtimes_path`, `cache_path`.

## Quality Gate

```powershell
powershell -ExecutionPolicy Bypass -File scripts/check.ps1
```

Läuft fmt / clippy / test für Rust, typecheck / lint für die UI, ruff / pytest
für den Sidecar. Schritte, deren Toolchain fehlt, werden übersprungen.
(`pwsh` / PowerShell 7 ist nicht installiert — Windows PowerShell 5.1 genügt.)

## Git-Hooks

`git config core.hooksPath scripts/githooks` (einmalig; für frische Clones nötig
— `scripts/install.ps1` setzt es mit).

- **pre-commit** — `cargo fmt --check` (instant)
- **pre-push** — vollständiges `scripts/check.ps1`

## CI

`.github/workflows/ci.yml` fährt denselben Gate auf `windows-latest` — greift nur,
falls das Repo je ein GitHub-Remote bekommt.

## Installierte Versionen (Stand WP-0)

| Werkzeug | Version | Bezug |
|---|---|---|
| Rust (rustc/cargo) | 1.98.1 | `winget install Rustlang.Rustup`, gepinnt via `rust-toolchain.toml` |
| MSVC C++ | vorhanden | war bereits über VS 2022 installiert (`cargo test` linkt) |
| Node.js | 24.19.0 LTS | `winget install OpenJS.NodeJS.LTS` (löst UAC aus) |
| pnpm | 12.3.4 | `npm install -g pnpm` (corepack scheiterte an Program-Files-Rechten) |
| uv | 0.12.10 | `winget install astral-sh.uv`; Python 3.11.11 von uv verwaltet |
| git / WebView2 / winget | 2.47 / vorhanden / 1.29 | vorinstalliert |

Hinweise:
- pnpm 12 blockt Dependency-Build-Skripte -> `ui/pnpm-workspace.yaml` erlaubt
  gezielt `esbuild`.
- pnpm 12 hat eine Supply-Chain-Policy (Mindest-Release-Alter). Sehr frische
  Paketversionen ggf. um einen Tag zurücksetzen (z. B. `typescript-eslint` auf
  8.69.0 gepinnt).
