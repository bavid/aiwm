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
| **3.2b** | gepinnter Custom-Node-Satz — **exakt einer**: `city96/ComfyUI-GGUF` am Commit `6ea2651e` (verifiziertes `.zip` → `custom_nodes/ComfyUI-GGUF/`, `uv pip install -r <node>/requirements.txt` = `gguf`/`sentencepiece`/`protobuf` in die venv). Idempotenz + „installed" verlangen jetzt auch den Node. SDXL-txt2img braucht **keine** Custom Nodes | ✅ |
| **3.3** | Getypter Bild-Store `<store>/image/{checkpoints,diffusion_models,vae,loras,text_encoders}/`; `ModelKind` + `import_model` nimmt `.safetensors` (+ Pickle-Ablehnung), routet per Typ-Hint/Endung; ComfyUI-Zugriff über **`extra_model_paths.yaml`** statt Junction (Store auf `E:`, Runtime auf `C:` → Junction unmöglich; ADR-019); `model_links` = `comfyui`/`extra_path`; UI-Typ-Dropdown | ✅ |
| 3.4 | `capability::image`: `job_type=image`, Params (prompt, negative, w/h, steps, cfg, seed, model|Auto über Rolle `base_diffusion`); **feste Pipeline** = Workflow-JSON-Template + Param-Substitution (`core::pipeline`); `POST /prompt` → `/history/{id}` pollen → Bild via `/view` nach `<outputs>/<job_id>.png` → `jobs.output_path`; Cancel via `POST /interrupt`; VRAM-Schätzung pro Familie (Import-Zeit, Namens-Heuristik) | ✅ |
| 3.5 | UI: Tab „Image" — Prompt/Negativ, Größe/Steps/CFG/Seed, Model [Auto], „Generate"; Ergebnisbild; einfache **Galerie** (Bild-Jobs mit Thumbnail, Klick → Prompt/Seed/Modell); Dashboard-Button „Generate Image" aktiv; Bild via `GET /jobs/{id}/output` | ✅ |
| 3.6 | Zweites Template **Flux.1-dev (GGUF)** über `ComfyUI-GGUF` (`UnetLoaderGGUF` + `DualCLIPLoaderGGUF` + `VAELoader`, `FluxGuidance`, SD3-Latent); Companion-Auflösung (T5/CLIP-L/VAE per Rolle + Name); `docs/IMAGE_MODELS.md`; `core::model::catalog` + `GET /models/known` + „Known models"-Panel im Models-Tab | ✅ |
| 3.7 | Politur: `[comfyui]`-Config-Tabelle (VRAM-Modus `--*vram`) + Settings-UI, Diagnostics-Statuszeile (ComfyUI-Version + Modus), Output-Retention-Hinweis + `outputs_dir` in `/about`; `core/tests/llm_diffusion_swap.rs` (LLM ↔ Diffusion Wechsel unterm VRAM-Budget) | ✅ |

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

## 3.2b — Ergebnis (abgeschlossen)

- **`GGUF_NODE_ARCHIVE`** — `city96/ComfyUI-GGUF` am Commit
  `6ea2651e7df66d7585f6ffee804b20e92fb38b8a` (das Repo hat keine Tags),
  verifiziertes GitHub-`.zip` (SHA-256 selbst berechnet, gleicher Caveat wie die
  ComfyUI-Quelle). `TOOLCHAIN_DOWNLOAD_BYTES` schließt es ein.
- **`FetchSpec`** bündelt die drei Archive + Basis-URLs (statt sechs Extra-
  Parameter durch jede Signatur). `fetch_sources` lädt/entpackt alle drei
  (Node flach nach `<home>/custom_nodes/ComfyUI-GGUF/`).
- **`build_venv`** hängt einen Schritt an: `InstallPhase::InstallingNode` →
  `uv pip install --python <venv> -r custom_nodes/ComfyUI-GGUF/requirements.txt`
  (`gguf>=0.13.0` / `sentencepiece` / `protobuf`).
- **Idempotenz + „fertig"-Prüfung** verlangen jetzt auch `<node>/__init__.py` —
  ein Abbruch nach der venv, aber vor dem Node, wird beim nächsten Lauf
  fertiggestellt. `detail()` rendert „installing the GGUF node…".

Verifiziert:
- Unit-Test `full_install_fetches_all_three_then_runs_the_uv_steps_in_order`:
  vier `uv`-Aufrufe in Reihenfolge (`venv` → torch → `requirements.txt` →
  `custom_nodes/ComfyUI-GGUF/requirements.txt`), alle Phasen inkl.
  `InstallingNode`, Node flach entpackt. `check.ps1` grün (185 Unit + 17 Integ.).
- **Smoke (`#[ignore]`, echt)**: `real_pinned_install` zieht jetzt zusätzlich den
  Node und `import torch, gguf` läuft in der frischen venv — **grün in 70 s**.

---

## 3.3 — Ergebnis (abgeschlossen) · **Plan-Abweichung, siehe ADR-019**

Junctions gehen nicht: der Store liegt auf `E:`, die ComfyUI-Installation unter
`%LOCALAPPDATA%` (`C:`) — NTFS-Junctions überspannen keine Volumes. ComfyUIs
`extra_model_paths.yaml` (`--extra-model-paths-config`) ist der native Weg und
funktioniert über Volumes.

- **`core::model::ModelKind`** (neu): `Chat` / `Checkpoint` / `DiffusionModel` /
  `Vae` / `Lora` / `TextEncoder`. `from_hint` (mit Aliassen), `default_for_ext`,
  `accepts_ext`, `store_subdir()` (`"llm"` bzw. `"image/checkpoints"` …),
  `comfy_folder()` (ComfyUI-`folder_paths`-Schlüssel).
- **`import_model`** generalisiert:
  - nimmt `.gguf` **und** `.safetensors`; `.ckpt`/`.bin`/`.pt` → Klartext-
    Ablehnung („Pickle … convert to .safetensors first").
  - `ImportRequest.model_type: Option<String>` — Hint oder aus der Endung
    abgeleitet (`.gguf`→`chat`, `.safetensors`→`checkpoint`), gegen die Endung
    validiert.
  - GGUF-Header nur für `chat` geparst; `.safetensors` wird nicht geparst
    (kein sicherer bounded Reader — Header-Inspektion später).
  - Ziel: `chat` → `<store>/llm/<slug>/<file>` (wie bisher); Bild → flach in
    `<store>/image/<typ>/<file>` (ComfyUI-Konvention). Namens-Kollision → `-<hash8>`.
  - VRAM-Schätzung: Bild = `Dateigröße + 2 GB` Headroom; Chat weiter über `compat`.
  - Link: `chat` → `llamacpp`/`passthrough`; Bild → `comfyui`/`extra_path` mit
    dem *Ordner* als `link_path`.
- **`core::link`**: neue Variante `LinkStrategy::ExtraPath` (`materialize` =
  No-Op, gibt den kanonischen Pfad zurück). `strategy_for("comfyui", …)` →
  `ExtraPath`; `strategy_for` für alles andere ohne bekannten Namen weiter
  `Junction` (LM Studio o. Ä.).
- **`ComfyDirs.models_store`** (= `Config::store_path`). `ComfyDirs::ensure()`
  schreibt jetzt zusätzlich `<comfyui-data>/aiwm-model-paths.yaml`
  (`base_path: <store>/image` + fünf Ordner-Mappings) — bei **jedem** Server-Start,
  damit ein geänderter Store-Pfad greift. `build_spawn_spec` hängt
  `--extra-model-paths-config <die Datei>` an.
- **UI**: Import-Formular bekommt ein **Typ-Dropdown** (Chat / Checkpoint /
  Diffusion / VAE / LoRA / Text-Encoder); Rollen-Chips nur für Chat.

Verifiziert:
- 10 neue Unit-Tests (`ModelKind` 4, `import` 4 — safetensors→checkpoint+comfyui,
  expliziter Typ, Pickle/unbekannt abgelehnt, GGUF-als-VAE abgelehnt; `link` 2 —
  `ExtraPath` round-trip + `strategy_for`). `check.ps1` grün (195 Unit + 17 Integ.).
- **Live** (`aiwm-cored`, Fake-ComfyUI): SDXL-`.safetensors` → `<store>/image/
  checkpoints/…`, `runtimes: ["comfyui"]`, Link `extra_path`; expliziter
  `model_type:vae` → `<store>/image/vae/…`; Chat-`.gguf` → `<store>/llm/…` +
  `llamacpp`; ComfyUI-Start schreibt `aiwm-model-paths.yaml` mit `base_path:
  <store>/image` + allen fünf Ordnern.

Bewusst **nicht** in 3.3: `.safetensors`-Header-Inspektion (Arch/Precision aus
dem JSON-Header — später), echtes Junctionen gegen LM Studio (Phase 3+),
Modell-Rollen für Bild (`base_diffusion` etc. — 3.4, wenn `Auto` sie braucht).

---

## 3.4 — Ergebnis (abgeschlossen)

Entscheidung **C** umgesetzt: **nur feste Pipelines**, ein Template
(`sdxl_txt2img`) — die kanonische ComfyUI-Default-Graph, braucht keine Custom
Nodes.

- **`core::pipeline`** (neu): `sdxl_txt2img(&Txt2ImgInputs) -> serde_json::Value`
  baut die API-Format-Graph (`CheckpointLoaderSimple` → 2× `CLIPTextEncode` →
  `EmptyLatentImage` → `KSampler` → `VAEDecode` → `SaveImage`), Params in die
  bekannten Node-Slots substituiert. Flux + eine TOML-Registry kommen mit 3.6.
- **`ComfyClient`** (3.1 erweitert): `submit_prompt` (`POST /prompt` →
  `prompt_id`, `node_errors` werden zu Klartext), `history` (`GET /history/{id}`
  → `Pending` / `Done(Vec<ImageRef>)` / `Failed(msg)`), `view` (`GET /view` →
  Bytes). `interrupt` ist jetzt live (Cancel-Hook).
- **`ComfyUiAdapter::generate_image(workflow, cancel) -> Option<GeneratedImage>`**
  (analog `LlamaCppAdapter::stream_completion`): Prompt einreihen, alle 750 ms
  `/history` pollen, bei Fertigstellung das erste Bild via `/view` holen.
  `None` = mittendrin gecancelt (`POST /interrupt`, Server bleibt oben).
  Timeout 600 s → Job schlägt fehl statt zu hängen.
- **`capability::image`**: `ImageRequest::from_params` (nur `prompt` Pflicht;
  w/h auf Vielfache von 8 gerundet + geklemmt, steps/cfg geklemmt, fehlender
  Seed → zufällig, JSON-sicher < 2⁵³). `run(...)` baut die Pipeline, ruft
  `generate_image`, schreibt `<outputs>/<job_id>.<ext>`, Event-Trail. Der
  aufgelöste Request (konkreter Seed) wird via `JobRepo::set_params` in die
  Job-Params zurückgeschrieben — reproduzierbar, und die Galerie (3.5) hat
  konkrete Zahlen.
- **`JobEngine`**: `resolve_image_target` — explizites Checkpoint oder `Auto`
  (`pick_for_role("base_diffusion")`, most-recently-used). VRAM-Reservierung =
  der Import-Schätzwert des Checkpoints (kein KV-Cache). Image-Body im
  `try_drive` nach `Running`; `output_path` wandert über den `Post→Completed`-
  Patch in die DB. Der Scheduler ist modalitäts-agnostisch — ein Image-Job
  evictet bei Bedarf das residente LLM (und umgekehrt).
- **`import_model`** (3.3-Nachzug): `ModelKind::default_role()` → `Checkpoint` /
  `DiffusionModel` bekommen `base_diffusion`; Familie + VRAM-Headroom aus einer
  Datei-Namens-Heuristik (`flux`/`sd3` → +3 GB, `xl` → +2 GB).
- **`aiwm-fake-comfy`**: `/prompt` / `/history/{id}` / `/view` (1×1-PNG) +
  `--fake-render-ms` / `--fake-history-error` für die Poll- / Cancel- / Fehler-
  Tests.

Bewusst **nicht** in 3.4: die UI (→ 3.5), `/ws`-Fortschritt (MVP pollt), SDXL-
Refiner-Pass, Batch > 1, echte per-Familie-VRAM-Zahlen (brauchen
Header-Inspektion + Kalibrierung, Phase 6). Ein HTTP-Endpunkt fürs Bild kam
dann doch in 3.5 (siehe unten).

Verifiziert:
- 16 neue Unit-Tests (`pipeline` 2, `capability::image` 6, `ComfyClient` 5,
  `ModelKind::default_role` 1, `import` 1 Flux-Familie, `JobRepo::set_params` 1).
  6 Integrationstests `tests/image_job.rs` gegen den Fixture-Subprozess: Auto-Job
  → PNG auf der Platte + gepinnter Seed, explizites Checkpoint, fehlender Prompt,
  kein Modell, ComfyUI-Execution-Error, Cancel mitten im Rendern.
  `check.ps1` grün (211 Unit + 23 Integ.).
- **Live** (`aiwm-cored` + Fake-ComfyUI über `AIWM_COMFYUI_PYTHON`): SDXL-
  `.safetensors` importiert (`family=sdxl`, `roles=[base_diffusion]`), Auto-
  Image-Job `queued → preparing → completed`, ComfyUI lazy gestartet,
  `output_path` = `<outputs>/<job_id>.png` (echte PNG-Bytes), `params.seed`
  konkret; zweiter Job mit langsamem `/history` → `POST …/cancel` →
  `cancelled`, keine Datei.

---

## 3.5 — Ergebnis (abgeschlossen)

- **Bild-Auslieferung:** neue Loopback-Route **`GET /jobs/{id}/output`** →
  `handlers::job_output_path` (validiert: Job existiert, hat `output_path`, Datei
  liegt kanonisiert **innerhalb** `outputs_dir`, sonst 404) → serviert die Bytes
  mit `image/png` + `Cache-Control: no-store`. Kein Tauri-Command — die UI setzt
  `<img src="http://127.0.0.1:<port>/jobs/<id>/output">`. `about.core_api_port`
  liefert den Port; die **CSP** in `tauri.conf.json` bekam
  `img-src 'self' data: http://127.0.0.1:* http://localhost:*`.
- **UI-Tab „Image"** (`ui/src/features/image/`): Sidebar-Formular
  (Prompt, Negativ, Größe-Presets Square/Portrait/Landscape + ↔-Swap,
  W/H/Steps/CFG als Zahlenfelder, Seed-Feld mit „random"-Default, Model-Select
  „Auto" + alle `base_diffusion`-Checkpoints), „Generate". Danach `jobDetail`-
  Polling (700 ms, wie Chat) → Result-Panel zeigt Zustand, dann das Bild + Meta
  (Prompt / Negativ / Modell-Name / Größe·Steps·CFG / Seed mit „reuse"). „Stop"
  während des Renderns. Der aufgelöste Seed steht in `job.params` (3.4), also
  zeigt die UI konkrete Zahlen und kann sie wieder einreihen.
- **Galerie**: aus `useJobs()` gefiltert (`job_type=image`, `completed`,
  `output_path`), Thumbnail-Grid (`object-fit: cover`, 1:1), Klick → Result-Panel
  mit den Params dieses Jobs. Aktualisiert sich über das 2-s-Job-Polling.
- **Dashboard**: `onOpenChat` → generisches `onNavigate(tab)`; „Generate Image"
  ist aktiv und springt in den Image-Tab. Neuer Tab in der Top-Nav.
- `ipc.ts`: `ImageParams`-Typ + `imageOutputUrl(port, jobId)`.

Verifiziert:
- 1 neuer Integrationstest (`api::tests` — `GET /jobs/{id}/output` serviert die
  PNG mit `image/png`, 404 für einen Chat-Job / eine unbekannte id).
  `check.ps1` grün (212 Unit + 23 Integ.), `tsc` + `eslint` sauber.
- **Live** (`aiwm-cored` + Fake-ComfyUI): Image-Job über `POST /jobs` →
  `GET /jobs/<id>/output` liefert `200 image/png` an genau der URL, die der
  `<img>`-Tag baut; `job.params` trägt prompt/negative/w/h/steps/cfg/seed;
  Chat-Job → 404.
- **UI** (Vite-Dev-Server, Browser): der Image-Tab rendert vollständig — Sidebar-
  Formular, Presets schalten W/H, „Generate" aktiviert sich mit einem Prompt,
  Result-Panel + Galerie-Platzhalter; Layout zweispaltig ab ~860 px, darunter
  gestapelt. (Der Live-Roundtrip braucht das echte Tauri-Fenster — die
  `invoke`-Aufrufe laufen nicht im reinen Browser.)

Bewusst **nicht** in 3.5: `/ws`-Fortschrittsbalken (MVP pollt), Galerie-
Paginierung / Löschen / Download-Button, Bild-Zoom/Lightbox, Prompt-History,
Style-Presets. Kommen bei Bedarf als eigene kleine Slices.

---

## 3.6 — Ergebnis (abgeschlossen)

Zweites Template + der Katalog. `core::pipeline` trägt jetzt zwei Templates —
die TOML-*Registry* (user-editierbare Definitionen) bleibt bewusst draußen.

- **`core::pipeline`** umgebaut: `sdxl_txt2img` → **`checkpoint_txt2img`**
  (jedes Single-File-Checkpoint, nicht SDXL-spezifisch) +
  **`flux_txt2img`**. `Recipe::for_family(family) -> Checkpoint | FluxGguf`
  wählt. `Txt2ImgInputs` trägt nur noch Prompt/Sampling-Params; Modell-Dateien
  kommen getrennt (`checkpoint: &str` bzw. `FluxModels { unet, t5, clip_l, vae }`).
- **Flux-Graph** (recherchiert gegen `city96/ComfyUI-GGUF` + HF): `UnetLoaderGGUF`
  + `DualCLIPLoaderGGUF` (type `flux`, T5 + CLIP-L; ComfyUI mappt den
  `clip`-Ordner-Key auf `text_encoders`) + `VAELoader`; `FluxGuidance` auf der
  positiven Conditioning; `EmptySD3LatentImage`; `KSampler` bei **CFG 1**,
  Sampler `euler`, Scheduler `simple`. Das UI-„CFG"-Feld wird für Flux zu
  **Guidance** (1–10, ≈ 3–4), der Sampler läuft immer bei 1.
- **Companion-Auflösung** (`capability::image::resolve_flux_companions`): über
  Rolle (`text_encoder` / `vae`) + Namens-Heuristik (`t5` → T5, `clip` ohne
  `t5` → CLIP-L). Fehlt eine Datei → Klartext-Fehler statt kryptischem
  Node-Fail. `ModelKind::default_role()` → `Vae` → `vae`, `TextEncoder` →
  `text_encoder` (damit `for_role` sie findet — neue `ModelRepo::for_role`).
- **`core::model::catalog`** (neu): `KnownModel` (id, name, kind, family,
  publisher, repo, file, url, sha256, size, license, note) + `KNOWN_MODELS`
  (SDXL + Flux-Stack, 7 Einträge, echte HF-SHA-256/Größen) +
  `find_by_sha256`. `import_model`: bei SHA-Treffer werden
  `publisher`/`family`/`source_revision = catalog:<id>` gestempelt.
  `GET /models/known` + Tauri-Command `list_known_models`.
- **UI**: Models-Tab bekommt einen **„Known models"**-Abschnitt (Name, Badges
  Typ/Familie, Notiz, Datei·Größe·Lizenz, „Set import type" + „Copy link").
  `modelType`-State nach `Models()` hochgezogen. Image-Tab: „CFG" → „Guidance"
  + Hinweis, wenn ein Flux-Modell explizit gewählt ist. Model-Library-Tabelle:
  Spalte „Arch" → „Family", horizontal scrollbar.
- **`import`-VRAM**: `HEAVY_IMAGE_HEADROOM_MB` 3072 → **2560** (der T5 wird
  ausgelagert, ist beim Sampling nicht resident) — Flux Q8 (~12,1 GiB + 2560)
  passt jetzt unter das 14 848-MB-Budget.

Verifiziert:
- 15 neue Unit-Tests (`pipeline` 6 — Recipe + beide Graphen + Guidance-Clamp;
  `catalog` 3; `capability::image` 3 — Encoder-Heuristik + Companion-Auflösung;
  `ModelKind::default_role` erweitert; `ModelRepo::for_role` 1; `api` 1 —
  `GET /models/known`). 2 neue Integrationstests `tests/image_job.rs` (Flux ohne
  Companions → Klartext-Fehler; Flux mit allen vieren → `Completed`).
  `check.ps1` grün (**223 Unit + 25 Integ.**), `tsc` + `eslint` sauber.
- **Live** (`aiwm-cored` + Fake-ComfyUI): `GET /models/known` liefert die 7
  Einträge mit 64-Hex-SHAs; Flux-Stack importiert (`family=flux`,
  `roles=[base_diffusion]` / `text_encoder` / `vae`); Image-Job → `auto-selected
  flux1-dev-Q8_0` → Event „Flux — T5 … CLIP-L … VAE …" + „guidance 3.5" →
  `completed`, PNG über `/jobs/<id>/output`; Flux-Job ohne T5 → `failed` mit
  „Flux needs a T5 text encoder — import …".
- **UI** (Vite-Dev-Server): „Known models"-Panel rendert (Badges, Aktionen,
  Datei-Zeile) im App-Stil.

Bewusst **nicht** in 3.6: TOML-Pipeline-Registry (2 Templates reichen noch
hartkodiert), `.safetensors`-Flux ohne GGUF, echte Verprobung des
`DualCLIPLoaderGGUF`-Ordner-Key-Mappings gegen die **echte** ComfyUI (mit dem
cu130-Treiber-Check zusammen, real), SD 3.5.

---

## 3.7 — Ergebnis (abgeschlossen) · **Phase 3 damit fertig**

- **`[comfyui]`-Config** (`ComfyConfig { vram_mode }`, `#[serde(default,
  deny_unknown_fields)]`, Default `"auto"`, validiert gegen `auto` / `highvram` /
  `normalvram` / `lowvram` / `novram`). `to_options() -> ComfyOptions
  { vram_mode: VramMode, extra_args }` — die neuen Runtime-Typen
  (`crate::runtime::{ComfyOptions, VramMode}`), analog `LlamaServerOptions`.
- **`ComfyUiAdapter::with_options`** — `build_spawn_spec` hängt den
  `--<mode>vram`-Flag (`Auto` → keiner) + `extra_args` an. `App::load` reicht
  `config.comfyui.to_options()` durch. Neustart-pflichtig (ADR-017, das
  Settings-Banner sagt es).
- **Diagnostics-Statuszeile**: `health()` ruft jetzt `GET /system_stats` direkt
  (war implizit über `client.health`) und **cacht** das Ergebnis; `detail()`
  zeigt `running on :<port> · ComfyUI <version> · <vram-mode> · model reserved`.
- **Output-Retention**: `AboutDto.outputs_dir` neu; Settings-Tab „Generated
  images"-Karte (Pfad + Hinweis „werden nicht automatisch gelöscht"),
  Diagnostics-Environment + „Copy" zeigen `outputs`.
- **Settings-UI**: neue „ComfyUI"-Sektion (VRAM-Modus-`<select>`) + „Generated
  images"-Karte; `ConfigUpdate.comfyui` (`#[serde(default)]` — alte Clients ok).
- **`core/tests/llm_diffusion_swap.rs`** — das Phase-3-DONE-Kriterium als ein
  Test: Chat-Modell resident → Bild-Job braucht VRAM → die Engine evictet das
  LLM (stoppt `llama-server`) und reserviert ComfyUI **selbst** → Bild
  gerendert → zweiter Chat-Job evictet ComfyUIs Reservierung zurück. Alle drei
  `Completed`, kein `blocked`, kein Mensch. (Analog `model_swap.rs` für
  Chat↔Chat.)

Verifiziert:
- 6 neue Unit-Tests (`config` 2 — `[comfyui]` round-trip + Validierung + „ohne
  `[comfyui]`-Tabelle lädt trotzdem"; `launch` — VRAM-Flag-Anhang + `Auto` fügt
  nichts an; `comfyui`-Adapter-Test angepasst; `api` — `comfyui` im
  `/config`-round-trip + 400 bei schlechtem Modus). 1 neuer Integrationstest
  (`llm_diffusion_swap.rs`). `check.ps1` grün (**225 Unit + 26 Integ.**), `tsc`
  + `eslint` sauber.
- **Live** (`aiwm-cored` + Fake-ComfyUI mit Argument-Spy): `GET /about` trägt
  `outputs_dir`; `[comfyui] vram_mode = "lowvram"` aus der `config.toml` wird
  gelesen; `PUT /config` persistiert `novram`, `turbo` → 400; die ComfyUI wird
  mit `--lowvram` gestartet; `GET /runtimes` detail =
  `running on :49646 · ComfyUI 0.34.0-fake · lowvram · model reserved`.
  (Der `PUT` nach dem Boot ändert die laufende Runtime nicht — Neustart-Semantik
  bestätigt.)
- **UI** (Vite-Dev, gemockter Bridge): Settings zeigt „ComfyUI" + „Generated
  images"; VRAM-Modus-Select ändert den Dirty-State / aktiviert „Save changes".

**Phase-3-DONE-Kriterium erreicht** (bis auf die reale ComfyUI, siehe „Offen").

---

## Offen nach Phase 3 (vor / mit Phase 4)

- **cu130-torch + Flux-GGUF-Graph gegen die *echte* ComfyUI**: (a) CUDA-Laufzeit
  auf der 4080 Super (der Installer-Smoke prüft nur `import torch`); (b)
  `UnetLoaderGGUF` / `DualCLIPLoaderGGUF` finden die Store-Ordner aus
  `extra_model_paths.yaml` (ComfyUIs `map_legacy` sollte es tun — im Quellcode
  geprüft, nicht live); (c) Steps/Scheduler/Guidance an einem echten Render
  kalibrieren. Fällt (a) aus → cu128/cu126-Pin.
- Output-Cleanup / Retention-Limit (nur Hinweis im MVP).
- `.safetensors`-Header-Inspektion (Familie/Precision/bessere VRAM-Schätzung,
  robustere Flux-Companion-Auflösung).

---

## Bewusst nicht in Phase 3

Image→Image / Inpaint / Upscale / Enhancement (ROADMAP nennt sie — kommen als
eigene Slices *nach* txt2img steht oder in Phase 3.x), ControlNet/IPAdapter,
LoRA-Stacking-UI, Prompt-Styles-Bibliothek, Batch/Queue-Größe > 1, Online-Modell-
Discovery (Phase 6), Video (Phase 4), Graph-Editor.
