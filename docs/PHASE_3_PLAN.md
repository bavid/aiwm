# Phase 3 — Bild-Generierung (ComfyUI)

Zweite echte Capability: **Text→Bild** über eine gekapselte ComfyUI-Runtime. In
Scheiben, jede für sich testbar — gleiche Mechanik wie Phase 2 (Adapter →
Installer → Link → Capability → UI → Politur).

**Phase-3-DONE-Kriterium:** frisches Windows → App → ComfyUI wird eingerichtet
(gepinnt, gekapselte venv) → SDXL-Modell importieren → „Generate" mit Prompt →
Bild landet im Output-Ordner + Galerie (mit Prompt/Seed/Modell) → währenddessen
läuft ein Chat-Job: der Scheduler tauscht LLM ↔ Diffusion ums VRAM-Budget, kein
manuelles Eingreifen → Netz trennen → das schon Installierte läuft weiter.

| Scheibe | Inhalt | Status |
|---|---|---|
| 3.1 | `ComfyUiAdapter` (`RuntimeAdapter`): Prozess-Spawn (`python main.py --listen 127.0.0.1 --port … --base-directory … --disable-auto-launch`), Health über `GET /system_stats`, Attach-Fallback, `unload_model` → `POST /free`, VRAM-Buchhaltung aus `/system_stats`; `aiwm-fake-comfy`-Fixture | offen |
| 3.2a | ComfyUI-**Installer** (Teil 1): `uv`-verwaltete venv unter `<local_root>/runtimes/comfyui/<tag>/`, ComfyUI am gepinnten Git-Tag (Clone/Tarball + Verify), Torch-CUDA-Wheel + gepinnte `requirements`, `offline_mode`-Hard-Refusal, „Repair" = venv neu, Fortschritt in `detail()` | offen |
| 3.2b | ComfyUI-Installer (Teil 2): gepinnter Custom-Node-Satz — **exakt einer**: `city96/ComfyUI-GGUF` (an einem Commit), für Flux-GGUF. SHA/Commit fest im Code. SDXL-txt2img braucht **keine** Custom Nodes | offen |
| 3.3 | Link-Manager **echt**: neue Store-Struktur `E:\AI\models\image\{checkpoints,unet,vae,clip,loras}`; `core::link` junctioniert diese Ordner in ComfyUIs `models/…` (Directory-Junction, kein Admin — das, wofür 2.3 gebaut wurde); `import_model` nimmt `.safetensors`, routet per Rolle in den richtigen Unterordner; `model_links` = `comfyui`/`junction` | offen |
| 3.4 | `capability::image`: `job_type=image`, Params (prompt, negative, w/h, steps, cfg, seed, model|Auto über Rolle); **feste Pipeline** = Workflow-JSON-Template + Param-Substitution (`core::pipeline`); `POST /prompt` → `/history/{id}` pollen → Bild via `/view` nach `<data>/outputs/<job_id>.png` → `jobs.output_path`; Cancel via `POST /interrupt`; VRAM-Schätzung pro Familie | offen |
| 3.5 | UI: Tab „Image" — Prompt/Negativ, Größe/Steps/CFG/Seed, Model [Auto], „Generate"; Ergebnisbild; einfache **Galerie** (Bild-Jobs mit Thumbnail, Klick → Prompt/Seed/Modell); Dashboard-Button „Generate Image" aktiv | offen |
| 3.6 | Zweites Template **Flux.1-dev (GGUF Q8)** über `ComfyUI-GGUF`; `docs/IMAGE_MODELS.md` (kuratierte Modelle: SHA256, Quelle HF, Lizenz, empfohlene Settings); kuratierte „Known models"-Liste für den assistierten Import | offen |
| 3.7 | Politur: ComfyUI-Optionen in Settings (`--lowvram`-Schalter / VRAM-Modus), Diagnostics-Statuszeile, Output-Retention-Hinweis; Scheduler: Diffusion-Slot neben LLM-Slot sauber verproben | offen |

---

## Research (2026-09, Pflicht-Schritt „Research & Reuse")

### ComfyUI als Runtime — passt auf unsere Abstraktion

ComfyUI hört per Default schon nur auf `127.0.0.1` (`--listen` Default) — deckt
sich mit ADR-008. Die HTTP/WS-API ist stabil und dokumentiert
([docs.comfy.org](https://docs.comfy.org/development/comfyui-server/comms_routes)):

| Zweck | Endpoint |
|---|---|
| Health / VRAM / Devices | `GET /system_stats` |
| Workflow einreihen | `POST /prompt` → `{prompt_id, number}` (oder `{error, node_errors}`) |
| Ergebnis holen | `GET /history/{prompt_id}` → Output-Dateien + Metadaten |
| Bild abrufen | `GET /view?filename=…&subfolder=…&type=output` |
| Abbrechen | `POST /interrupt` |
| VRAM freigeben | `POST /free` (Modelle entladen) |
| Node-Schemas (Workflow validieren) | `GET /object_info` |
| Fortschritt | `GET /ws?clientId=…` — `status` / `execution_start` / `executing` / `progress` / `executed` / `execution_error` |

→ **Gleiches Muster wie llama-server (ADR-002/ADR-014):** ComfyUI läuft als
überwachter Kindprozess, `ComfyUiAdapter` ist HTTP/WS-Client. Attach-Fallback
über `/system_stats`. Cancel = `POST /interrupt` (analog `stream.abort()` beim
Chat). MVP pollt `/history` (wie die Chat-UI `jobs.result` pollt); `/ws`-Progress
ist eine spätere Verfeinerung.

Relevante CLI-Flags: `--listen 127.0.0.1 --port <frei> --base-directory <data>
--output-directory <out> --disable-auto-launch --dont-print-server`. VRAM ist
dynamisch (Default); `--lowvram` schiebt die Text-Encoder auf die CPU (nützlich
für Flux auf 16 GB).

### Installation — gekapselte venv, gepinnter Tag

- ComfyUI: schnelle Release-Kadenz (`v0.34.0` 2026-08-26, wöchentlich-ish),
  saubere `vX.Y.Z`-Tags, `requires-python >=3.10`. `pyproject.toml` +
  `requirements.txt` (Frontend-Pakete gepinnt, `torch` frei → CUDA-Index-URL).
- **Weg:** ComfyUI am Tag klonen/als Tarball ziehen (verifiziert), `uv venv` +
  `uv pip install -r requirements.txt` + Torch-CUDA-Wheel in
  `<local_root>/runtimes/comfyui/<tag>/`. **Nicht** `comfy-cli` als Laufzeit-
  Abhängigkeit (eigenes Pip-Tool, eigener Env-Zustand) — wir kontrollieren die
  venv wie bei llama.cpp die Binärdatei. `comfy-cli` als Referenz für die
  Install-Schritte ist ok.
- Referenz-Implementierungen „ComfyUI als API": `replicate/cog-comfyui`,
  `SaladTechnologies/comfyui-api`, `itsKaynine/comfy-ui-client` (WS-Contract).

### Custom Nodes = Risiko (ANALYSIS.md §13)

Custom Nodes sind beliebiger Python-Code — **nie automatisch installieren**,
**nie** `ComfyUI-Manager` einbinden. Phase 3 pinnt **genau einen** Node:
`city96/ComfyUI-GGUF` (3,9k ⭐, Standard für Flux/SD3.5-GGUF) an einem festen
Commit. SDXL-txt2img braucht **null** Custom Nodes (alles Core). ControlNet /
Upscaler / IPAdapter → jeweils später, bewusst.

### Modell-Landschaft für 16 GB (RTX 4080 Super), Stand 2026-09

| Modell | VRAM | Bewertung |
|---|---|---|
| **SDXL 1.0** (3,5B) | ~6–8 GB | reif, größtes LoRA/ControlNet-Ökosystem, **kommerzielle Lizenz**, schwache Text-Wiedergabe → **Default & `Auto`-Wahl** |
| **Flux.1-dev** (12B) | fp8 ~12 GB / GGUF Q8 ~13 GB | beste Prompt-Treue + In-Bild-Text; **nicht-kommerzielle** Lizenz (für dieses Projekt ok, ADR-011); braucht `ComfyUI-GGUF` → **zweites Template (3.6)** |
| SD 3.5 | 12–18 GB | krummer VRAM-Bedarf, kleines Ökosystem → **übersprungen** |
| Qwen-Image | — | stark bei Text im Bild, neuer → Phase 6+ |

Quellen: [willitrunai VRAM-Guide 2026](https://willitrunai.com/blog/image-generation-vram-guide-2026),
[localaimaster FLUX vs SDXL vs Qwen](https://localaimaster.com/blog/best-local-image-models-compared),
[spheron GPU-Guide 2026](https://www.spheron.network/blog/best-gpu-ai-image-generation-2026/).

---

## Offene Entscheidungen (Empfehlung → deine Freigabe)

**A — ComfyUI-Prozessmodell.**
→ **Empfehlung: eigener überwachter Kindprozess + HTTP/WS-Client-Adapter**, mit
Attach-Fallback (identisch zu llama-server, Phase 2 bewährt). Keine ernsthafte
Alternative — ComfyUI *ist* ein Server.

**B — Erstes Modell-Template.**
→ **Empfehlung: SDXL zuerst, `Auto`-Default; Flux als zweites Template (3.6).**
SDXL passt bequem in 16 GB, riesiges Ökosystem, kommerzielle Lizenz, braucht
keine Custom Nodes → 3.4 wird ohne VRAM-Drama fertig. Flux (GGUF Q8, ~13 GB) ist
die Qualitäts-Vorführung, kommt drauf, wenn SDXL steht. (Spiegelt 2.4: „erst mit
einem kleinen Modell zum Laufen bringen".)

**C — Workflow-Abstraktion.**
→ **Empfehlung: nur feste Pipelines im MVP.** Eine Handvoll kuratierter
Workflow-JSON-Templates (`sdxl_txt2img`, `flux_txt2img`), Params in bekannte
Node-Slots substituiert. **Kein** Graph-Editor, **kein** Custom-Workflow-Upload —
das wäre wieder „beliebige Node-Ausführung". Start: Substitution in
`capability::image` hartkodiert pro Template; **TOML-Pipeline-Registry**
(`pipelines`-Tabelle, ARCHITECTURE.md) sobald es 2+ Templates gibt (echte
DRY-Pressure, kein spekulativer Vorbau). Custom Workflows → Post-MVP, mit Warnung.

**D — Store-Layout & Junction-Ziel.**
→ **Empfehlung:** `E:\AI\models\image\{checkpoints,unet,vae,clip,loras}` als
kanonische Ordner; `core::link` junctioniert sie nach
`<comfy base>/models/<typ>`. Directory-Junctions (kein Admin, gleiche Volume —
Store `E:`, Runtime `C:` heißt: Junction zeigt von `C:` nach `E:`, das geht).
`import_model` routet `.safetensors` per Rolle (`base_diffusion` → checkpoints,
`vae` → vae, `lora` → loras …).

---

## Bewusst nicht in Phase 3

Image→Image / Inpaint / Upscale / Enhancement (ROADMAP nennt sie — kommen als
eigene Slices *nach* txt2img steht oder in Phase 3.x), ControlNet/IPAdapter,
LoRA-Stacking-UI, Prompt-Styles-Bibliothek, Batch/Queue-Größe > 1, Online-Modell-
Discovery (Phase 6), Video (Phase 4), Graph-Editor.
