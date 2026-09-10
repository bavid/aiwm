# Benchmarks

Modell-Bewertung. **Umgesetzt in Scheibe 6.5** (`core::bench`,
[PHASE_6_PLAN.md](PHASE_6_PLAN.md) → „## 6.5 — Ergebnis") — der „Test model"-Job
misst nur lokal; der „Overall Score" ist eine offen deklarierte Heuristik.

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
Qualitäts-Benchmark (ADR-024). Deshalb:

- **kein** gebündeltes externes Leaderboard im MVP (HF Open LLM Leaderboard
  abgeschaltet; GGUF-Quant → Leaderboard-Zeile ist unlösbar). Externer
  Score-Fetch = opt-in, Post-6.5, wenn eine tragbare Quelle auftaucht.
- der „Overall Score" ist eine **offen deklarierte gewichtete Heuristik** —
  kein Anspruch auf eine Qualitäts-Achse.

## Datenmodell (6.5)

`benchmarks`-Tabelle (Migration `0007`, STRICT): `id`, `model_id` (FK,
`ON DELETE CASCADE`), `job_id`, `kind` (`llm`; `image`/`video` später), `runs`,
`prompt_tps`, `gen_tps`, `load_ms` (NULL wenn schon resident), `vram_peak_mb`
(NULL ohne GPU), `ram_peak_mb`, `stability_score` (0–1), `overall_score`
(0–100), `notes`, `created_at`.

## Ablauf (6.5)

`job_type = bench` — ein Job wie „Modell testen": Scheduler lädt das Modell
(Ladezeit getimed), `core::bench::run` schickt einen festen Prompt N-mal durch
(`stream_completion`), liest die tok/s aus dem `Done`-Event, sampelt VRAM/RAM
aus der Telemetrie an den Lauf-Grenzen, schreibt eine `benchmarks`-Zeile.

- **`stability_score`** = `1 − Variationskoeffizient` der gen-tok/s, geklammert.
- **`overall_score`** = `(0,65·speed + 0,35·stability) · fit_faktor`, wobei
  `speed = gen_tps / SPEED_REF_TPS` (80, Faustwert) geklammert und `fit_faktor`
  Green 1,0 · Yellow 0,85 · Unknown 0,8 · **Red 0,35** (`core::compat::verdict`).
  Ein Modell, das nicht in den VRAM-Etat passt, wird hart gedeckelt.

## Nutzung (6.6)

`core::select::pick_for_role` bezieht die Daten in `Model: Auto` ein: **Fit
zuerst** (VRAM-Schätzung ≤ Budget) → **Score** (`selection_score`,
preference-gewichtet aus `gen_tps` / `stability` / Params) → **Nutzung**.
`[models].auto_preference` = `balanced` / `fast` / `quality`. Ohne jeden
Benchmark ist es exakt die alte „zuletzt/meist genutzt/Name"-Regel. Gilt für
`chat`, `coding`, `base_diffusion`, `base_video`.
