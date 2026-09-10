# Agent models — coding-Rolle für Agent-Sessions

Ein Agent (`opencode`, später `hermes`) braucht ein lokales LLM mit
**verlässlichem Tool-Calling** hinter einem OpenAI-kompatiblen Endpoint. AIWM
serviert dafür ein GGUF über `llama-server --jinja` (5.1a) und pinnt es für die
Session-Dauer (5.1ca).

Wie bei Chat/Bild/Video: **kein Download-Manager im MVP** (Phase 6). Man lädt das
GGUF selbst von Hugging Face und importiert es über den **Models**-Tab als
**Chat**-Typ, mit der zusätzlichen Rolle **`coding`**. Fehlt eine Datei mit
`coding`-Rolle, lehnt `open_agent_session` mit Klartext ab.

> **Noch nicht mit einem echten Coding-Modell verprobt.** OpenCode-Adapter
> (5.1b) + Subsystem (5.1ca) + API (5.1cb) + Sandkasten-Config (5.2) + UI (5.3)
> + Hermes-Adapter + Installer (5.4) laufen gegen Fixtures. Der erste echte Lauf
> (Smoke unten) kalibriert die OpenCode-Event-Ecken *und* prüft, dass die
> erzwungene `permission`-Config von der echten `opencode`-Version akzeptiert
> wird — wie der ComfyUI-`/history`-Key in Phase 4. Für **Hermes** sind
> zusätzlich zu verifizieren: der `uv`-Installer + `hermes postinstall` (der
> `#[ignore]`-Test `agent::hermes::install::tests::real_pinned_install` fährt
> ihn), die SSE-Event-Namen (`assistant.delta` vs. `message.delta`,
> `approval.request`, ob `/chat/stream` nach der Approval offen bleibt) und die
> `config.yaml`-Sandbox-Keys. Fixes fließen hierher zurück.

Hardware-Kontext: RTX 4080 Super, 16 GB VRAM, 32 GB RAM. Ein 7B-Q4/Q5-Modell
lässt ~9–11 GB VRAM für alles andere; ein 14B-Q4 füllt die Karte fast allein.

---

## Modell-Auswahl

`open_agent_session` löst das Modell so auf:

1. `agent.model_id` gesetzt → genau dieses (muss in der Library sein).
2. sonst **`Auto`** → `core::select::pick_for_role("coding", …)` (6.6): passt es
   ins VRAM-Budget → benchmark-Score (`[models].auto_preference`) → Nutzung.
   Ohne Benchmark-Daten = zuletzt / meist genutzt / Name.

Die `EndpointConfig.model` an OpenCode ist der Library-**Name** des Modells;
`llama-server` serviert ohnehin nur das geladene Modell, der Name ist für
OpenCodes Provider-Map.

---

## Kuratierte Kandidaten (recherchiert)

Alle mit nativem Tool-Calling-Support in llama.cpp `--jinja` (das GGUF-eigene
Chat-Template trägt die Tool-Grammatik). **KV-Cache nicht zu hart quantisieren**
(`-ctk q4_0` schadet Tool-Calls — Default `f16` lassen).

| Modell | Quant | Größe | ~VRAM (Gewichte + 8k ctx) | Quelle | Lizenz |
|---|---|---|---|---|---|
| **Qwen2.5-Coder-7B-Instruct** | Q5_K_M | 5,4 GB | ~8–9 GB | [`bartowski/Qwen2.5-Coder-7B-Instruct-GGUF`](https://huggingface.co/bartowski/Qwen2.5-Coder-7B-Instruct-GGUF) | Apache-2.0 |
| Qwen2.5-Coder-14B-Instruct | Q4_K_M | 9,0 GB | ~12–13 GB | [`bartowski/Qwen2.5-Coder-14B-Instruct-GGUF`](https://huggingface.co/bartowski/Qwen2.5-Coder-14B-Instruct-GGUF) | Apache-2.0 |
| Qwen2.5-Coder-32B-Instruct | Q3_K_M | ~15 GB | knapp / zäh auf 16 GB | `bartowski/Qwen2.5-Coder-32B-Instruct-GGUF` | Apache-2.0 |
| Hermes-3-Llama-3.1-8B | Q5_K_M | 5,7 GB | ~9 GB | [`NousResearch/Hermes-3-Llama-3.1-8B-GGUF`](https://huggingface.co/NousResearch/Hermes-3-Llama-3.1-8B-GGUF) | Llama-3.1 Community |
| Llama-3.1-8B-Instruct | Q5_K_M | 5,7 GB | ~9 GB | `bartowski/Meta-Llama-3.1-8B-Instruct-GGUF` | Llama-3.1 Community |

**Empfehlung als `Auto`-Default:** Qwen2.5-Coder-7B-Instruct Q5_K_M — bestes
Tool-Calling/VRAM-Verhältnis auf 16 GB, lässt genug Kopf für ComfyUI-freie
Arbeit.

**Für den Hermes-Runtime** (5.4) ist **Hermes-3-Llama-3.1-8B** der natürliche
Griff — Nous Researchs eigenes, auf ihren Agenten trainiertes Modell. Für den
OpenCode-Runtime ist Qwen2.5-Coder stärker beim reinen Code. Beide brauchen die
`coding`-Rolle; ein Profil bindet einen Runtime + ein Modell.

Wenn ein GGUF-Template das Tool-Format nicht kennt: in den Settings
`llama.chat_template` setzen (z. B. `qwen2.5-coder`, `hermes-3`) — `--jinja`
bleibt an.

---

## Import (Models-Tab)

1. GGUF von HF laden (eine Datei).
2. Models → **Import** → Typ **Chat**, Rollen `chat, coding` (Komma).
3. Import stempelt Familie/Kontext aus dem GGUF-Header.

Danach greift `Auto` in einem Agent-Profil (Rolle `coding`).

---

## 5.1cb-Smoke — der erste echte Lauf (manuell)

Voraussetzungen: `opencode` installiert (`AIWM_OPENCODE_PATH` oder
`<data>/runtimes/opencode/`), llama.cpp installiert, ein Coding-GGUF importiert
(`coding`-Rolle).

```bash
# 1. Profil anlegen (loopback-API; Port aus GET /about)
curl -s localhost:<port>/agents -X POST -H 'content-type: application/json' \
  -d '{"name":"Repo coder","adapter":"opencode","workspace_path":"E:\\AI"}'

# 2. Session öffnen mit einem ersten Turn
curl -s localhost:<port>/agent-sessions -X POST -H 'content-type: application/json' \
  -d '{"agent_id":"<id>","first_message":"read README.md and summarise the project in two sentences"}'

# 3. Transkript pollen (state, events)
curl -s localhost:<port>/agent-sessions/<session-id>

# 4. bei state=awaiting_approval: die request_id aus dem letzten permission-Event nehmen
curl -s localhost:<port>/agent-sessions/<session-id>/permission -X POST \
  -H 'content-type: application/json' -d '{"request_id":"<id>","decision":"allow_once"}'

# 5. weiter pollen bis state=idle
# 6. beenden
curl -s localhost:<port>/agent-sessions/<session-id>/stop -X POST
```

**Zu prüfen / kalibrieren:**

- Emittiert das Coding-Modell über `--jinja` echte `tool_calls` (nicht Klartext-
  Delimiter)? → sonst `llama.chat_template` setzen.
- Stimmen die `GET /event`-Formen mit `agent/opencode/sse.rs::map_event` überein
  (`message.part.updated` `state`-Keys, `permission.asked` vs. `.updated`,
  `session.error`)?
- Kommt nach `POST /session/:id/abort` ein `session.idle`? (`interrupt`-Verhalten)
- **Sandkasten (5.2):** `GET /config` gegen die erzwungene Config prüfen —
  akzeptiert die echte `opencode`-Version `permission.edit` als Objekt +
  `external_directory`? Ein Edit außerhalb des Workspace muss abgelehnt werden;
  `webfetch` darf gar nicht erst als Tool auftauchen.
- VRAM: Modell resident + gepinnt, `GET /runtimes` zeigt `llamacpp` mit dem
  Modell; ein Bild-/Chat-Job dazwischen → `blocked` mit „pause it or queue".
- Netz trennen → die Session läuft weiter (nur der lokale Endpoint).

---

## Bewusst (noch) nicht

- **Download-Manager / kuratierter Katalog** wie bei den Bild-/Video-Modellen —
  Phase 6; bis dahin ist diese Liste die „Quelle".
- **Nicht-lokale Endpoints** (OpenRouter etc.) — hart aus (ADR-009).
- **Auto-Benchmark der Tool-Call-Zuverlässigkeit** — Post-MVP.
