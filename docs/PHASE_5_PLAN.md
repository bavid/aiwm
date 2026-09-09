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
| **5.0** | **Windows-Voraussetzungen verproben**: (a) `opencode` installieren + `opencode serve` + API; (b) Hermes installieren + `bash -l`-Frage klären; (c) **`llama-server`-Tool-Calling** (`--jinja` + Coding-Modell). Ergebnis + Fixes dokumentieren. **Voraussetzung.** | ✅ *(bis auf den echten Coding-Modell-Render — siehe „## 5.0 — Ergebnis" + ADR-021)* |
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

**A — Adapter-Reihenfolge.** → **✅ entschieden (ADR-021): OpenCode zuerst,
Hermes zweiter.** 5.0 hat es bestätigt — OpenCode = eine `.exe`, Config per Env
erzwingbar, Approval-Fluss verprobt; Hermes = 120 Deps + node/Browser/ripgrep/
ffmpeg. Dreht die ROADMAP-Reihenfolge um; ADR-010s „beide Adapter" bleibt.

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
→ **✅ weitgehend entschärft (5.0 / ADR-021):** `llama-server --jinja` emittiert
Standard-OpenAI-`tool_calls`, die OpenCodes `@ai-sdk/openai-compatible`-Runtime
parst (Stub-Test bewiesen); nativ für Qwen 2.5 Coder / Hermes / Llama 3.x.
`LlamaCppAdapter` braucht dafür ein `--jinja`-Flag (5.1). **Offen:** ein echter
Coding-Modell-Lauf → 5.1-Smoke.

**F — Backup-Umfang.**
→ **Empfehlung: `config.toml` + `aiwm.db` + `<data>/agents/<profil>/`** (die
gemanagten Agent-Configs/Memory/Skills, die wir besitzen) + ein **Modell-
Manifest** (Namen/SHAs, nicht die GB-Dateien). Ein `.zip`, versioniert.
`~/.hermes/` des Users bleibt außen vor (wir fahren ein eigenes Profil).

---

## 5.0 — Ergebnis (weitgehend abgeschlossen) · **ADR-021**

Alles auf der echten Maschine verprobt (`node` 24, `uv` 0.12, `wsl`, Git-Bash da).
Details + Entscheidungen in **ADR-021**; Kurzfassung:

- **OpenCode** (`opencode-ai` 1.18.30, npm → **eine self-contained `bin/opencode.exe`**,
  läuft nativ Windows): `opencode serve --port … --hostname 127.0.0.1` läuft;
  `GET /doc` = OpenAPI 3.1 (~180 Ops). **`OPENCODE_CONFIG_CONTENT`-Env erzwingt
  die Config** — `GET /config` spiegelt sie 1:1. `enabled_providers: ["local"]`
  + `disabled_providers: ["opencode", …]` → `GET /config/providers` zeigt **nur
  `local`** (der eingebaute „OpenCode Zen"-Provider muss explizit in
  `disabled_providers`). Der **Approval-Fluss verprobt** (Streaming-Stub emittiert
  einen OpenAI-`tool_calls`-Delta): `GET /event` liefert `permission.asked`
  `{ id, permission:"bash", patterns, metadata.command, always:["cat *"],
  tool:{messageID,callID} }` → `GET /permission` → `POST /permission/{id}/reply`.
  Transkript-Events: `message.part.updated` mit `part.type` `text` / `tool`
  (`status` pending→running, `input`). Prompt: `POST /session/{id}/prompt_async`
  bzw. `POST /session/{id}/message` `{providerID, modelID, parts:[{type:text,text}]}`.
  **CWD = Projekt** — der Adapter muss `cwd` = Workspace setzen. State unter
  `~/.local/share/opencode/` (respektiert `XDG_*`). **→ erster Adapter, bestätigt.**
- **Hermes Agent** (`hermes-agent` 0.19.0, PyPI, `uv pip install` — **120
  Dependencies** + `hermes postinstall` zieht `node` / Browser / `ripgrep` /
  `ffmpeg`; deutlich schwerer als OpenCode). `hermes serve` = headless
  JSON-RPC/WS-Gateway (`127.0.0.1:9119`, `--skip-build` ohne npm); `hermes -z
  "<prompt>"` single-shot; `hermes acp` (ACP-stdio). Config
  `$HERMES_HOME/config.yaml` (`model: {provider: custom, base_url, api_key}`),
  `hermes doctor` als Selbsttest. **`bash -l` ist entschärft**: 0.19 hat
  native Windows- + Git-Bash-Pfad-Behandlung (MSYS `/c/Users/…`), der
  WSL2-Zwang aus ADR-010 gilt nicht mehr — Git-Bash reicht; `terminal.
  auto_source_bashrc` / `shell_init_files` sind nur PATH-Feinschliff.
- **`llama-server`-Tool-Calling** (Entscheidung E — **war Blocker-Kandidat,
  jetzt „Config-Detail"**): `llama-server --jinja` parst OpenAI-Tool-Calls aus
  dem im GGUF eingebetteten Chat-Template; nativ für **Qwen 2.5 / Qwen 2.5
  Coder**, **Hermes 2/3**, Llama 3.x, Functionary, Mistral Nemo. Ohne `--jinja`
  → Tool-Delimiter kommen als Klartext durch. Ggf. `--chat-template <name>` /
  `--chat-template-file`. Der Stub-Test hat bewiesen: OpenCodes
  `@ai-sdk/openai-compatible`-Runtime parst **Standard-OpenAI-`tool_calls`-
  Deltas** — genau was `llama-server --jinja` emittiert. **Offen bleibt nur**:
  ein echter Coding-Modell-Lauf (Qwen2.5-Coder-GGUF) → verlässlich `tool_calls`?
  KV-Cache nicht zu hart quantisieren (`-ctk q4_0` schadet). → 5.1-Smoke.
- **`LlamaCppAdapter`** braucht in 5.1 ein **`--jinja`-Flag** (+ optional
  `--chat-template`), wenn er ein `coding`-Rollen-Modell für einen Agenten
  serviert — neues Feld in `LlamaServerOptions` / `[llama]` oder ein
  Agent-spezifischer Spawn.

Verifiziert (Scratchpad-Probes, keine Repo-Änderung außer Docs):
- `opencode serve` + forced config + `enabled_providers` + der
  `permission.asked` → `reply`-Zyklus, end-to-end gegen einen SSE-`tool_calls`-Stub.
- `hermes-agent` Installation (`uv venv` 3.13 + `uv pip install`), `hermes
  doctor` / `config` lesen unsere `config.yaml` (custom endpoint).
- Research: llama.cpp `docs/function-calling.md`, OpenCode `/docs/server` +
  `/docs/config`, Hermes FAQ/Docs.

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
