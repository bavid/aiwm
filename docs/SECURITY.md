# Security

Sicherheitsmodell des Tools. Kontext: **Single-User, lokale Workstation,
Loopback-only.** Kein öffentlicher Dienst.

## Grundsätze

1. **Loopback-only.** Core-API und alle Runtimes binden `127.0.0.1` (ADR-008).
   Kein LAN-Listener, keine Auth im MVP (es gibt nichts Fernzugreifbares).
2. **Datenhoheit.** Fotos, Videos, Code, Repos, Prompts, Agent-Memory, Job-History
   bleiben lokal. Kein Cloud-Provider im Default-Pfad; Cloud nur pro Aktion mit
   Bestätigung (ADR-009).
3. **Kein Telemetrie-Callback, kein Account-Zwang** — weder im Tool noch (soweit
   möglich) in den integrierten Komponenten.
4. **Null `unsafe`** im `core`-Produktivcode (Stand Phase 1). Der einzige FFI-Aufruf
   ist in einem Integrationstest (`daemon_shutdown.rs`, dokumentiert).

## Prozess-Isolation

- Alle Kindprozesse (Runtimes, Sidecar) laufen in einem **Windows Job Object**
  mit `KILL_ON_JOB_CLOSE` + `kill_on_drop` → keine verwaisten Prozesse.
- Race-Fenster `CreateProcess` → `AssignProcessToJobObject`: aktuell sofortige
  Zuweisung (für llama-server/ComfyUI vernachlässigbar), CREATE_SUSPENDED später
  ([TODO.md](TODO.md)).

## Modell-Integrität

- Bevorzugt `.safetensors` / `.gguf` (reine Daten). Pickle-`.bin` → Warnung.
- **Custom ComfyUI-Nodes = beliebiger Python-Code** → nie automatische
  Installation; nur eine getestete, festgepinnte Menge (Phase 3).
- SHA256-Verifikation bei Downloads (Phase 6). Echte Signaturen gibt es kaum.
- Unbekannte Quellen: „⚠ Unverified" anzeigen, nicht automatisch ausführen.

## Agent-Sandbox (Phase 5, ADR-010 / ADR-021)

Coding-/Agent-Runtimes können Dateien lesen/schreiben und Shell-Kommandos
ausführen. **Der MVP-Sandkasten ist config-level, nicht prozess-level** —
umgesetzt über die *erzwungene* Runtime-Config (5.1b/5.2), nicht über
OS-Isolation. Für OpenCode (`agent::opencode::forced_config`, via
`OPENCODE_CONFIG_CONTENT`, das merged und gewinnt):

- **Pfad-Allowlist.** `cwd` = Workspace. `permission.edit`/`.write` =
  `{ "*": "deny", "<workspace>/**": "ask" }` (jeder Edit fragt, außerhalb gar
  nicht). `permission.external_directory` = `{ "*": "deny", "<extra>/**":
  "allow" }` — die Profil-Extra-Roots sind **read-only** (edit verbietet sie
  weiter).
- **Command-Approval.** `permission.bash = "ask"` — jedes Shell-Kommando geht
  als `permission`-Event durch den Core an die UI; deren Antwort
  (`allow_once` / `allow_always` / `deny`) wird zurück-`POST`et.
- **Kein Netz.** `permission.webfetch`/`.websearch = "deny"` **und**
  `tools.webfetch`/`.websearch = false`. `enabled_providers: ["local"]` +
  `disabled_providers: [alle Cloud]` → nur der lokale `llama-server`. Gilt
  **immer** für MVP-Agenten, unabhängig vom globalen `offline_mode`.
- **Secrets nicht in der Agent-Env.** Der `opencode`-Kindprozess wird über
  `SpawnSpec.env_remove` von bekannten Cloud-Credentials befreit
  (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `AWS_*`, `GITHUB_TOKEN`, `HF_TOKEN`,
  `OPENCODE_CONFIG`, … — Liste `SCRUBBED_ENV`); der `apiKey` im Provider ist ein
  Dummy (`aiwm-local`).
- **Vertagt (opt-in, eigener ADR):** echte FS-/Prozess-Isolation (WSL2,
  Container, AppContainer, Firewall-Regel pro Agent), Toolset-Whitelisting pro
  Profil, per-Kommando-Bash-Deny-Muster.

## Config / Secrets

- `config.toml` unter `%APPDATA%`, keine Secrets darin.
- `.env` ist in `.gitignore`.
- Erforderliche Werte werden beim Start validiert (`Config::validate`), kaputte
  `AIWM_*`-Overrides sind harte Startfehler (kein stilles Fallback).

## Offene Punkte

- App-Selbst-Update offline (manueller Installer + Signaturprüfung angenommen)
- `cargo audit` / `cargo deny` in die CI aufnehmen
- Firewall-Dialog beim ersten Start der gebündelten Runtimes dokumentieren
