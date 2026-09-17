# Benchmark Tab — Design (Chat/Coding tokens per second)

Status: written 2026-09-17 under the user's standing "work through the backlog" mandate. User
request (2026-09-17): "a test that determines how many tokens/sec I produce — chat/coding only
for now — simple: select model, run predefined test, measure output".

## Ziel

Ein eigener Tab „Benchmark": Modell wählen → vordefiniertes Test-Set wählen → Start → tok/s
sehen und mit früheren Läufen / anderen Modellen vergleichen. Offline, lokal, ohne neue
Laufzeitabhängigkeit.

## Was schon da ist (wird wiederverwendet, nicht neu gebaut)

`job_type=bench` (`core/src/bench/mod.rs`): lädt ein llama.cpp-Modell über die JobEngine
(VRAM-Plan, Evict, Cancel), misst `gen_tps`, `prompt_tps`, Kaltstart-Ladezeit, VRAM-/RAM-Spitze,
Stabilität, schreibt eine `benchmarks`-Zeile (Migration 0007). API: `POST /models/{id}/benchmark`,
`GET /models/{id}/benchmarks`, `GET /benchmarks`. Heute: **ein** fester Prompt, Ergebnis nur als
Chip in der Model Library.

## Entscheidungen

| Frage | Entscheidung |
|---|---|
| Test-Sets | Zwei eingebettete, **versionierte** Suites in `core/src/bench/suites.rs`: `chat-v1` und `coding-v1`, je 3 feste Prompts mit fester `max_tokens`. Eine Änderung an Prompts/Längen = neue Version (`chat-v2`), damit alte Zahlen vergleichbar bleiben. Selbst geschriebene Prompts — keine Aufgaben aus fremden Benchmarks kopiert (Lizenz + Kontamination). |
| Metrik | tok/s (Generierung) als Hauptzahl, dazu Prefill-tok/s, Ladezeit, VRAM-Spitze, Stabilität — alles, was `BenchReport` heute schon ehrlich misst. Pro Prompt ein Teilergebnis (Tokens erzeugt, tok/s), gespeichert als JSON. |
| Coding-Korrektheit (Tests ausführen) | **Nicht in dieser Phase.** Modell-generierten Code auszuführen braucht eine Sandbox (eigene Spec, Sicherheitsfrage). `coding-v1` misst Durchsatz auf Coding-Prompts (andere Token-Verteilung als Prosa), nicht Richtigkeit. Ehrlich so beschriftet (ADR-024: kein lokaler Qualitäts-Score). |
| Inspiration | `llama-bench` (pp/tg als Referenzmetrik, feste Längen, mehrere Wiederholungen); Aider-polyglot / HumanEval nur als Vorbild für die spätere Korrektheits-Phase. Kein Code übernommen. |
| Modelle | Nur llama.cpp/GGUF-Chatmodelle (wie der bestehende Job); andere Laufzeiten bekommen einen klaren Hinweis statt eines kaputten Buttons. |
| Persistenz | Migration 0017: `benchmarks.suite TEXT NULL`, `benchmarks.detail_json TEXT NULL`. Alte Zeilen (`suite NULL`) = „quick test" aus der Model Library, bleiben gültig. |
| Vergleich | Tabelle „letzter Lauf pro Modell für die gewählte Suite" mit horizontalen tok/s-Balken, plus Verlauf des gewählten Modells. Kein Netz-Leaderboard. |

## Abschnitt 1 — Core

- `bench::suites`: `Suite { id, title, description, max_tokens, prompts: &[SuitePrompt { id, title, text }] }`, `all()`, `find(id)`. Tests: Ids eindeutig, Prompts nicht leer, `max_tokens` in den Grenzen, Id endet auf `-vN`.
- `BenchRequest.suite: Option<String>` aus `params.suite`; unbekannte Suite → Job schlägt mit klarer Meldung fehl (kein stiller Fallback). Mit Suite: jeder Prompt `runs`-mal (Default 2, damit ein Lauf ≤ ~1–2 min bleibt), `max_tokens` aus der Suite. Ohne Suite: Verhalten wie heute (unverändert für die Model Library).
- `BenchReport.detail: Vec<PromptResult { prompt_id, tokens, gen_tps, prompt_tps }>`; `gen_tps` gesamt = Mittel über alle Pässe; Stabilität über alle Pässe.
- DB: Migration 0017, `NewBenchmark`/`Benchmark` um `suite`, `detail_json`; `benchmarks().list_all(suite, limit)`.
- API: `GET /bench/suites`; `POST /models/{id}/benchmark` nimmt optional `{ suite, runs }`; `GET /benchmarks/history?suite=&limit=`. Tauri-Commands + `ipc.ts` + Hooks + dev-mock spiegeln das.

## Abschnitt 2 — UI (`ui/src/features/benchmark/`)

Formular (Modell-Select nur GGUF-Chatmodelle, Suite-Select mit Beschreibung + Prompt-Liste,
Runs 1–5, Start/Stop) · Live-Zustand des laufenden Jobs (Events: „prompt 2/3, pass 1/2 — 71.3
tok/s") · Ergebnis-Karte (tok/s groß, Prefill, Ladezeit, VRAM, Stabilität, Teilergebnisse pro
Prompt) · Vergleichstabelle pro Suite mit Balken (tabular-nums, Balken relativ zum schnellsten)
· Verlauf des gewählten Modells. Leerzustände: kein GGUF-Modell → Hinweis auf Discover.
Hinweistext: „Misst Geschwindigkeit auf diesem Rechner, nicht Antwortqualität."

## Abschnitt 3 — Tests / Beweis

Unit (Suites, Request-Parsing, Report-Aggregation), DB-Roundtrip, `core/tests/bench.rs`
(Suite-Lauf über `aiwm-fake-llama`: N Prompts × Runs Pässe, `detail_json` gefüllt, unbekannte
Suite → failed), API-Test, UI live über dev-mock, **ein echter Lauf** mit einem installierten
GGUF-Modell, Zahlen in `docs/TODO.md`.

## Nicht enthalten

Code-Ausführung/Korrektheit, Judge-Modelle, Bild-/Video-Benchmarks, Batchgrößen-Sweeps,
Export/Leaderboard, andere Laufzeiten als llama.cpp.
