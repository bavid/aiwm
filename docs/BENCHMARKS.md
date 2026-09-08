# Benchmarks

Konzept für die Modell-Bewertung. **Noch nicht implementiert** — Phase 6.

## Zweck

Objektive(re) Daten für die automatische Modell-Auswahl (Brief 15, 32, 33),
statt Sortierung nach Popularität.

## Was lokal messbar ist

| Metrik | Quelle |
|---|---|
| tokens/sec (Prompt + Generation) | eigener Kurz-Benchmark im Tool |
| Modell-Ladezeit | Runtime-Adapter |
| VRAM-Peak | NVML während des Laufs |
| RAM-Peak | sysinfo |
| Stabilität (Crashes / OOM über N Läufe) | Job-History |

Für Bild/Video: Generierungszeit, VRAM, Auflösung.

## Was **nicht** lokal messbar ist

Modell-**Qualität**. Es gibt keinen billigen, lokalen, objektiven
Qualitäts-Benchmark. Lösung:

- extern gepflegte Benchmarks (SWE-bench Verified für Coding usw.) **online**
  ziehen und cachen
- der „Overall Score" ist eine **gewichtete Heuristik**, klar als solche
  gekennzeichnet — kein Anspruch auf Objektivität der Qualitäts-Achse

## Datenmodell

`benchmarks`-Tabelle (Schema-Erweiterung Phase 6): model_id, ts,
tokens_per_sec, load_ms, vram_peak_mb, ram_peak_mb, stability_score, notes.

## Nutzung

Ab Phase 6 fließen Benchmark-Daten in `Model: AUTO` ein. Bis dahin ist die
Auto-Auswahl regelbasiert (Aufgabe + VRAM-Budget + installierte Modelle).
