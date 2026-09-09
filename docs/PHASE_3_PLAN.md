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
| **3.1** | `ComfyUiAdapter` (`RuntimeAdapter`): **ein** langlebiger Server, lazy beim ersten `load_model` gestartet (`--listen 127.0.0.1 --port … --base-directory … --output-directory … --disable-auto-launch --dont-print-server`), Health über `GET /system_stats`, Attach-Fallback, `unload_model` → `POST /free` (Server bleibt oben), `load_model` = Server hoch + VRAM-Slot reservieren + alten via `/free` verdrängen; `aiwm-fake-comfy`-Fixture | ✅ |
| **3.2a** | ComfyUI-**Installer** (Kern): `uv` bootstrappen (verifizierter Static-Binary), ComfyUI-Quelle am gepinnten Tag (verifiziertes GitHub-`.zip`, flach entpackt), `uv venv --python 3.13` (lädt Python) + `uv pip install torch … --index-url cu130` + `-r requirements.txt` — die `uv`-Schritte hinter einem `CmdRunner`-Trait (unit-getestet). `offline_mode`-Hard-Refusal, idempotent, `InstallState`/`detail()`, `RuntimeRepo`, `POST /runtimes/comfyui/install`, Diagnostics-„Set up"-Knopf | ✅ |
| 3.2b | gepinnter Custom-Node-Satz — **exakt einer**: `city96/ComfyUI-GGUF` (an einem Commit; `gguf`/`sentencepiece`/`protobuf` in die venv). SDXL-txt2img braucht **keine** Custom Nodes | offen |
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

## 3.1 — Ergebnis (abgeschlossen)

Entscheidung **A** bestätigt umgesetzt (ADR-018).

- **`core::runtime::comfyui`** (neues Submodul, wie `llamacpp/`: `mod.rs` /
  `client.rs` / `launch.rs`):
  - **`ComfyUiAdapter`** implementiert `RuntimeAdapter`. Anders als llama-server
    (ein Prozess pro Modell): **ein** ComfyUI-Server, lazy beim ersten
    `load_model` gestartet, bleibt bis zum App-Ende oben (Python-Boot ist teuer).
    `load_model` = Server hoch + VRAM-Slot (ADR-003) reservieren + vorheriges
    Modell via `POST /free` verdrängen; **`unload_model` = `/free`, Server bleibt
    laufen**. `Server`-Slot: `Down` / `Starting` / `Up { port, supervisor }`
    (`supervisor: None` = attached).
  - **Auflösung** (`launch::resolve_launch`): `AIWM_COMFYUI_PYTHON` (+ optional
    `AIWM_COMFYUI_DIR` mit `main.py`), sonst
    `<runtimes_dir>/comfyui/<ver>/main.py` + `.venv`-Python. `ComfyLaunch
    { program, main: Option, extra_args }` — `main: None` = Fixture / Standalone-
    Exe. `ComfyDirs { base, output }` (neue `AppPaths::comfyui_data_dir()` /
    `outputs_dir()`, beide unter `local_root`).
  - **Start:** `--listen 127.0.0.1 --port <frei> --base-directory <d>
    --output-directory <o> --disable-auto-launch --dont-print-server` (+
    `extra_args`). Health-Gate: `GET /system_stats` bis 200 oder Timeout (120 s)
    / `SupervisorState::GaveUp`.
  - **`ComfyClient`** (`client.rs`): `system_stats` (Version + erste CUDA-Device-
    VRAM → `SystemStats`), `health` (200 → Healthy, sonst Unhealthy — ComfyUI
    kennt kein „503 lädt noch", der Adapter trackt `Starting` selbst), `free`
    (`{unload_models, free_memory}`), `interrupt` (Cancel-Hook für 3.4).
  - **`attach(port)`** (ADR-002-Fallback): probt `/system_stats`, adoptiert ohne
    Lebensdauer-Übernahme. `stop()` für expliziten Runtime-Neustart (Drop tut
    dasselbe).
  - `detail()`: „not installed" / „installed · idle" / „starting…" / „running on
    :<port>" / „attached to :<port> · model reserved".
- **Wiring:** `App::load` registriert `ComfyUiAdapter::discover` (erscheint in
  `GET /runtimes` / Diagnostics als „comfyui — not installed").
- **`free_loopback_port`** aus `llamacpp/launch.rs` nach `runtime/mod.rs`
  hochgezogen (jetzt von beiden Adaptern genutzt).
- **`aiwm-fake-comfy`** (`core/src/bin/`): minimaler ComfyUI-Ersatz
  (`/system_stats`, `/free`, `/interrupt`, `--fake-ready-ms` für langsamen Boot).

Verifiziert:
- 15 neue Unit-Tests (`comfyui::client` 4, `comfyui::launch` 6, `comfyui`-Adapter
  4, `free_loopback_port` 1) + **4 Integrationstests** `tests/comfyui_adapter.rs`
  gegen den echten Fixture-Subprozess: lazy Start beim ersten Load → healthy →
  `system_stats` → unload lässt den Server oben → zweites Modell tauscht den Slot
  ohne Server-Neustart → langsamer Kaltstart wird ausgesessen → „not installed"-
  Fehler klar. `check.ps1` grün (179 Unit + 17 Integ.).
- **Live** (`aiwm-cored`): `GET /runtimes` → `comfyui: not installed`; mit
  `AIWM_COMFYUI_PYTHON` → `installed · idle`; `noop`-Job auf `runtime=comfyui` →
  Adapter startet den Fixture-Server auf einem freien Port, Job
  `queued → … → completed`, `GET /runtimes` → `healthy · 6000 MB · running on
  :62100`. Kein Orphan-Prozess nach cored-Shutdown (Job Object).

Bewusst **nicht** in 3.1: Installer (→ 3.2), echtes `POST /prompt` / Bild-Job
(→ 3.4), `runtimes`-Tabellen-Zustand (kommt mit dem Installer), reale VRAM aus
`/system_stats` fürs Scheduler-Accounting (aktuell die deklarierte Zahl, wie bei
llama.cpp).

---

## 3.2a — Ergebnis (abgeschlossen)

- **`core::runtime::download`** (neues geteiltes Modul): `download_verified`
  (Streaming + mitlaufender SHA-256, Größe + Hash geprüft), `extract_zip` /
  `extract_zip_flat` (Letzteres wirft das `<repo>-<ref>/`-Wrapper-Verzeichnis der
  GitHub-Source-Archive weg), `hex`. `llamacpp::install` auf das Modul
  umgestellt — verhaltensgleich, alle Bestandstests grün.
- **`core::runtime::comfyui::install`**: `install(runtimes_dir, offline, runner,
  on_progress)`:
  1. **`uv`** bootstrappen — `uv-x86_64-pc-windows-msvc.zip` von den
     `astral-sh/uv`-Releases (SHA-256 aus dem `.sha256`-Sidecar), nach
     `<runtimes_dir>/comfyui/uv/uv.exe`.
  2. **ComfyUI-Quelle** — `v0.34.0.zip` von `github.com/.../archive/refs/tags/`
     (SHA-256 selbst berechnet — GitHub publiziert für Source-Archive keinen
     Digest; eine Regeneration = `SHA-256 mismatch` + Pin-Bump), flach nach
     `<runtimes_dir>/comfyui/v0.34.0/`.
  3. **venv** — `uv venv --python 3.13` (lädt Python 3.13; `UV_PYTHON_INSTALL_DIR`
     + `UV_CACHE_DIR` zeigen in den Install-Baum → „Repair" = ein `rm -rf`).
  4. **torch** — `uv pip install torch torchvision torchaudio --index-url
     https://download.pytorch.org/whl/cu130` (ComfyUIs aktuelle Empfehlung für
     RTX 20+; kein Versions-Pin — ComfyUI selbst pinnt torch nicht).
  5. **deps** — `uv pip install -r requirements.txt`.
  - Die vier `uv`-Aufrufe gehen durch das **`CmdRunner`**-Trait (`SystemRunner`
    real; ein aufzeichnender Fake im Test) — die Orchestrierung ist unit-getestet
    ohne echtes Python.
  - Idempotent (venv-Python + `main.py` vorhanden → sofort zurück).
    `offline_mode` → Hard-Refusal. `InstallPhase` = `Downloading` / `Extracting`
    / `CreatingVenv` / `InstallingTorch` / `InstallingDeps`.
- **Adapter**: `InstallState` (`Idle`/`Running`/`Failed`), `install(offline)`
  treibt `install::install` + schreibt `runtimes`-Zustand (`installing` →
  `stopped`+Version bzw. `error`). `detail()` rendert die Phase
  („downloading ComfyUI — 42%" / „installing PyTorch (this is a big download)…"
  / „setup failed: …"). `App::comfyui` typisiert gehalten.
- **API/UI**: `POST /runtimes/comfyui/install` (202/200) + Tauri-Command
  `install_comfyui`. `LlamaSetup` → generisches **`RuntimeSetup`** in Diagnostics,
  jetzt für llama.cpp *und* ComfyUI.

Verifiziert:
- 9 neue Unit-Tests (4 `download`: hex/extract/extract-flat/bad-hash; 5
  `comfyui::install`: offline-refusal, idempotent, volle Pipeline mit
  Runner-Reihenfolge + Phasen, fehlschlagender `uv`-Schritt, Source-Hash-Abbruch
  vor den `uv`-Schritten). `check.ps1` grün (185 Unit + 17 Integ.).
- **Smoke (`#[ignore]`, echt)**: `comfyui::install::tests::real_pinned_install`
  zieht das echte `uv` + die echte ComfyUI-Quelle + einen **echten CUDA-torch-
  Build** und importiert `torch` in der frischen venv — **grün in 73 s** (der
  cu130-Wheel passt zu Python 3.13). GPU-Treiber-Kompatibilität wird endgültig
  bewiesen, wenn 3.4 ein Bild rendert.
- **Live** (`aiwm-cored`): `GET /runtimes` → `comfyui: not installed` →
  `POST …/install` → 202 → `detail` zeigt „downloading ComfyUI — 0%" → erneuter
  `POST` während des Laufs → 400.

Bewusst **nicht** in 3.2a: der Custom Node (→ 3.2b), Speicherplatz-Check vor dem
Download (Phase 6), Cleanup alter `<tag>/` + `uv-cache/` + `python/` beim
Versions-Bump ([TODO.md](TODO.md)).

---

## Bewusst nicht in Phase 3

Image→Image / Inpaint / Upscale / Enhancement (ROADMAP nennt sie — kommen als
eigene Slices *nach* txt2img steht oder in Phase 3.x), ControlNet/IPAdapter,
LoRA-Stacking-UI, Prompt-Styles-Bibliothek, Batch/Queue-Größe > 1, Online-Modell-
Discovery (Phase 6), Video (Phase 4), Graph-Editor.
