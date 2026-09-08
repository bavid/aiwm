# Phase 2 — MVP: Plattform-Kern

Erste echte Capability: **Chat** über eine lokale llama.cpp-Runtime. In Scheiben,
jede für sich testbar.

| Scheibe | Inhalt | Status |
|---|---|---|
| **2.1** | `ModelRepo` (CRUD + Rollen) · GGUF-Header-Inspektion · manueller Import in den kanonischen Store · API + UI-Tab „Models" | ✅ |
| 2.2 | `LlamaCppAdapter`: gepinnte llama-server-Version herunterladen/einrichten (CUDA-Build), Health-Check, Start/Stop über `RuntimeSupervisor`; Attach-Fallback auf laufenden Port | offen |
| 2.3 | Link-Manager: kanonische Datei ↔ Runtime via NTFS-Junction (ADR-007); `model_links`-Tabelle | offen |
| 2.4 | Chat-Job: `job_type=chat`, Modell (explizit oder `Auto`) → Scheduler → llama-server laden → Prompt → Antwort streamen; `job_events` + Cancel | offen |
| 2.5 | UI: „Chat"-Capability-Button aktiv, einfache Prompt/Antwort-Oberfläche; Job-Fortschritt aus dem Event-Stream | offen |
| 2.6 | Kompatibilitäts-Check vor dem Laden (VRAM-Budget vs. `vram_estimate_mb` + KV-Cache-Schätzung), Klartext-Fehler | offen |
| 2.7 | Settings-UI (Store-Pfad, Offline-Schalter, Theme), Diagnostics erweitert | offen |

**Phase-2-DONE-Kriterium:** frisches Windows → App → llama.cpp wird eingerichtet
→ GGUF importieren → Chat-Job läuft → zweites Modell → Wechsel ohne manuelles
VRAM-Management → Netz trennen → läuft weiter.

---

## 2.1 — Ergebnis (abgeschlossen)

- `db::ModelRepo` — `insert` (mit Rollen, in einer Transaktion, dedupe+sort) /
  `get` / `list` (Rollen via zweite Query gejoined) / `find_by_sha256` /
  `find_by_path` / `delete` (FK-Cascade auf `model_roles`) / `mark_used` /
  `roles`. `models.file_path` ist UNIQUE.
- `model::gguf` — **eigener, bounded GGUF-Header-Reader** (kein Fremd-Crate für
  untrusted Binärdaten): Magic/Version, Metadaten-KVs (alle Typen, Arrays werden
  *übersprungen* statt materialisiert), Tensor-Infos für die exakte
  Parameterzahl. Alle Counts/Längen range-checked. Liefert `architecture`,
  `name`, `quantization` (aus `general.file_type`), `context_length`,
  `parameter_count`.
- `model::import` — `import_model(db, store_root, ImportRequest)`:
  validieren (`.gguf`) → SHA256 (streaming, `spawn_blocking`) → GGUF inspizieren →
  Dedupe über SHA256 (No-op wenn schon da) → Zielpfad
  `<store>/llm/<slug>/<originalname>` → move (oder copy bei `keep_original`,
  cross-volume-Fallback) → `ModelRepo::insert`. `vram_estimate_mb` =
  Dateigröße + 1 GB Headroom (klar als Schätzung markiert).
- API: `GET /models` (jetzt echt), `POST /models` (201 / 200 bei Dedupe);
  Tauri-Commands `list_models` / `import_model`.
- UI: Tab **„Models"** — Import-Formular (Pfad, Rollen-Chips, keep-original) +
  Model-Tabelle (Name, Arch, Quant, Params, Größe, Ctx, VRAM-Schätzung, Rollen).

Verifiziert:
- 16 neue Tests (6 ModelRepo, 5 GGUF-Parser inkl. truncated/garbage/implausible,
  4 Import inkl. move/copy/dedupe, 1 slugify).
- Live: `POST /models` mit einer handgebauten Mini-GGUF → korrekt geparst
  (`llama` / `Q4_K_M` / param_count aus Tensoren summiert / ctx 4096), Datei in
  den Store verschoben, `GET /models` liefert sie zurück.

Zusatz: `rust-version` von 1.80 auf **1.85** angehoben (`ErrorKind::CrossesDevices`).

Offen für 2.2+: `safetensors`-Import, GGUF-Metadaten für Multimodal/Embedding,
publisher aus HF-Repo-Struktur, RAM-Schätzung.
