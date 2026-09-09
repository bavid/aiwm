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
| **5.1a** | **Agent-Fundament**: `core::agent` (`AgentAdapter`-Trait, `AgentEvent`/`SessionSpec`/`PermissionDecision`, `FakeAgentAdapter`), Migration `0005` (`agents`, `agent_sessions`, `agent_session_events`) + `db::AgentRepo`. `LlamaServerOptions.jinja`/`chat_template` (+ `[llama]`-Config, Settings-UI) — `--jinja` default an (Tool-Calls). | ✅ |
| **5.1b** | **OpenCode-Adapter** (nur der Adapter, gegen ein Fixture): `opencode serve --port … --hostname 127.0.0.1` unter `RuntimeSupervisor`, **ein Prozess pro Session**, `cwd` = Workspace; Config **erzwungen** über `OPENCODE_CONFIG_CONTENT` (custom provider → `spec.endpoint.base_url`, `enabled_providers: ["local"]` + `disabled_providers: ["opencode",…]`, `permission` alles `ask`, `webfetch: deny`, `tools.webfetch: false`). `AgentAdapter` für OpenCode: `open_session` / `send` (`POST /session/:id/prompt_async`) / `events` (`GET /event` SSE → `AgentEvent`, mit `roles`-Cache gegen Echo) / `reply_permission` (`POST /permission/{id}/reply`) / `interrupt` (`abort`) / `close_session` (`DELETE /session/:id`). `aiwm-fake-opencode`-Fixture + Integrationstest (open → send → approve → idle, sowie deny). | ✅ |
| **5.1ca** | **Agent-Subsystem (Core, ohne API)**: `capability::agent::AgentSessions` — eigenes Subsystem, **kein** Job (läuft nicht im Job-Loop). `open` → Coding-Modell auflösen (`coding`-Rolle \| explizit) → `CodingRuntime`-Trait platziert+pinnt es (`HybridScheduler::plan` → load/evict → `pin`, `LlamaCppAdapter::base_url()` neu+public) → `adapter.open_session` gegen `<llama>/v1` → Event-Drain-Task (`AgentEvent` → `agent_session_events` + `agent_sessions.state`). `message`/`reply`/`interrupt`/`stop` (stop = `close_session` + unpin + unload). MVP: **eine Session gleichzeitig**. Integrationstest `agent_session.rs` (fake-llama + fake-opencode, open→send→approve→idle→stop). | ✅ |
| **5.1cb** | **Agent-Vertikale (API/UI-Anbindung)**: `App.agents` = `AgentSessions` + `OpenCodeAdapter::discover`; API `GET/POST /agents`, `DELETE /agents/:id`, `POST /agent-sessions`, `GET /agent-sessions/:id`, `POST /agent-sessions/:id/{message,permission,stop}` + DTOs + 8 Tauri-Commands + `ipc.ts`/dev-mock. `docs/AGENT_MODELS.md` mit kuratierten Kandidaten + manueller Smoke-Prozedur. Echter Qwen2.5-Coder-GGUF-Lauf = manuell (dokumentiert, wie 4.0). | ✅ |
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

## 5.1a — Ergebnis (abgeschlossen)

Das Fundament — Datenmodell, Trait, Fake, `--jinja`. Kein realer Adapter, keine
API, kein `capability::agent` (das ist 5.1b).

- **Migration `0005_agents.sql`**: `agents` (id, name, adapter, model_id NULL,
  workspace_path, allowed_paths_json, toolset_json, created_at) · `agent_sessions`
  (adapter_session_id, state, error_text, checkpoint_path, started/ended_at,
  FK cascade) · `agent_session_events` (session_id, ts, kind, payload_json —
  append-only wie `job_events`). `db::AgentRepo` (`db.agents()`): Profil-CRUD,
  `create_session` / `set_session_state` (terminal → `ended_at`) /
  `bind_adapter_session` / `set_checkpoint` / `append_event` / `session_events` /
  `live_sessions` (Crash-Recovery, 5.5).
- **`core::agent`** (`agent/mod.rs` + `fake.rs`): `AgentAdapter`-Trait
  (`open_session(SessionSpec) → String`, `send`, `events → mpsc::UnboundedReceiver
  <AgentEvent>`, `reply_permission`, `interrupt`, `close_session`, `health`).
  `AgentKind {OpenCode, Hermes, Fake}`. `SessionSpec { workspace, allowed_paths,
  toolset, endpoint: EndpointConfig { base_url, model } }`. `AgentEvent`
  (`#[serde(tag="type")]`): `Text` / `Tool {status, input, output}` /
  `Permission {id, kind, summary, always_pattern}` / `Idle` / `Error {terminal}`
  — serialisiert ganz nach `payload_json`, `kind()` = die grobe Spalte.
  `PermissionDecision {AllowOnce, AllowAlways, Deny}`. `FakeAgentAdapter`
  (`with_script(events)` → `send` replayt; `replies()` / `last_spec()` für
  Assertions).
- **`--jinja`**: `LlamaServerOptions` bekommt `jinja: bool` (**default `true`** —
  korrekt für Chat, nötig für Tool-Calls) + `chat_template: Option<String>` →
  `--jinja` / `--chat-template <n>` im Spawn. `[llama]`-Config `jinja` +
  `chat_template`; Settings-UI: „Jinja chat template"-Toggle + Override-Feld.

Verifiziert:
- **+6 Unit-Tests** (`agent` — Event-Serde + `kind`; `agent::fake` — Script-
  Replay + Reply-Recording + Fehler ohne Subscriber; `db::agents` — Profil-
  Round-Trip inkl. JSON-Spalten, Session-Lebenszyklus + Transkript, Cascade-
  Delete) → **254 Lib-Tests**. `config` + `launch`-Tests um `jinja` erweitert.
  `check.ps1` grün, `pnpm typecheck`/`lint` grün.
- **Live** (Boot-Smoke): `aiwm-cored` startet, Migration `0005` legt
  `agents` / `agent_sessions` / `agent_session_events` an.

---

## 5.1b — Ergebnis (abgeschlossen)

Der erste echte Adapter — `OpenCodeAdapter`, isoliert getestet gegen ein
Fixture. Noch **keine** Scheduler-Anbindung, kein `capability::agent`, keine
API/UI — das ist 5.1c. Die 5.1b-Zeile im Plan wurde entlang dieser Naht
geteilt (Adapter jetzt, Vertikale = 5.1c).

- **`agent::opencode`** (`opencode.rs` + `opencode/sse.rs`): `OpenCodeAdapter`
  implementiert `AgentAdapter`. `open_session` startet `opencode serve --port
  <frei> --hostname 127.0.0.1 --print-logs --log-level WARN` über
  `RuntimeSupervisor` (Job Object), `cwd` = `spec.workspace`, Env
  `OPENCODE_CONFIG_CONTENT` = die erzwungene Config; wartet per `GET /config`
  auf Bereitschaft (30 s, `GaveUp`-Check), dann `POST /session` → Session-Id.
  **Ein Prozess pro Session** (MVP — der Scheduler pinnt ohnehin ein
  Agent-Modell). `send` → `POST /session/:id/prompt_async`
  `{parts:[{type:text,text}]}`. `reply_permission` → `POST
  /permission/{id}/reply` `{response: once|always|reject}`. `interrupt` →
  `POST /session/:id/abort` (best-effort). `close_session` → `DELETE
  /session/:id` + `Session` droppen (killt Reader-Task + Kindprozess).
- **`forced_config`**: `provider.local` (`@ai-sdk/openai-compatible`,
  `options.baseURL` = `spec.endpoint.base_url`, Dummy-`apiKey`),
  `model: "local/<model>"`, `enabled_providers: ["local"]`,
  `disabled_providers: ["opencode", …9 weitere]`, `permission` alles `ask` +
  `webfetch: deny`, `tools.webfetch: false`.
- **`opencode/sse.rs`**: der `GET /event`-Reader-Task (`resp.chunk()`-Schleife
  + Zeilenpuffer, wie `llamacpp/client.rs`). `map_event` übersetzt
  `message.updated` (→ `roles`-Cache), `message.part.updated` (text/tool, nur
  von `assistant`-Messages — sonst würde der User-Prompt zurückgespiegelt),
  `permission.asked`/`updated`, `session.idle`, `session.error` in
  `AgentEvent`s; Events fremder Sessions werden verworfen.
- **`bin/aiwm-fake-opencode`**: axum-Fixture, skriptet einen Turn (Text →
  `bash`-Tool → `permission.asked` → [warten] → Tool-Ergebnis → Text →
  `session.idle`; bei `reject` direkt idle). `AIWM_OPENCODE_PATH` zeigt den
  Adapter im Test darauf.
- **Binärauflösung**: `AIWM_OPENCODE_PATH` → `<runtimes>/opencode/[<ver>/]
  [bin/]opencode` → `PATH`. `discover(runtimes_dir)` (Rescan) vs.
  `with_binary(path)` (fix, Tests).

Verifiziert:
- **+9 Tests** — 6 Lib (`agent::opencode` — `forced_config`, „not installed",
  `resolve_bin`-Env-Vorrang; `agent::opencode::sse` — User-Text nicht
  gespiegelt/Assistant schon, Tool+Permission+Idle, Fremd-Session verworfen)
  + 3 Integration (`tests/opencode_adapter.rs` — voller Turn
  open→send→approve→idle, Deny beendet den Turn, `interrupt` mitten im Turn)
  → **260 Lib-Tests**. `check.ps1` grün.
- **Kein realer `opencode`-Lauf** — wie beim ComfyUI-`/history`-Key (Phase 4)
  werden die exakten Event-Ecken beim ersten echten Coding-Modell-Lauf in
  5.1c kalibriert.

---

## 5.1ca — Ergebnis (abgeschlossen)

Das Agent-Subsystem im Core — verdrahtet Scheduler + `LlamaCppAdapter` +
`AgentAdapter` zu einer treibbaren Session. Noch **keine** API/Tauri/UI (5.1cb).

- **`LlamaCppAdapter::base_url()`** (neu, public) → `Some("http://127.0.0.1:<port>")`
  solange ein Modell resident ist, sonst `None`.
- **`capability::agent::CodingRuntime`** (Trait) — „halte das Coding-Modell für
  die Session-Dauer auf der GPU": `acquire(model_id, vram) -> <base>/v1`,
  `release(model_id)`. `LlamaCodingRuntime` (Produktion): `HybridScheduler::plan`
  → `RunNow` / `load_model` / evict+load / `SchedulerBlocked` → `pin`. Der Trait
  hält das Subsystem vom Job-Loop **und** von einem echten `llama-server` im
  Unit-Test entkoppelt (Stub).
- **`capability::agent::AgentSessions`** — `new(db, Arc<dyn CodingRuntime>)` +
  `.with_adapter(Arc<dyn AgentAdapter>)`. `open(agent_id, first_message?)`:
  Profil laden → `AgentKind::from_adapter` → Adapter da? → Modell auflösen
  (`profile.model_id` \| `models.pick_for_role("coding")`, sonst Klartext-Fehler)
  → `coding.acquire` → `create_session` → `adapter.open_session(SessionSpec {
  workspace, allowed_paths, toolset, endpoint })` → `bind_adapter_session` →
  `adapter.events` → **Drain-Task** → ggf. `first_message` senden. Schlägt der
  Runtime-Start fehl: `coding.release` + Session `Failed`. `message` / `reply`
  (Permission) / `interrupt` / `stop` (= `close_session` + `release` + `Stopped`).
  **MVP: eine Session gleichzeitig** (`open` lehnt eine zweite mit Klartext ab —
  der Scheduler pinnt genau ein Agent-Modell).
- **Drain-Task** (`drain_events`): jedes Event → `append_event(kind, payload)` +
  `next_state()` (`Permission`→`AwaitingApproval`, `Idle`→`Idle`, `Text`/`Tool`
  →`Working`, `Error{terminal:true}`→`Failed` und Task-Ende).
- **`AgentKind`**: `+ Hash`, `+ from_adapter(&str)`.

Verifiziert:
- **+6 Lib-Tests** (`next_state`-Mapping; „kein Coding-Modell"; „falscher Adapter
  → nichts gepinnt"; `SchedulerBlocked` → keine Session; voller Zyklus mit
  Stub-`CodingRuntime` + `FakeAgentAdapter`; eine Session gleichzeitig) →
  **266 Lib-Tests**. `attach`-Test um `base_url()` erweitert.
- **+1 Integrationsdatei** `core/tests/agent_session.rs` (2 Tests, `#![cfg(windows)]`):
  echtes `AgentSessions` + `LlamaCodingRuntime` + `LlamaCppAdapter`(fake-llama) +
  `OpenCodeAdapter`(fake-opencode) — open (mit Turn) → Modell resident + gepinnt
  → `permission`-Event parkt die Session → approve → `tool`/`text`/`idle` → stop
  entpinnt + entlädt; plus Deny-Variante. **40 Integrationstests.**
- `check.ps1` grün.

---

## 5.1cb — Ergebnis (abgeschlossen)

Die Anbindung — `AgentSessions` hängt jetzt in `App`, ist über beide Transporte
erreichbar. Kein UI-Tab (das ist 5.3); die Smoke ist eine dokumentierte
`curl`-Prozedur.

- **`App.agents`**: `App::load` baut `LlamaCodingRuntime` + `AgentSessions::new`
  `.with_adapter(OpenCodeAdapter::discover(&paths.runtimes_dir()))`. Kein Job-Loop
  nötig (die Drain-Tasks sind selbständig); `Services`/`spawn` unverändert.
- **DTOs** (`api/dto.rs`): `NewAgentDto`, `OpenAgentSessionDto`, `AgentMessageDto`,
  `AgentPermissionDto` (`decision` = `PermissionDecision` serde), `AgentSessionDetailDto`
  (`{ session, events, live }`).
- **Handlers** (`api/handlers.rs`): `create_agent` (trim + `AgentKind::from_adapter`
  validiert → 400), `list_agents`, `delete_agent`, `open_agent_session`,
  `agent_session_detail` (`None` → 404), `agent_session_message` (leerer Text →
  400), `agent_session_permission`, `stop_agent_session`.
- **HTTP** (`api/http.rs`): `GET/POST /agents`, `DELETE /agents/{id}`,
  `POST /agent-sessions`, `GET /agent-sessions/{id}`,
  `POST /agent-sessions/{id}/{message,permission,stop}`. `ApiError` mappt jetzt
  `CoreError::SchedulerBlocked` → 400 (VRAM-Klartext ist client-actionable);
  „already running" / „no coding model" sind `CoreError::Config` → 400.
- **Tauri** (`src-tauri/src/lib.rs`): 8 `#[tauri::command]`s + `generate_handler!`.
- **UI-Bindings** (`ui/src/lib/ipc.ts`): `Agent` / `AgentSession` /
  `AgentSessionEvent` / `AgentSessionDetail` / `PermissionDecision` Typen +
  `listAgents` / `createAgent` / `deleteAgent` / `openAgentSession` /
  `agentSessionDetail` / `agentSessionMessage` / `agentSessionPermission` /
  `stopAgentSession`. `dev-mock.ts` beantwortet alle acht (skriptet eine
  `awaiting_approval`-Session für den späteren Tab).
- **`docs/AGENT_MODELS.md`** (neu): `coding`-Rolle, Auflösungsreihenfolge,
  kuratierte GGUF-Kandidaten (Qwen2.5-Coder-7B als `Auto`-Empfehlung), Import-
  Schritte, die manuelle Smoke-Prozedur + Kalibrier-Checkliste.

Verifiziert:
- **+4 Lib-Tests** (`api::tests` — Profil-CRUD über HTTP inkl. Trim; `create_agent`
  lehnt leeren Namen + unbekannten Adapter mit 400 ab; `open_agent-session` ohne
  Coding-Modell = 400 mit „coding" im Body; `GET /agent-sessions/nope` = 404 +
  `stop` idempotent 204) → **270 Lib-Tests**. `check.ps1` grün (inkl.
  `pnpm typecheck`/`lint`).
- **Der echte GGUF-Lauf ist manuell** (dokumentiert) — kein persistentes
  `opencode`+Coding-Modell-Setup im Repo, analog Slice 4.0.

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
