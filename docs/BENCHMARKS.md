# Benchmarks

Konzept für die Modell-Bewertung. **Noch nicht implementiert** — Phase 6,
Scheibe 6.5 ([PHASE_6_PLAN.md](PHASE_6_PLAN.md)).

## 6.0-Befund → ADR-024: keine gebündelte externe Benchmark-Quelle im MVP

- Das **HF Open LLM Leaderboard ist abgeschaltet** (v1 Juni 2024 archiviert, v2
  **März 2025**). Kein kanonischer Nachfolger — HF setzt auf dezentrale
  „Community Evals" (`eval.yaml` pro Repo) + 200+ Community-Leaderboards.
- Maschinenlesbare Alternativen sind heterogen und cloud-lastig: Aider-Polyglot
  (`Aider-AI/aider` → `polyglot_leaderboard.yml`, Apache-2.0, aber **Provider-
  API-Namen**, keine GGUF-Quant-IDs), SWE-bench (verstreut im `experiments`-
  Repo). **Das Matching „GGUF-Quant-Repo → Leaderboard-Zeile" ist der Blocker.**
- **Entscheidung:** `core::bench` misst **nur lokal**. „Overall Score" = offen
  deklarierte Heuristik aus lokaler Performance + Fit + objektiven HF-Signalen
  (Downloads/Likes/Recency/`base_model`-Lineage). **Keine „Qualitäts"-Achse**,
  die wir nicht belegen können. Externe Scores = opt-in, Post-6.5, wenn eine
  tragbare Quelle auftaucht (aussichtsreich: HF Community Evals — Daten hängen
  am Modell selbst).

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
