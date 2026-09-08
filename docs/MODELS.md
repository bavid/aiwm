# Models

Wie das Tool Modelle verwaltet. Stand + Plan.

## Abstraktionskette (Brief 10.22)

```
Model → Capability → Runtime → Pipeline → Job → Hardware
```

Ein Modell wird nicht direkt gewählt — der Nutzer wählt eine **Capability**, das
System schlägt passende Modelle/Pipelines vor.

## Datenmodell (Schema v1, seit WP-2)

`models` — id (uuid v7), publisher, name, family, format, quant, arch,
param_count, file_path (unique), sha256, size_bytes, ctx_max, vram_estimate_mb,
ram_estimate_mb, source, source_revision, imported_at, last_used_at, use_count,
`n_layers` / `n_embd` / `n_heads` / `n_kv_heads` (GGUF-Arch-Dims für die
VRAM-Schätzung, Migration 0004 / 2.6). `model_roles` (coding / chat / upscaler /
base_diffusion / …), `model_links` (pro Runtime: passthrough | junction |
hardlink | copy).

## Status

| Feature | Stand |
|---|---|
| Schema + Tabellen | ✅ WP-2 (+ `jobs.result`, Migration 0002, 2.4a) |
| `ModelRepo` (CRUD, Rollen, `find_by_*`, `pick_for_role`) | ✅ 2.1 / 2.4a |
| Manueller Modell-Import (GGUF wählen → Store) | ✅ 2.1 |
| `Auto`-Modellwahl (Rolle → zuletzt/meist genutzt → Name) | ✅ 2.4a (ADR-015); benchmark-gestützt erst Phase 6 |
| Kanonischer Store + Link-Manager | ✅ Store 2.1 · `core::link` 2.3 (Passthrough/Junction/Hardlink/Copy, `model_links`, ADR-007). GGUF → llama.cpp = `passthrough` |
| VRAM-Fit-Schätzung vor dem Load (`core::compat`) | ✅ 2.6 (ADR-016): Gewichte + KV-Cache aus GGUF-Arch-Dims + flacher Overhead, geschätzt für `min(ctx_max, 8192)`; Scheduler plant dagegen; passt es nicht → `blocked` mit Klartext. Kalibrierung → Phase 6 |
| Online-Discovery (HF Hub, Ollama-Library) | Phase 6 |
| Download-Manager (Queue, Resume, Verify, Speicherplan) | Phase 6 |
| Kompatibilitäts-Engine (🟢/🟡/🔴 vor Download) | Phase 6 (baut auf `core::compat` auf) |
| Dedup-/Unused-/Versions-Reports | Phase 6 |
| Benchmark-gestützte Auto-Auswahl | Phase 6 (braucht [BENCHMARKS.md](BENCHMARKS.md)) |

## Hardware-Realität

Welche Modellgrößen/Quantisierungen auf der RTX 4080 Super (16 GB) sinnvoll sind:
[HARDWARE.md](HARDWARE.md). Kurz: 7–14B komfortabel, ~24–30B an der Kante,
70B unrealistisch. Video: Kurzclips 480–720p.

## Zu untersuchen vor Phase 6 (Brief 10.24)

Model-Repository-APIs (HF Hub, Ollama-Registry), Download-/Resume-Support,
Metadaten-Umfang, Versionserkennung, Checksums, Quant-Erkennung aus
Dateiname + GGUF-Header, VRAM-Schätzformel + Kalibrierung, Trust-Bewertung von
Quellen. **Kein automatischer Download aus unbekannten Quellen.**
