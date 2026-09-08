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
ram_estimate_mb, source, source_revision, imported_at, last_used_at, use_count.
`model_roles` (coding / chat / upscaler / base_diffusion / …), `model_links`
(pro Runtime: junction | copy | import).

## Status

| Feature | Stand |
|---|---|
| Schema + Tabellen | ✅ WP-2 |
| `ModelRepo` (CRUD, Rollen, Links) | Phase 2 (mit dem Importer) |
| Manueller Modell-Import (GGUF wählen → Store) | Phase 2 |
| Kanonischer Store + Link-Manager | Phase 2 (ADR-007) |
| Online-Discovery (HF Hub, Ollama-Library) | Phase 6 |
| Download-Manager (Queue, Resume, Verify, Speicherplan) | Phase 6 |
| Kompatibilitäts-Engine (🟢/🟡/🔴 vor Download) | Phase 6 |
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
