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

## Agent-Sandbox (Phase 5)

Coding-/Agent-Runtimes können Dateien lesen/schreiben und Shell-Kommandos
ausführen. MVP-Niveau der Grenzen:

- Workspace-Pfad-Allowlist (Agent sieht nicht den ganzen PC)
- Command-Approval für Shell-Aufrufe
- kein Netzzugriff per Default
- Secrets nicht in der Agent-Umgebung
- echte FS-/Prozess-Isolation (WSL2/Container) optional, später

## Config / Secrets

- `config.toml` unter `%APPDATA%`, keine Secrets darin.
- `.env` ist in `.gitignore`.
- Erforderliche Werte werden beim Start validiert (`Config::validate`), kaputte
  `AIWM_*`-Overrides sind harte Startfehler (kein stilles Fallback).

## Offene Punkte

- App-Selbst-Update offline (manueller Installer + Signaturprüfung angenommen)
- `cargo audit` / `cargo deny` in die CI aufnehmen
- Firewall-Dialog beim ersten Start der gebündelten Runtimes dokumentieren
