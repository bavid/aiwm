# Phase 5 — Agents

Die vierte Capability, aber anders als 2–4: kein einzelner Job, sondern eine
**langlaufende Session** mit vielen Turns, Werkzeugen (Shell, Datei-Edits,
Suche) und einem Arbeitsverzeichnis. Wir **bauen keinen Agenten** — wir
orchestrieren und **begrenzen** zwei fertige, offline-fähige Open-Source-Agents
und geben ihnen einen lokalen `llama-server`-Endpoint.

Die Maschinerie trägt: `RuntimeSupervisor` + Windows Job Object (Prozess-
Lebenszyklus), der `HybridScheduler` kennt `is_agent_session` schon (pinnt das
Modell, blockt konkurrierende Jobs mit Klartext — R8), das loopback-API-Muster,
`offline_mode` (ADR-009). Phase 5 fügt hinzu: `core::agent` + `AgentAdapter`,
zwei Adapter, Sandkasten-Grenzen (Pfad-Allowlist + Command-Approval + erzwungene
Offline-Config), ein Agents-UI mit Approval-Prompts, Session-Persistenz + Backup.

**Der Sandkasten ist die harte Anforderung.** Ein Coding-Agent führt Shell-
Kommandos aus und schreibt Dateien. MVP-Niveau (ADR-010): **Pfad-Allowlist**
(Arbeitsverzeichnis, rekursiv), **Command-Approval** (Mensch bestätigt im UI,
lernbare „always allow"), **erzwungene Config** (Endpoint = lokaler
`llama-server`, alle anderen Provider aus, Netz-Tools aus). Echte Prozess-/FS-
Isolation (WSL2 / Container / Firewall-Regel) ist **opt-in und später**.

**Phase-5-DONE-Kriterium:** frisches Windows → App → „Set up" für den Agenten
(uv + gepinnte Version) → Coding-Modell importieren (`coding`-Rolle) → Agents-Tab
→ Workspace-Ordner wählen → „New session" → Aufgabe stellen („füge einen Test
für X hinzu") → der Agent liest Dateien, schlägt einen Shell-Befehl vor → UI
fragt **Allow / Deny** → nach Freigabe editiert er + führt die Tests aus →
Transkript zeigt Tool-Calls + Diffs → Scheduler pinnt das Modell (ein
Bild-/Chat-Job dazwischen → `blocked` „Agent pausieren?") → Netz trennen → alles
läuft weiter → App neu starten → Session per Checkpoint wieder aufnehmen →
Export/Import bringt Config + DB + Agent-Profil auf eine zweite Maschine.

| Scheibe | Inhalt | Status |
|---|---|---|
| **5.0** | **Windows-Voraussetzungen verproben** (Phase-4-`bash -l`-TODO): auf der echten Maschine — (a) `uv` da (aus Phase 3); (b) `opencode` installieren + `opencode serve` startet, `/doc` erreichbar; (c) Hermes' `bash -l`-Env-Snapshot: Git-Bash mitliefern **oder** WSL2 dokumentieren — entscheiden; (d) **`llama-server`-Tool-Calling**: `--jinja` + ein Coding-Modell (Qwen2.5-Coder o. Ä.) — funktionieren `tool_calls` im `/v1/chat/completions`? Welche Chat-Template? Ergebnis + Fixes dokumentieren. **Voraussetzung.** | offen |
| 5.1 | **Agent-Framework + OpenCode-Adapter**: `core::agent` (`AgentAdapter`-Trait, `AgentProfile`, `AgentSession`), Migration `0005` (`agents`, `agent_sessions`). `OpenCodeAdapter` — `opencode serve --port … --hostname 127.0.0.1` unter `RuntimeSupervisor`; Config **erzwungen** über `OPENCODE_CONFIG_CONTENT` (custom provider → `http://127.0.0.1:<llama-port>/v1`, `enabled_providers: [local]`, `permission` alles `ask`, `webfetch: deny`), `cwd` = Workspace. Scheduler: `reserve + pin` fürs Session-Modell (`coding`-Rolle \| explizit). HTTP-Proxy: `POST /session`, `POST /session/:id/message`, `GET /global/event` (SSE) → Core-Events. `POST /agents`, `POST /agent-sessions`, `POST /agent-sessions/:id/message`, `GET /agent-sessions/:id` + Tauri-Commands. `capability::agent` treibt eine Session. | offen |
| 5.2 | **Sandkasten**: Approval-Fluss — der SSE-Stream trägt `permission`-Events → Core reicht sie durch → UI „Agent will ausführen: `<cmd>` — Allow / Deny / Always" → Core `POST`t die Entscheidung. **Pfad-Allowlist**: `cwd` + OpenCode-`permission` verbietet Edits außerhalb; Lese-Extras optional read-only. **Offline erzwungen**: `offline_mode` → Netz-Tools hart aus, dokumentiert dass echte Prozess-Isolation vertagt ist. Secrets: kein `.env` / keine Keys in der Agent-Env (Dummy-`apiKey`). | offen |
| 5.3 | **Agents-UI-Tab**: Profil-Liste + „New profile" (Runtime, Modell [Auto über `coding`], Workspace-Ordner-Picker, erlaubte Pfade, Toolset); Session-Ansicht — Transkript mit Tool-Call-Karten (Datei-Diffs, Shell-Output), Approval-Prompts inline, „Stop"; „Coding"-Dashboard-Button aktiv. Workspace-Registry (Pfad + Label) im Core. | offen |
| 5.4 | **Hermes-Agent-Adapter**: `uv`-Installer (gepinnte Version) → gemanagtes Profil unter `<data>/agents/hermes/` (`HERMES_HOME`/`-p <profil>`); `config.yaml` **erzwungen** (`provider: custom`, `base_url` = `llama-server`, `security.redact_secrets`, Workspace, `HERMES_STREAM_READ_TIMEOUT=1800`). Treiben über Hermes' HTTP-Server (`/v1/responses` / `/api/jobs`) **oder** `hermes chat -q` pro Turn. Memory (`MEMORY.md`) + Skills leben im Profil-Ordner → Backup (5.5). `bash -l` per 5.0-Entscheidung. | offen |
| 5.5 | **Session-Persistenz + Backup/Restore**: `agent_sessions.checkpoint_path`, Wiederaufnahme nach Absturz/Neustart (OpenCode `/session` list + resume, Hermes `/resume`); Kontext-Kompaktierung: die Agents machen sie selbst, wir zeigen sie an. **Export/Import** — ein Archiv aus `config.toml` + `aiwm.db` + `<data>/agents/<profil>/` (+ Modell-Manifest, **nicht** die Modell-Dateien). Politur: Pin/Unpin-Lebenszyklus, „Agent pausieren?"-Fluss (R8), Diagnostics-Zeile pro Agent. | offen |

Danach (Post-MVP): `aider` als optionaler dritter Adapter, lokaler Repository-
Index (Embeddings über ein kleines lokales Modell) für Retrieval, MCP-Server aus
dem Tool heraus verwalten, mehrere parallele Sessions.

---

## Research (2026-09)

### OpenCode — der schlanke Coding-Agent

- **`opencode serve`** — headless HTTP-Server (`--port 4096`, `--hostname
  127.0.0.1`, `--cors`), OpenAPI unter `/doc`, JS-SDK `@opencode-ai/sdk`.
  Auth: `OPENCODE_SERVER_PASSWORD` (User `opencode`).
- **API:** `POST /session` (neu), `POST /session/:id/message` (Prompt + warten),
  `GET /session/:id/message` (Liste), `POST /session/:id/abort`,
  `GET /global/event` (SSE-Stream — Tool-Calls, Permission-Requests).
- **Config** (`opencode.json`/JSONC, **wird gemerged, nicht ersetzt** →
  `OPENCODE_CONFIG_CONTENT`-Env erzwingt Werte): `provider.<name>.baseURL` +
  `options.apiKey` für einen OpenAI-kompatiblen lokalen Endpoint;
  `enabled_providers` / `disabled_providers`; `model: "<name>/<model>"`.
- **Permissions:** `permission: { "*": "allow", "bash": "ask", "edit": "ask",
  "webfetch": "deny" }`, Bash-Muster `{ "bash": { "*": "ask", "rm -rf *":
  "deny" } }`; `tools` schaltet Fähigkeiten ab.
- Läuft nativ auf Windows, kein `bash -l` nötig. **→ erster Adapter (5.1).**

### Hermes Agent (Nous Research) — „der Agent, der mit dir wächst"

- Python 3.11+, `uv`-Installer, **keine Telemetrie**. v0.14.0 (Source) /
  0.13.0 (PyPI), aktiv gepflegt (github.com/NousResearch/hermes-agent, MIT).
- **Modi:** persistenter HTTP-Server (`/v1/chat/completions`, `/v1/responses`,
  `/api/jobs`-REST für Cron); TUI; `hermes chat -q` (single-shot);
  `hermes gateway` (Messaging-Bridge — brauchen wir **nicht**).
- **Config:** `~/.hermes/config.yaml` (YAML), `.env`, `profiles/<name>/`
  (isoliert: config + memory + sessions + skills). `model: { default:,
  provider: custom, base_url: }`. Lokaler Endpoint: `hermes model` → „Custom
  endpoint" (llama.cpp / vLLM / Ollama / SGLang).
- **Sessions** in SQLite, `parent_session_id`-Kette, FTS5-Suche, `/resume`
  (v0.5.0). **Memory:** `MEMORY.md`, agent-kuratiert. **Skills:** nach
  komplexen Aufgaben (5+ Tool-Calls) selbst erzeugt, selbst-verbessernd;
  `hermes curator` konsolidiert.
- **Sandkasten:** blockt destruktive Kommandos + fragt; „smart approvals"
  lernen sichere Kommandos; Workspace-Constraints (`@file`/`@url` nur im
  Workspace, Secrets außerhalb blockiert); `security.redact_secrets`,
  `privacy.redact_pii`; `/permission [mode]`.
- **Windows:** WSL2 **oder** Git-Bash für den `bash -l`-Env-Snapshot;
  `terminal.shell_init_files` konfigurierbar. `HERMES_STREAM_READ_TIMEOUT=1800`
  für lokale Endpoints. **→ zweiter Adapter (5.4), nach 5.0.**

Quellen: [OpenCode Server](https://opencode.ai/docs/server) ·
[OpenCode Config](https://opencode.ai/docs/config) ·
[hermes-agent](https://github.com/NousResearch/hermes-agent) ·
[Hermes FAQ](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/reference/faq.md) ·
[hermes-agent-docs (mudrii)](https://github.com/mudrii/hermes-agent-docs).

### Wiederverwendung aus Phase 1–4

- `RuntimeSupervisor` + Job Object → die Agent-Server als Kindprozesse, sterben
  mit der App. `free_loopback_port`, `await_healthy`.
- `HybridScheduler`: `PlanRequest.is_agent_session` pinnt das Modell schon;
  `Decision::Blocked` mit „pause it or queue this job" ist da (R8, ADR-003).
- `runtime::download` + der `uv`-`CmdRunner` (Phase 3.2) → Hermes-Installer.
- `capability`-Muster (was passiert, während eine Session läuft) → `capability::agent`.
- `job_events`-Muster → `agent_session_events` (Transkript + Tool-Calls).
- `config.toml` + `[…]`-Tabellen (ADR-017) → `[agents]` (Timeouts, Default-
  Toolset). `offline_mode` (ADR-009) → erzwungene Agent-Offline-Config.

---

## Offene Entscheidungen (Empfehlung → deine Freigabe)

**A — Adapter-Reihenfolge.**
→ **Empfehlung: OpenCode zuerst (5.1), Hermes zweiter (5.4)** — dreht die
ROADMAP-Reihenfolge um. OpenCodes `serve`-API ist voll dokumentiert, die Config
per Env erzwingbar, kein `bash -l`. Das bringt schneller einen **funktionierenden,
gesandboxten, offline** Agenten und entkoppelt das Windows-Shell-Risiko von
Hermes vom Rest der Phase. ADR-010s „beide Adapter" bleibt.

**B — Sandkasten-Niveau (MVP).**
→ **Empfehlung: Pfad-Allowlist (cwd, rekursiv) + Command-Approval (Mensch im UI,
lernbare „always allow") + erzwungene Config (nur lokaler Endpoint, Netz-Tools
aus).** Kein WSL2/Container/Firewall-Regel im MVP — das ist opt-in und später
(eigener ADR). Deckt sich mit ADR-010; hier nur bestätigen.

**C — Workspace-Konzept.**
→ **Empfehlung: eine Workspace-Registry im Core** (Pfad + Label, vom User
angelegt). Eine Session wählt einen Workspace; die Allowlist = dieser Pfad
rekursiv + optionale read-only-Extras. **Kein** Repository-Index im MVP — die
Agents lesen Dateien selbst; Retrieval-Index ist Post-MVP.

**D — Agent-Modell.**
→ **Empfehlung: `Auto` über die schon existierende `coding`-Rolle** (zuletzt/
meist genutzt), analog `chat`. Ein GGUF mit verlässlichem Tool-Calling
(Qwen2.5-Coder-7B/14B, o. Ä.) — kuratierte Liste als spätere `docs/AGENT_MODELS.md`.

**E — `llama-server`-Tool-Calling (Risiko).**
→ **In 5.0 verproben.** OpenCode/Hermes brauchen verlässliche `tool_calls` aus
`/v1/chat/completions`. `llama-server --jinja` + das richtige Chat-Template pro
Modell. Fällt es aus → GBNF-Grammar-Fallback / ein anderes Modell / OpenCodes
„native tool calling off". **Blocker-Kandidat** — deshalb eigene Scheibe 5.0.

**F — Backup-Umfang.**
→ **Empfehlung: `config.toml` + `aiwm.db` + `<data>/agents/<profil>/`** (die
gemanagten Agent-Configs/Memory/Skills, die wir besitzen) + ein **Modell-
Manifest** (Namen/SHAs, nicht die GB-Dateien). Ein `.zip`, versioniert.
`~/.hermes/` des Users bleibt außen vor (wir fahren ein eigenes Profil).

---

## Bewusst nicht in Phase 5

- **Echte Prozess-/FS-Isolation** (WSL2, Container, seccomp/AppContainer,
  Firewall-Regel pro Agent) — opt-in, eigener ADR, nach dem MVP.
- **Lokaler Repository-Index / Embeddings-Retrieval** — Post-MVP; die Agents
  machen ihr eigenes File-Reading.
- **`aider` als dritter Adapter**, **MCP-Server-Verwaltung** aus dem Tool,
  **mehrere parallele Sessions**, **Sub-Agent-Bäume im UI** — später.
- **Hermes „Gateway"** (Slack/Discord-Bridge), Hermes' bezahlte Portal-Tiers.
- **Nicht-lokale Endpoints** (OpenRouter etc.) — hart aus, kein Opt-in-Schalter
  im MVP.
- **Agent-Autostart beim App-Start** (`agents.autostart`-Spalte existiert, bleibt
  aber `false`) — erst wenn der Sandkasten steht.
- **Windows-Firewall-Automatik** — der Nutzer bestätigt den Dialog (wie bei den
  anderen Loopback-Runtimes).
