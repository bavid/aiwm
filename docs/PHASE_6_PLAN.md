# Phase 6 — Automatisierung & Model-Manager v2

Bisher ist der Modell-Umgang **manuell**: der Nutzer lädt eine Datei selbst von
Hugging Face, gibt den Pfad ein, `import_model` übernimmt sie in den Store. Der
`KNOWN_MODELS`-Katalog ist eine hand-gepflegte Handvoll Einträge. Es gibt keine
Suche, keinen Download, keine Benchmarks, keinen Versions-/Update-Check.

Phase 6 macht daraus einen **Model-Manager**: online suchen, verifiziert laden,
lokal messen, und — die neue Backlog-Idee — den **Upgrade-Check** („gibt es
inzwischen was Besseres, das auf dieser Hardware läuft?"). Alles offline-first
(ADR-009): jede Online-Funktion cached lokal und degradiert ohne Netz zu einer
Klartext-Meldung, nicht zu einem Fehler; `offline_mode` blockt hart.

Die Maschinerie trägt: `runtime::download::download_verified` (Streaming +
SHA-256, Phase 3.2), `core::compat` (VRAM-Schätzung, Phase 2.6), `ModelRepo` +
`ModelKind` + der getypte Store (ADR-019), `import_model`, das `job_events`-
Muster, das loopback-API-Muster, der Offline-Gate (`app.offline()` vor jedem
externen Call). Phase 6 fügt hinzu: `core::registry` (Quellen-Adapter), einen
Download-Manager mit Queue/Resume, `core::bench`, ein Discovery-UI, den
Upgrade-Check und Aufräum-Reports.

**Kein Anspruch auf einen objektiven Qualitäts-Score** (R12). Lokal messbar sind
nur Speed / VRAM / Ladezeit / Stabilität. „Qualität" kommt aus extern gepflegten
Benchmarks (Quelle offen — 6.0) und ist immer als **gewichtete Heuristik**
gekennzeichnet.

**Phase-6-DONE-Kriterium:** frisches Windows → App → Models-Tab → „Discover" →
`coding` + VRAM-Budget → Trefferliste von Hugging Face (Name, Params, Downloads,
Lizenz, 🟢/🟡/🔴-Fit) → einen wählen → „Download & import" → der Download-Manager
lädt verifiziert (SHA-256 aus `lfs.oid`, Resume nach Netz-Abbruch) und
`import_model` übernimmt ihn in den Store → „Test model" misst tok/s + Ladezeit +
VRAM-Peak → bei einem installierten Modell „Gibt es was Besseres?" → das lokale
LLM rankt echte HF-Kandidaten, die in 16 GB passen, mit Ein-Satz-Begründung →
Reports zeigen 1 Duplikat + 1 ungenutztes Modell → Netz trennen → Discovery zeigt
den letzten Cache + „offline" statt Fehler.

| Scheibe | Inhalt | Status |
|---|---|---|
| **6.0** | **Registry-/Benchmark-Spike** (Voraussetzung, wie 5.0 / 4.0, **kein Feature-Code**): HF-Hub-API real testen, Ollama-Library prüfen, Benchmark-Datenquelle festlegen. Ergebnisse in `MODELS.md` / `RUNTIMES.md` / `BENCHMARKS.md`, **ADR-022** (Registry) + **ADR-024** (Score-Heuristik). Schließt R9 / R10 / „Zu untersuchen vor Phase 6". | ✅ *(siehe „## 6.0 — Ergebnis")* |
| **6.1** | **`core::registry` — HF-Hub-Quellen-Adapter** (read-only, gegen ein Fixture): `ModelSource`-Trait + `HuggingFaceSource`, `Registry`-Wrapper mit TTL-JSON-Cache (`<data>/cache/registry/`) + `Freshness` (Live/Stale/Offline). SHA-256 aus `lfs.oid`. `aiwm-fake-hfhub`-Fixture + Integrationstest + `#[ignore]`-Live-Test. | ✅ *(siehe „## 6.1 — Ergebnis")* |
| **6.2** | **Discovery-UI** — `App.registry` + `GET /registry/search` + `GET /registry/models/{*id}` + 2 Tauri-Commands + DTOs (`FitLevel`, `RegistryFileDto` mit Fit + Download-Link, `RegistryDetailsDto`). „Discover"-Panel im Models-Tab (`Discover.tsx`): Suchfeld, „GGUF only", Sort; Ergebnis-Karten (id, Params, Downloads/Likes, Lizenz, Gated-Badge, `base_model`); „Files" → Dateiliste mit Quant + Größe + `🟢/🟡/🔴`-Fit-Punkt (via `core::compat`) + „Copy link" + „Set import type". `Freshness`-Banner bei Stale/Offline. `useRegistrySearch` (400 ms debounced). | ✅ *(siehe „## 6.2 — Ergebnis")* |
| **6.3** | **Kompatibilitäts-Engine v2** (`core::compat` + `core::model` erweitert — verlängert ADR-016). **6.3a**: `compat::verdict(dims, ctx, vram_budget, free_ram) -> FitVerdict { Green \| Yellow{reason} \| Red{reason} \| Unknown }` (Gewichte + KV + Overhead vs. VRAM-Budget **und** freier System-RAM für den Offload-Fall), in der „Discover"-Dateiliste. **6.3b**: bounded `.safetensors`-Header-Reader (`read_safetensors_info` — Param-Count + dominante Precision + `__metadata__`, der seit 3.3 vertagte TODO), im Import verdrahtet (nicht-lesbarer Header failt nicht); familien-bewusste `media_headroom_mb` statt der `+2,5 GB`-Faustregel. Konstanten in `HARDWARE.md` dokumentiert, echte Messkalibrierung wartet auf 4.0. | ✅ *(siehe „## 6.3 — Ergebnis")* |
| **6.4** | **Download-Manager** (`core::download` erweitert — ADR-023): `downloads`-Tabelle (Migration `0006`: id, url, dest, sha256, size, bytes_done, state {queued\|running\|paused\|verifying\|done\|failed}), Queue mit einem aktiven Slot, **Resume über HTTP-Range**, Verify gegen die erwartete SHA-256 (aus 6.1), dann Übergabe an `import_model`. `GET/POST /downloads`, `POST /downloads/{id}/{pause,resume,cancel}`, Fortschritts-Events (`job_events`-Muster). `offline_mode` → Ablehnung. UI: „Download & import"-Knopf auf den Discovery-Karten + eine Downloads-Liste. Speicherplanung: freier Platz auf dem Store-Volume vs. Download-Größe, Klartext-Warnung. | ✅ *(siehe „## 6.4 — Ergebnis")* |
| **6.5** | **`core::bench` — lokale Mikro-Benchmarks**: `benchmarks`-Tabelle (Migration `0006`: model_id, ts, tokens_per_sec, load_ms, vram_peak_mb, ram_peak_mb, stability_score, notes), ein „Test model"-Job pro Modell (kurzer Prompt → tok/s Prompt+Gen, Ladezeit vom Adapter, VRAM-Peak via NVML, RAM-Peak via sysinfo). Für Bild/Video analog: Generierungszeit + VRAM. Optionaler Fetch der externen Benchmark-Scores (Quelle aus 6.0), lokal gecacht. `Overall Score` = klar gekennzeichnete gewichtete Heuristik (lokale Perf + externer Score + Stabilität). UI: „Test"-Knopf + eine Score-Spalte in der Model Library. | ✅ *(siehe „## 6.5 — Ergebnis")* |
| **6.6** | **Benchmark-gestützte `Auto`-Auswahl**: `ModelRepo::pick_for_role` bezieht Benchmark-Daten ein (statt nur `last_used_at` / `use_count`) — Fit zuerst, dann Score, dann Nutzung. Gewichtung + „bevorzuge schnell / bevorzuge Qualität" in `[models]`-Config. Betrifft Chat, Coding, `base_diffusion`, `base_video`. Regelbasierter Fallback bleibt, wenn keine Benchmark-Daten da sind. | ✅ *(siehe „## 6.6 — Ergebnis")* |
| **6.7** | **Upgrade-Check** (die Backlog-Idee — `core::registry` + `core::compat` + lokales LLM): Knopf „Gibt es was Besseres?" pro installiertem Modell **und** pro Rolle. Ablauf: HF-Hub nach neueren/populäreren Modellen derselben Rolle+Familie fragen → `core::compat`-Fit-Filter auf „läuft in `vram_budget_mb`" **vor** der LLM-Bewertung (spart Tokens, hält die Liste ehrlich) → das lokale LLM (Rolle `chat`/`coding`) rankt die **echten API-Treffer** als JSON, schreibt eine Ein-Satz-Begründung, **darf keinen Modellnamen erfinden** (Antwort gegen die Kandidaten-IDs validieren) → Ausgabe: kurze Liste + „Download & import" (→ 6.4). „Besser" misst sich in der MVP-Fassung an **objektiven, abrufbaren** Signalen (Release-Datum, Downloads/Likes, größere/neuere Basis in der Familie, Fit) + ggf. externem Score (6.5); das LLM markiert „Qualität nicht lokal verifizierbar". HF-Query = externer Call → **per-Aktion-Consent**, im `offline_mode` gesperrt. ADR-025. | ✅ *(siehe „## 6.7 — Ergebnis")* |
| **6.8** | **Aufräum-Reports**: **Dedup** (gleiche SHA-256 / dieselbe Datei an mehreren Pfaden, inkl. per-Junction gebundener Ollama-Blobs — nur anzeigen, R4), **Unused** (nie genutzt / seit N Tagen nicht), **Old versions** (installiertes Modell hat eine neuere Katalog-/Registry-Revision — nutzt 6.1). Eine „Storage"-Ansicht: was belegt wie viel, was ist gefahrlos löschbar. Löschen bleibt eine bestätigte Nutzer-Aktion. | offen |
| **6.9** | **Collections + Politur**: benannte Modell-Sammlungen / Tags (`model_collections`), Discovery-Verlauf, Rate-Limit-Handling mit Backoff + `RateLimit`-Header, optionales `HF_TOKEN`-Feld in Settings (nie Pflicht — nur für Gated-Repos / hohe Limits), Diagnostics-Zeile für die Registry (letzter Fetch, Cache-Alter, Rate-Limit-Rest). | offen |

Cloud-Provider-Adapter (Claude/OpenAI als optionale **Agent**-Backends, opt-in)
ist in der ROADMAP unter Phase 6 gelistet, gehört aber thematisch zu Phase 5
(Agent-Runtimes) — **eigener ADR, nach Phase 6**, kein Model-Manager-Thema.

Nach Phase 6: Plugin-/Adapter-Plattform für Dritt-Quellen und Dritt-Runtimes
(erst wenn `ModelSource` + `RuntimeAdapter` als interne Schnittstellen stabil
sind), lokaler Repository-Index / Embeddings-Retrieval, LAN-/Remote-Zugriff.

---

## Research (2026-09)

### Hugging Face Hub API

- **Liste:** `GET /api/models?search=<q>&author=<a>&filter=<tag>&pipeline_tag=<t>&library=<l>&sort=downloads&direction=-1&limit=<n>&full=true`.
  Rückgabe pro Treffer: `id`, `author`, `downloads`, `likes`, `tags`,
  `pipeline_tag`, `library_name`, `createdAt`, `lastModified`, `gated`,
  `private`, `sha`, `siblings` (nur `rfilename`, **keine Größe**). `filter=gguf`
  greift; `full=true` bringt `siblings`/`tags`, aber **kein** `gguf`/`safetensors`.
- **Detail:** `GET /api/models/{id}?expand[]=gguf&expand[]=safetensors&expand[]=downloadsAllTime&expand[]=cardData&expand[]=gated&expand[]=lastModified`.
  `gguf` → `{ total (Param-Count), architecture, context_length, chat_template,
  bos_token, eos_token, totalFileSize }`. `safetensors` (wenn vorhanden) →
  `{ parameters: {…}, total }`. `cardData` → Lizenz + Card-Tags.
- **Dateigrößen + SHA-256:** `GET /api/models/{id}/tree/{rev}?recursive=true` →
  pro Datei `{ path, size, oid, lfs: { oid: "sha256:…", size, pointerSize } }`.
  **`lfs.oid` ist die SHA-256** — genau die, gegen die `download_verified`
  prüft. Damit steht der Verify-Hash **vor** dem Download fest.
  `GET …/treesize/{rev}/{path}` gibt die Ordner-Gesamtgröße.
- **Rate-Limits** (Stand 09/25, 5-Minuten-Fenster): Anonym **500 API / 3 000
  Resolver pro IP**, Free-Token **1 000 / 5 000**, PRO 2 500 / 12 000. `429`
  mit `RateLimit`-Header (`r=<rest>;t=<reset-s>`). Für Suche + Detail eines
  Upgrade-Checks (≈ 1 + 10 Calls) reicht **anonym** locker.
- **Auth:** anonym für öffentliche Repos; `Authorization: Bearer <HF_TOKEN>`
  nur für Gated/Private oder höhere Limits. Nie Pflicht.
- **Quant-Erkennung:** aus dem Dateinamen (`Q4_K_M`, `Q8_0`, `Q5_K_S`, `fp16`,
  `fp8_e4m3fn`, `bf16`) + GGUF-Header `general.file_type`. Split-GGUFs
  (`…-00001-of-00003.gguf`) als Gruppe behandeln.
- OpenAPI: `https://huggingface.co/.well-known/openapi.json` (+ `.md`).

### Ollama-Library

- **Kein offizieller Such-/Listen-Endpoint.** `registry.ollama.ai` folgt dem
  OCI-Distribution-Spec: `GET /v2/library/<model>/manifests/<tag>` liefert
  Layer-Digests + Größen für einen **bekannten** Namen, aber keine Suche.
- Discovery bräuchte HTML-Scraping von `ollama.com/library` oder eine
  Drittquelle (`ollamadb.dev/api/v1`, `frefrik/ollama-models-api`) — brüchig,
  keine Lizenz-Garantie. → **best-effort, zweiter Adapter, niedrige Priorität.**
- Ollama-Blobs sind SHA-256-benannt unter `~/.ollama/models/blobs/sha256-…` —
  relevant für den Dedup-Report (R4: Junction auf GGUF-Blobs möglich, auf das
  Ollama-Manifest nicht).

### Benchmark-Landschaft

- **Das HF Open LLM Leaderboard ist eingestellt.** Es gibt keine einzelne
  kanonische, frei abrufbare Quelle mehr.
- Kandidaten mit (teils) maschinenlesbaren Daten: **Artificial Analysis**
  (Intelligence Index), **LMArena** (Elo, Präferenz), **llm-stats.com**,
  **BenchLM**, **SWE-bench Verified** (offizielles JSON, Coding-Agenten),
  **Aider-Polyglot**, **LiveCodeBench**. Lizenz + Stabilität + Abdeckung
  gepinnter GGUF-Quants sind pro Quelle zu prüfen (6.0).
- **Empfehlung:** im MVP **keine** gebündelte externe Leaderboard-Quelle —
  lokale Mikro-Benchmarks (ehrlich, reproduzierbar) + die Katalog-Notizen;
  externe Scores als opt-in-Fetch, sobald 6.0 eine tragbare Quelle bestätigt.

### Wiederverwendung aus Phase 1–5

- `runtime::download::download_verified` (Streaming + SHA-256 + Größe, löscht
  Partials) → Kern des Download-Managers; nur Range/Resume + Queue fehlen.
- `core::compat` (Gewichte + KV-Cache aus Arch-Dims + Overhead) → Fit-Verdikt
  für Discovery + Upgrade-Check; braucht `.safetensors`-Inspektion + Kalibrierung.
- `core::model::gguf` (bounded Header-Reader) → Quant/Arch aus lokalen Dateien;
  dasselbe Muster für die Remote-`gguf`-Metadaten.
- `ModelRepo` + `model_roles` + `pick_for_role` → Ziel der benchmark-gestützten
  Auswahl; `catalog.rs` → mit Registry-Daten abgleichbar (Old-versions-Report).
- `job_events` / `JobEngine` → der „Test model"-Benchmark ist ein Job;
  Download-Fortschritt nutzt dasselbe Event-Muster.
- `app.offline()` + der Handler-Gate (`install_*` lehnt offline mit Klartext ab)
  → jeder Registry-/Download-Handler.
- `config.toml` + `[…]`-Tabellen (ADR-017) → `[models]` (Auto-Gewichtung,
  optionales `hf_token`).
- `core::backup` (5.5b) — das Modell-Manifest im Export wird mit Registry-Daten
  reicher (Revision, Upgrade-Hinweis beim Import auf einer zweiten Maschine).

---

## Offene Entscheidungen (Empfehlung → deine Freigabe)

**A — Quellen-Umfang MVP.**
→ **Empfehlung: nur Hugging Face Hub.** Offizielle API, großzügige Anon-Limits,
`lfs.oid` liefert den Verify-Hash vorab. Ollama-Library als späterer best-effort-
Adapter hinter derselben `ModelSource`-Schnittstelle (kein Such-API → Scrape).

**B — Externe Benchmark-Quelle.**
→ **Empfehlung: im MVP keine.** Lokale Mikro-Benchmarks sind der ehrliche Kern.
Eine externe Score-Quelle erst nach dem 6.0-Spike (Lizenz + stabiles JSON +
GGUF-Abdeckung), dann als opt-in-Fetch mit „Heuristik"-Kennzeichnung.

**C — HF anonym vs. Token.**
→ **Empfehlung: anonym per Default.** Optionales `HF_TOKEN` in Settings für
Gated-Repos / Nutzer, die an Limits stoßen. Nie Pflicht, nie im Backup-Export.

**D — Download-Manager: eigener Downloader vs. `huggingface_hub`.**
→ **Empfehlung: eigener** (Erweiterung von `download_verified` um Range/Resume +
Queue-Tabelle). Kein Python-Prozess für eine Kern-Funktion; der erwartete Hash
kommt aus 6.1.

**E — Upgrade-Check: welches Modell bewertet?**
→ **Empfehlung: das über die Rolle `chat`/`coding` ladbare lokale Modell.** Die
Aufgabe (≈ 10 JSON-Kandidaten ranken) ist leicht; kein dediziertes Analyse-
Modell. Strukturierte Ausgabe, IDs gegen die Kandidatenliste validiert.

**F — Woran misst „besser"?**
→ **Empfehlung: nur objektive, abrufbare Signale** — Release-Datum, Downloads/
Likes, neuere/größere Basis derselben Familie, Fit; optional externer Score
(B). Das LLM schreibt die Begründung und markiert explizit „Qualität nicht
lokal verifizierbar". Keine Qualitätsbehauptung ohne Beleg (R12).

**G — `ModelSource` als öffentlicher Plugin-Punkt?**
→ **Empfehlung: nein.** Ein sauberer `core::registry::ModelSource`-Trait (HF-
Impl jetzt, Ollama später), aber **kein** nutzer-ladbares Plugin-System — das
ist „nach Phase 6" (ROADMAP).

**H — Persistenz.**
→ **Empfehlung: Migration `0006`** mit `downloads` + `benchmarks` +
`model_collections` als STRICT-Tabellen. Remote-Metadaten-Cache als **TTL-JSON
unter `<data>/cache/registry/`** — wegwerfbar, nicht in der DB, nicht im Backup.

**I — Scope-Wächter (R1 / R2).**
→ Phase 6 ist bewusst groß. Reihenfolge 6.0 → 6.4 liefert schon den Kernnutzen
(suchen + verifiziert laden). 6.5–6.9 sind einzeln abschaltbar, falls der
Aufwand kippt. **Kein** generisches Scoring-Framework, **kein** Auto-Download
aus unbekannten Quellen, **kein** Plugin-System.

---

## 6.0 — Ergebnis (abgeschlossen) · **ADR-022 + ADR-024**

Live-Probes gegen die echte HF-Hub-API + `registry.ollama.ai` (2026-09,
anonym, read-only — kein Repo-Code). Bestätigt A/C/D aus den offenen
Entscheidungen, verfeinert B (→ ADR-024). Details in DECISIONS.md.

### HF-Hub-API — was wirklich geht

- **Ein Listen-Call reicht meist.** `expand[]` funktioniert **auch auf
  `GET /api/models`** (nicht nur auf dem Detail-Endpoint):
  `?search=&filter=&pipeline_tag=&library=&author=&sort=&direction=-1&limit=&expand[]=gguf&expand[]=safetensors&expand[]=gated&expand[]=lastModified&expand[]=downloadsAllTime&expand[]=trendingScore&expand[]=cardData`.
  Pro Treffer: `gguf` (`total` = Param-Count, `architecture`, `context_length`,
  `chat_template`, `bos/eos_token`, `totalFileSize`), `safetensors`
  (`parameters: { <DTYPE>: n }` → **Precision** `BF16`/`F16`/`F8_E4M3` + Param-
  Count **ohne Download**), `gated`, `lastModified`, `createdAt`,
  `downloadsAllTime`, `trendingScore`, `cardData` (u. a. `license`,
  `base_model`, `tags`).
- **Ohne `expand[]`** liefert die Liste nur `id`, `likes`, `downloads`, `tags`,
  `pipeline_tag`, `library_name`, `createdAt`, `private`, `modelId`. `tags`
  enthält aber schon `base_model:<id>`, `base_model:quantized:<id>` und
  `license:<slug>` als parsebare Strings.
- **`sort`:** `downloads`, `likes`, `likes7d`, `trendingScore`, `createdAt`,
  `lastModified` (+ `direction=-1`). **`filter=base_model:<owner/repo>`
  funktioniert** — der zuverlässige Weg zu allen Quant-Re-Uploads + Abkömmlingen
  eines bekannten Basismodells (Kern des Upgrade-Checks).
- **Verify-SHA-256 vor dem Download:** `GET /api/models/{id}/tree/{rev}?recursive=true`
  → pro Datei `{ path, size, oid (git-sha1), lfs: { oid, size, pointerSize }, xetHash }`.
  **`lfs.oid` = die SHA-256** (64 hex, auch auf Xet-Repos present). `xetHash`
  ist ein **anderer** Hash (Xet-Content-Addressing) — nicht verwenden. Split-
  GGUFs (`…-00001-of-00003.gguf`) + eine ggf. zusätzlich vorhandene Merge-Datei
  → als Set behandeln.
- **Gated-Repos:** Metadaten + `/tree` liefern **200 ohne Token**
  (`gated: "manual"|"auto"`); nur `/resolve/` (der Download) braucht akzeptierte
  Lizenz + Token → 6.4 muss `gated` erkennen und den Nutzer hinschicken, nicht
  mitten im Download scheitern.
- **Rate-Limits:** `RateLimit-Policy: "fixed window";"api";q=500;w=300` →
  **500 API-Calls / 5 min / IP anonym** (1 000 mit Free-Token), `RateLimit`-
  Header trägt Rest + Reset-Sekunden, `429` → Backoff auf `t`. **Kein
  `Cache-Control`**, aber schwacher `ETag` → `If-None-Match` beim Cache-Refresh.
- **Paginierung:** cursor-basiert über den `Link: <…cursor=…>; rel="next"`-Header.

### Ollama-Library

- **Kein Such-/Listen-API.** `GET https://registry.ollama.ai/v2/library/<model>/manifests/<tag>`
  (OCI, anonym) → `layers[]` mit `{ digest: "sha256:…", size }`; die
  `application/vnd.ollama.image.model`-Layer ist das GGUF — aber nur für einen
  **bekannten** Namen+Tag. `ollama.com/search?format=json` → HTML. Drittquellen
  (`ollamadb.dev`) DNS-tot / brüchig. → **best-effort, zweiter Adapter, später.**

### Benchmark-Landschaft → **ADR-024**

- **Das HF Open LLM Leaderboard ist abgeschaltet** (v2 seit März 2025). Kein
  kanonischer Nachfolger — HF setzt auf dezentrale „Community Evals"
  (`eval.yaml` pro Repo).
- Maschinenlesbare Alternativen sind heterogen + cloud-lastig: Aider-Polyglot
  (`polyglot_leaderboard.yml`, Apache-2.0, aber **Provider-API-Namen**, keine
  GGUF-Quant-IDs), SWE-bench (verstreut im `experiments`-Repo). Das Matching
  „GGUF-Quant-Repo → Leaderboard-Zeile" ist der eigentliche Blocker.
- **Entscheidung (ADR-024): kein gebündeltes externes Leaderboard im MVP.**
  `core::bench` misst nur lokal (tok/s, Ladezeit, VRAM/RAM-Peak, Stabilität).
  „Overall Score" = offen deklarierte Heuristik aus lokaler Perf + Fit +
  objektiven HF-Signalen. Externe Scores = opt-in, Post-6.5, wenn eine tragbare
  Quelle auftaucht (Kandidat: HF Community Evals).

### Konsequenzen für die weiteren Scheiben

- **6.1:** `HuggingFaceSource` — `search` = ein Listen-Call mit `expand[]`;
  `details` = derselbe + `/tree?recursive=true` für Größen/SHA-256. Cache mit
  `ETag`. `xetHash` ignorieren.
- **6.3:** `.safetensors`-Precision kommt aus der API (`safetensors.parameters`-
  DTYPE-Key) — die Header-Inspektion ist nur für **lokale, nicht via Registry
  importierte** Dateien nötig.
- **6.4:** `gated`-Erkennung + Split-GGUF-Sets + `If-Range`/Range-Resume;
  SHA-256 = `lfs.oid`.
- **6.7:** ~1–3 Calls (Basis-Modell aus `base_model:`-Tag → `filter=base_model:` +
  `sort=lastModified`/`likes7d` + `expand[]`), dann Fit-Filter, dann LLM-Ranking.
  **Spam-Filter nötig** (R9): Autor-Allowlist / `base_model`-Lineage gewichten,
  nicht nackte `trendingScore`.

---

## 6.1 — Ergebnis (abgeschlossen) · **ADR-022**

`core::registry` — der HF-Hub-Quellen-Adapter, nur der Adapter + der Cache-
Wrapper (API/UI = 6.2). **Nativer `reqwest`-Client, kein Sidecar** — der
Plan-Entwurf hatte `huggingface_hub` im Python-Sidecar angedacht (ARCHITECTURE
§3.3); die 6.0-Befunde (anonym, einfache GETs, `lfs.oid` gratis) machen einen
Rust-Client klar einfacher. ADR-022 hält das fest.

- **`registry/mod.rs`** — `ModelSource`-Trait (`search(&SearchQuery)` /
  `details(&str)`), die Typen (`RemoteModel` mit Params/`ctx_max`/Arch/Lizenz/
  `base_model`/Gated/Precision/Format, `RemoteFile` mit `size` + `sha256` +
  `quant` + `shard`, `RemoteModelDetails`), `SearchQuery`
  (`text` / `base_model` / `gguf_only` / `sort` / `limit`), `SearchSort`
  (Downloads/Likes/Trending=`likes7d`/RecentlyUpdated/RecentlyCreated).
  **`Registry`** = `Box<dyn ModelSource>` + Cache + `Arc<AtomicBool>` offline:
  `offline` → Cache oder Klartext-Ablehnung; online → Quelle, bei Transport-
  Fehler Fallback auf den Cache als `Freshness::Stale`, bei Erfolg
  Write-Through. `Fetched<T> { data, freshness }`, `Freshness`
  `Live | Stale{age_secs} | Offline{age_secs}`.
- **`registry/huggingface.rs`** — `HuggingFaceSource::{new, with_base_url,
  with_token}`. `search` baut **einen** `GET /api/models`-Call mit
  `expand[]=gguf,safetensors,gated,downloadsAllTime,lastModified,createdAt,
  trendingScore,cardData` + optional `search=` / `filter=gguf` /
  `filter=base_model:<id>`. `details` = `GET /api/models/{id}?expand[]=…` +
  `GET /api/models/{id}/tree/main?recursive=true`. `429` → Klartext-Fehler.
  Pure, einzeln getestete Parser: `parse_model` (Tags → `license:` /
  `base_model:` [bevorzugt die blanke Form, ignoriert `quantized:` etc.],
  `downloadsAllTime` vor `downloads`), `parse_tree` (**`sha256` nur aus
  `lfs.oid`, 64-hex-validiert — nie `oid` oder `xetHash`**),
  `quant_from_filename` (`Q4_K_M` / `IQ4_XS` / `F16` / `FP8`, Shard-Suffix
  vorher abgeschnitten), `shard_from_filename` (`-00001-of-00003` → `(1,3)`),
  `safetensors_precision` (dominanter DTYPE-Key), `gated_of`.
- **`registry/cache.rs`** — ein JSON-Envelope (`{stored_at, value}`) pro
  SHA-256-Schlüssel unter dem Cache-Ordner, atomarer temp+rename-Write. Jede
  Operation best-effort: Miss / korrupte Datei / nicht anlegbarer Ordner →
  „kein Cache", nie ein Fehler.
- **`AppPaths::cache_dir()`** = `<local_root>/cache` (wegwerfbar, nie geroamt,
  nie im Backup).
- **`bin/aiwm-fake-hfhub`** — axum-Fixture, zwei kanonische Repos (ein GGUF-
  Quant-Repo + sein Basismodell), honoriert `search` / `filter` / `limit`.
- **Tests:** 15 Unit (Parser, Cache, `Registry`-Fetch/Offline/Stale gegen eine
  `FakeSource`) + **5 Integration** (`core/tests/registry.rs`, echter
  Kindprozess + Loopback-HTTP: `search` parst die `expand[]`-Felder,
  `filter=base_model:` findet die Quant-Re-Uploads, `details` listet Dateien
  mit SHA-256 aus `lfs.oid` + Quant + Shard, `Registry` serviert den Cache
  offline, tote Quelle → `Stale`) + **1 `#[ignore]`-Live-Test** gegen das echte
  `huggingface.co` (lief einmal grün — der Parser passt zur echten API).
  **316 Lib + `registry.rs`** (5 + 1 ignored). `check.ps1` grün.
- Noch **nicht** an `App` / API / Tauri / UI verdrahtet — das ist 6.2.
  `HF_TOKEN` aus der Config, `ETag`/`If-None-Match` beim Refresh und der
  `RateLimit`-Header-Backoff kommen mit 6.9.

---

## 6.2 — Ergebnis (abgeschlossen)

Die Discovery-Vertikale: `core::registry` an `App` + API + Tauri + ein
„Discover"-Panel im Models-Tab. UI-only-Aktionen (kein Download-Manager bis
6.4).

- **`App.registry: Registry`** (in `App::load` gebaut: `HuggingFaceSource::new()`
  + `paths.cache_dir().join("registry")` + der `offline`-`Arc`). `App::with_registry`
  für Tests (zeigt auf `aiwm-fake-hfhub`). `AppPaths::cache_dir()` =
  `<local_root>/cache` (schon in 6.1).
- **Handlers** (`api/handlers.rs`): `registry_search(app, RegistrySearchDto) ->
  Fetched<Vec<RemoteModel>>` (der `Registry`-Wrapper regelt offline/stale),
  `registry_details(app, id) -> RegistryDetailsDto`. `enrich_file` hängt pro
  Datei den Browser-Link (`https://huggingface.co/<id>/resolve/<rev>/<path>`)
  und ein Fit-Verdikt an: für GGUF-/safetensors-Gewichtsdateien
  `compat::estimate(ModelDims { size_bytes: file.size, ctx_max, param_count })`
  gegen `scheduler.budget_mb()` → `FitLevel` (Green < 85 % Budget, Yellow bis
  100 %, Red darüber, Unknown sonst). **Erste Näherung** — 6.3 ersetzt das durch
  das echte `FitVerdict` mit `.safetensors`-Header, Aktivierungen, System-RAM
  und Begründung.
- **DTOs** (`api/dto.rs`): `RegistrySearchDto` (`q` / `base_model` / `gguf` /
  `sort`-String / `limit`) + `into_query()`, `FitLevel`, `RegistryFileDto`
  (`path`, `size_bytes`, `sha256`, `quant`, `shard: [u32;2]`, `download_url`,
  `vram_estimate_mb`, `fit`), `RegistryDetailsDto` (flattenes `RemoteModel` +
  `revision` + `files` + `freshness`).
- **HTTP** (`api/http.rs`): `GET /registry/search` (`Query<RegistrySearchDto>`)
  + `GET /registry/models/{*id}` (Wildcard — die id trägt ein `/`). **Tauri**:
  `registry_search(params)` / `registry_model(id)`.
- **UI**: `ipc.ts` — `Freshness`/`RemoteModel`/`RegistryFile`/`RegistryDetails`
  + `registrySearch`/`registryModel`. `hooks.ts` — `useRegistrySearch(params,
  enabled)` (400 ms debounce, nicht gepollt, erst ab 2 Zeichen). `Discover.tsx`
  (neu, `ui/src/features/models/`): Suchfeld + „GGUF only" + Sort → Ergebnis-
  Karten; „Files" lädt `registryModel` on-demand → Zeilen mit Fit-Punkt
  (`--load-ok`/`--load-warn`/`--load-crit`), Quant, Größe, „Copy link",
  Gated-Hinweis. `Stale`/`Offline` → gelber Banner. „Set import type"
  (`gguf`→`chat`, sonst `checkpoint`) setzt den Import-Typ oben.
  `Models.tsx` rendert `<Discover>`; `dev-mock` beide Commands.
- **Tests:** +4 Unit (`fit_of`-Schwellen, `is_weight_file`,
  `RegistrySearchDto::into_query` Sort-Mapping + Blank-Trim) + **1 Integration**
  (`discovery_endpoints_over_http` in `core/tests/registry.rs` — echter `App` +
  `ApiServer` + `aiwm-fake-hfhub`: `/registry/search` → `freshness: live` +
  geparste Felder, `/registry/models/{id}` → Dateien mit `download_url` +
  SHA-256 + `fit`, Nicht-Gewichtsdatei → `fit: unknown`). **320 Lib + 49 integ.**
  `check.ps1` grün, Browser-Smoke (dev-mock): Suche → Karten → „Files" zeigt
  Q4_K_M grün / Q8_0 gelb, keine Konsolenfehler.

---

## 6.3 — Ergebnis (abgeschlossen, a + b)

Die Kompatibilitäts-Engine v2 — der Fit-Verdikt mit Begründung (überall wo
Discovery ihn schon zeigt) und das `.safetensors`-Header-Wissen (der seit 3.3
vertagte TODO).

**6.3a** (`e3198f2`) — `FitVerdict`:
- **`compat::verdict(dims, ctx, vram_budget_mb, free_ram_mb) -> FitVerdict`**
  (`Green | Yellow{reason} | Red{reason} | Unknown`, `#[serde(tag="level")]`).
  Wickelt `estimate()`: **Green** unter 85 % des VRAM-Budgets; **Yellow** wenn's
  passt, aber eng ist (kein Puffer für längeren Kontext / ein zweites Modell);
  über Budget → **Yellow** wenn der freie System-RAM den Überhang + 4 GB
  OS-Reserve trägt (langsamer Offload), sonst **Red**. `Unknown` ohne Budget
  oder Größe. Jede Nicht-Green-Antwort trägt einen Klartext-Grund.
- **Discovery** (`registry_details` / `enrich_file`): der Ad-hoc-`dto::FitLevel`
  + `handlers::fit_of` fliegen raus; `compat::verdict` mit freiem RAM aus der
  Telemetrie (`ram_total - ram_used`). `RegistryFileDto.fit` ist jetzt der
  serialisierte `FitVerdict` (`{level, reason?}`).
- **UI**: `ipc.ts` `FitLevel` → `FitVerdict`-Union. `Discover.tsx`: Punktfarbe
  aus `fit.level`, ein „tight"/„won't fit"-Pill mit `fit.reason` als Tooltip.
- **+5 Compat-Unit-Tests** (green / tight-yellow / offload-yellow / red /
  unknown). `handlers::fit_of_thresholds` entfernt.

**6.3b** (dieser Commit) — `.safetensors`-Header:
- **`core::model::safetensors::read_safetensors_info(path) -> SafetensorsInfo`**
  — bounded Reader: 8-Byte-LE-Länge + range-checked (≤ 64 MiB, `+8 ≤ file_len`)
  JSON-Header → **Param-Count** (Σ `product(shape)` über echte Tensoren),
  **Precision** (dominanter DTYPE nach Param-Count, `F8_*`/`FP8*` → `FP8`),
  `__metadata__`-Strings (`modelspec.architecture`). Nie die Tensor-Bytes.
- **Import** (`media_new_model`): der `.safetensors`-Header füllt `param_count`,
  `quant` (= Precision) und `arch` (aus `__metadata__`); ein **nicht lesbarer
  Header failt den Import nicht** (`.ok()` → Fallback auf die Namens-Heuristik,
  GGUF-Parse bleibt fatal). `media_family` gibt nur noch die Familie zurück,
  `media_headroom_mb(family)` das VRAM-Polster: Wan 6 GB · LTX/Flux/SD3 4 GB ·
  SDXL 2 GB · sonst 2,5 GB — die `+2,5 GB`-Faustregel + die zwei alten
  `*_HEADROOM_MB`-Konstanten sind weg.
- **`docs/HARDWARE.md`** — neue „Estimator-Konstanten"-Tabelle
  (`RUNTIME_OVERHEAD_MB`, `KV_ROUGH_MB_PER_1K_CTX`, `FIT_TIGHT_PCT`,
  `OFFLOAD_RAM_RESERVE_MB`, `media_headroom_mb`) + Kalibrierungs-Plan
  (`nvidia-smi`-Peak bei 4.0 gegen `estimate()` halten).
- **+4 Reader-Unit-Tests** (Param-Count + dominante Precision + Metadaten,
  fp8→FP8, bogus-Länge abgelehnt, nicht-JSON abgelehnt) + **+1 Import-Test**
  (echter `.safetensors`-Header → `param_count` + `quant` gesetzt). **329 Lib +
  49 integ.** `check.ps1` grün.
- **Nicht** in 6.3: `FitVerdict` in die Job-Preflight-Meldung einbauen — der
  `HybridScheduler` trifft die Block/Allow-Entscheidung schon, mit
  `VramEstimate::describe()` als Grund; `verdict`s RAM-Offload-Logik ist rein
  advisory für die „soll ich das laden"-Frage vor dem Download (ADR-016-Zusatz).

---

## 6.4 — Ergebnis (abgeschlossen, a + b)

Der Download-Manager: von der „Files"-Liste in der Discovery mit einem Klick in
den Store — Range-Resume, Verify gegen die SHA-256 aus 6.1, dann `import_model`.
**ADR-023.**

**6.4a** (`f61af65`) — `core::download`:
- **Migration `0006`** — `downloads` (STRICT): `url`, `filename`, `dest_path`,
  `model_type`, `sha256`, `size_bytes`, `bytes_done`, `retries`, `state`
  (`queued|running|paused|verifying|done|failed`), `error_text`, `model_id`,
  `created_at`/`updated_at` + Index `(state, created_at)`.
- **`db::DownloadRepo`** — `create` (dest = `<staging>/<id>/<filename>`) / `get`
  / `list` (neueste zuerst) / `next_actionable` (ein `queued`/`running`,
  recovertes `running` zuerst) / `set_state` / `set_progress` (COALESCE Größe)
  / `bump_retries` (RETURNING) / `set_model_id` / `delete` / `recover_interrupted`
  (`running` + `verifying` → `queued`; `paused` bleibt).
- **`DownloadManager`** (`Arc`, `Notify`-Wakeup) — `enqueue` (offline-gated),
  `list` / `get`, `pause` (`running`/`queued` → `paused`), `resume`
  (`paused`/`failed` → `queued`, offline-gated, weckt den Worker), `cancel`
  (→ `failed` + Staging-Verzeichnis weg), `run()` = die Ein-Slot-Worker-Schleife.
- **`transfer()`** — On-Disk-Offset lesen, `Range: bytes=<off>-` bei > 0; `206`
  anhängen · `200` neu · `416` fertig. Sauberes Ende unter Soll-Größe **oder**
  Stream-Fehler → Transport-Fehler → Retry mit Resume bis `MAX_RETRIES = 5`. Zeile
  alle `TICK = 400 ms` neu gelesen → Pause / Cancel greift im Stream. **`verify()`**
  = voller Re-Hash (`spawn_blocking`) + Größen-Check; Mismatch → löschen + `failed`.
- **`App.downloads`** + `App::seed` ruft `recover_interrupted`; der Worker wird
  von `api::spawn` gestartet (`Services.download_worker`, `Drop` bricht ab).
  `AppPaths::downloads_dir()` = `<local_root>/.downloads`.
- **+4 DB-Unit + 5 Integration** (`core/tests/download.rs`, In-Process-Datei-
  Server): Verify-+-Import-Fluss, Resume aus vorgeseedeter Teil-Datei mit
  `Range`-Request, Pause hält die Teil-Datei, SHA-256-Mismatch → `failed` +
  Datei weg, `enqueue` im Offline-Modus abgelehnt.

**6.4b** (dieser Commit) — API / Tauri / UI:
- **DTOs** (`api::dto`): `EnqueueDownloadDto { url, filename, model_type?,
  sha256?, size_bytes? }`.
- **Handler** (`api::handlers`, transport-agnostisch): `list_downloads`,
  `enqueue_download` (behält nur den Basename des `filename`), `pause_download`
  / `resume_download` / `cancel_download`.
- **HTTP** (`api::http`): `GET`/`POST /downloads`, `POST /downloads/{id}/pause`
  · `/resume` · `/cancel` (`201` / `204`).
- **Tauri**: `list_downloads`, `enqueue_download`, `pause_download`,
  `resume_download`, `cancel_download` in `generate_handler!`.
- **UI** (`ui/src/features/models/`): `Downloads.tsx` — die Downloads-Liste
  (Fortschrittsbalken, Pause/Resume/Cancel, „Importiert"), oben im Models-Tab,
  versteckt wenn leer, `useDownloads` pollt `GET /downloads` (1,5 s).
  `Discover.tsx` — „Download & import" pro Datei-Zeile (deaktiviert für Split-
  GGUFs und Gated-Repos), ruft `enqueueDownload({ url, filename, model_type,
  sha256, size_bytes })`.
- **+1 Integration** (`download_endpoints_over_http`: echtes `App::load` +
  `ApiServer::bind`, `POST /downloads` mit `filename: "sub/dir/x.gguf"` →
  Basename bleibt, pollt `GET /downloads` bis `state == "done"` + `model_id`).
- **dev-mock-Fix**: `list_downloads` gab die `DOWNLOADS`-Array-**Referenz**
  zurück und mutierte sie in-place → `usePolled`s `setData(sameRef)` lief in
  Reacts `Object.is`-Bailout, das Panel re-renderte nie. Jetzt frische Kopie pro
  Poll (der echte Core serialisiert ohnehin einen neuen `Vec`).
- **Smoke** (dev-mock): Enqueue → Zeile erscheint, Balken animiert, Pause →
  Paused, Resume → Downloading, Cancel → weg, Fertigstellung → „Importiert".
  Konsole fehlerfrei.

**→ 333 Lib + 55 integ + 5 pytest.** `check.ps1` grün.

**Nicht in 6.4 (bewusst verschoben):**
- **Speicherplanung** — freier Platz auf dem Store-Volume vs. Download-Größe,
  Klartext-Warnung *vor* dem Enqueue. Gehört in die „Storage"-Ansicht (6.8);
  aktuell scheitert ein zu großer Download erst beim Schreiben.
- **Split-GGUF-Sets** als Ganzes laden + **Gated-Repos** mit `HF_TOKEN` — der
  Knopf ist für beide deaktiviert (6.9).
- **Streaming-Hash** während des Transfers — Verify bleibt ein separater
  Re-Hash, weil Resume den Zwischenstand nicht mitführt.

---

## 6.5 — Ergebnis (abgeschlossen, a + b)

`core::bench` — der „Test model"-Job. Misst **nur lokal** (ADR-024): tok/s
Prompt + Generation, Kalt-Ladezeit, VRAM-/RAM-Peak, Konsistenz über N Läufe.
Der „Overall Score" ist eine **offen deklarierte Heuristik**, keine
Qualitäts-Achse.

**6.5a** (`62217ff`) — `core::bench` + Persistenz + Engine:
- **Migration `0007`** — `benchmarks` (STRICT): `model_id` (FK, `ON DELETE
  CASCADE`), `job_id`, `kind` (`llm`), `runs`, `prompt_tps`, `gen_tps`,
  `load_ms`, `vram_peak_mb`, `ram_peak_mb`, `stability_score`, `overall_score`,
  `notes`, `created_at` + Index `(model_id, created_at)`.
- **`db::BenchRepo`** (`db.benchmarks()`) — `insert` / `get` / `list_for` /
  `latest_for` / `latest_all` (eine Zeile pro Modell — die Score-Spalte).
- **`core::bench`**: `BenchRequest::from_params` (Default-Prompt, 3 Läufe, 128
  Tokens; geklammert 1–10 / 16–512). **`stability_score(&[f64])`** = `1 −
  Variationskoeffizient`, geklammert; < 2 Samples → 1,0. **`overall_score(gen_tps,
  stability, &FitVerdict)`** = `(0,65·speed + 0,35·stability) · fit_faktor`,
  `speed = gen_tps / 80` geklammert; `fit_faktor` Green 1,0 · Yellow 0,85 ·
  Unknown 0,8 · **Red 0,35** (ein Modell, das nicht passt, wird hart gedeckelt,
  egal wie schnell). **`bench::run`**: N gestreamte Läufe, Telemetrie-Peak an
  den Lauf-Grenzen gesampelt (der Sampler tickt 1×/s → grober Peak), eine
  `benchmarks`-Zeile + Job-Events.
- **`GenerationEvent::Done`** hat jetzt `prompt_tokens_per_second` (aus
  `timings.prompt_per_second`); `chat.rs` ignoriert es, `aiwm-fake-llama` emit­tiert es.
- **`JobEngine`**: `job_type == "bench"`-Zweig → `bench::run`; die Plan-`match`
  misst jetzt die Modell-Ladezeit (`Instant` um `self.load`) und reicht sie als
  Kalt-Ladezeit weiter. **`JobEngine::with_telemetry(rx)`** — der Bench-Body
  sampelt daraus; `App` verdrahtet `telemetry.subscribe()`, jeder andere
  Aufrufer behält eine eingefrorene „kein GPU"-Reading. **`Scheduler`-Trait**
  hat jetzt `budget_mb()` (Default `0`; `HybridScheduler` gibt sein Budget) →
  der Engine kann Fit gegen ein `dyn Scheduler` beurteilen.
- **+4 DB-Unit + 7 Bench-Unit** (stability / score / from_params / run
  misst+speichert / run abgebrochen) **+1 Integration** (`core/tests/bench.rs`:
  echter `JobEngine` + `aiwm-fake-llama` → ein Bench-Job timed die Kalt-Ladung,
  schreibt eine Zeile, markiert das Modell genutzt).

**6.5b** (dieser Commit) — API / Tauri / UI:
- **Handler** (`api::handlers`): `latest_benchmarks` (`latest_all`),
  `model_benchmarks` (Verlauf), `benchmark_model` (nur `format == "gguf"` → sonst
  400; berechnet die VRAM-Schätzung, submitted `NewJob::new("bench").on("llamacpp",
  …)`).
- **HTTP**: `GET /benchmarks`, `GET /models/{id}/benchmarks`,
  `POST /models/{id}/benchmark` (201 + `Job`). **Tauri**: `list_benchmarks`,
  `model_benchmarks`, `benchmark_model`.
- **UI**: `ipc.ts` `Benchmark` + `listBenchmarks`/`modelBenchmarks`/`benchmarkModel`;
  `hooks.ts` `useBenchmarks` (3 s). `Models.tsx` → `ModelLibrary` extrahiert,
  neue **„Score"-Spalte**: `ScoreCell` zeigt das gefärbte Score-Pill (ok ≥ 70 ·
  warn ≥ 40 · crit) mit tok/s-Kürzel + Tooltip (gen/prompt tok/s, Ladezeit,
  VRAM-Peak, Stabilität), „Test" / „re-test"-Knopf (nur GGUF), „testing…" solange
  ein `bench`-Job für das Modell aktiv ist (aus `useJobs`).
- **dev-mock**: `list_benchmarks` / `model_benchmarks` / `benchmark_model`;
  `progressBenchJobs()` flippt einen laufenden `bench`-Job nach ~3,5 s auf
  `completed` + legt eine Bench-Zeile an. `list_jobs` gibt jetzt frische Kopien
  zurück (gleicher `Object.is`-Bailout wie bei 6.4s `list_downloads`).
- **Smoke** (dev-mock): „Test" → Zelle „testing…" → nach ~3,5 s grünes Pill
  „81 · 70 t/s" + „re-test"; Konsole fehlerfrei.

**→ 345 Lib + 56 integ + 5 pytest.** `check.ps1` grün.

**Nicht in 6.5 (bewusst verschoben):**
- **Bild-/Video-Benchmarks** (Generierungszeit + VRAM) — der Plan nennt sie
  „analog"; braucht einen ComfyUI-Bench-Pfad. `kind`-Spalte + `overall_score`
  sind schon dafür ausgelegt.
- **Externer Benchmark-Score-Fetch** — ADR-024: erst wenn eine tragbare Quelle
  auftaucht (Post-6.5).
- **Kalibrierung** von `SPEED_REF_TPS` (80) und den Gewichten — Faustwerte;
  echte Zahlen zusammen mit der Estimator-Kalibrierung bei 4.0 (`HARDWARE.md`).

---

## 6.6 — Ergebnis (abgeschlossen)

Benchmark-gestützte `Auto`-Auswahl — die 6.5-Messungen fließen in die
Modellwahl ein. Verlängert **ADR-015** (siehe „Zusatz" dort).

- **`core::select`** (neu) — `AutoPreference { Balanced (Default) | Fast |
  Quality }` (`#[serde(rename_all="snake_case")]`, `parse`/`as_str`).
  **`pick_for_role(db, role, vram_budget_mb, pref) -> Option<Model>`**: holt
  `for_role_with_benchmark`, ordnet best-first, nimmt den ersten. `rank()` ist
  pur + getestet: **Fit-Partition** (`vram_estimate_mb ≤ Budget` schlägt „passt
  nicht"; Budget `0` oder unbekannte Schätzung → nicht ausgeschlossen) → **Score**
  (`selection_score`: benchmarkt → `speed`/`stability` aus dem `benchmarks`-Eintrag
  + `heft` aus `param_count` gegen 14 B, per `pref` gewichtet [Fast 0,8·speed ·
  Quality 0,55·heft]; nicht benchmarkt → neutral 50, Quality nudged mit `heft`) →
  **Nutzung** (`last_used_at` desc, `use_count` desc, Name). Ohne jeden Benchmark
  ist es exakt die alte Regel.
- **`db::ModelRepo::for_role_with_benchmark(role) -> Vec<(Model, Option<Benchmark>)>`**
  — `for_role` + `benchmarks().latest_for` pro Modell.
- **`config`**: neue `[models]`-Tabelle, `ModelsConfig { auto_preference:
  AutoPreference }` (`#[serde(default, deny_unknown_fields)]`, Enum-Variante wird
  von toml validiert). Restart nötig.
- **Verdrahtung**: `JobEngine::with_auto_preference` + `AgentSessions::with_auto_preference`
  (Default `Balanced`), von `App::load` aus `config.models.auto_preference`
  gesetzt. `engine::resolve_target` (`chat`) + `resolve_comfyui_target`
  (`base_diffusion`/`base_video`) + `capability::agent::resolve_model` (`coding`)
  rufen jetzt `select::pick_for_role`. `Scheduler`-Trait + `CodingRuntime`-Trait
  bekamen `budget_mb()` (Default `0`; die echten Impls geben das Scheduler-Budget).
  `vae` / `text_encoder` bleiben auf `ModelRepo::pick_for_role`.
- **API/UI**: `ConfigUpdate.models` (`#[serde(default)]`), `save_config` schreibt
  es. `ipc.ts` `AutoPreference` + `ModelsConfig` in `AppConfig`/`ConfigUpdate`;
  `dev-mock` `CONFIG.models`. `Settings.tsx` neue Karte **„Model selection
  (Auto)"** — ein `<select>` balanced / fast / quality mit Erklärung.
- **+7 select-Unit** (fit schlägt Speed · Fast vs Quality · Usage-Regel ohne
  Bench · Budget 0 · DB-Join) **+1 config-Unit** (`[models]` liest die Preference,
  lehnt Junk ab) **+ api-Roundtrip erweitert** (`models.auto_preference` durch
  `PUT /config`). **→ 351 Lib + 56 integ.** `check.ps1` grün; Browser-Smoke:
  Settings → „Prefer quality" → Save → `get_config` zeigt `quality`.

**Nicht in 6.6:**
- **Live-Anwenden** der Preference — wie `[llama]` / `[comfyui]` erst nach
  Neustart (die drei Subsysteme halten den Wert bei Konstruktion).
- Eine sichtbare „warum dieses Modell?"-Begründung in der UI — das
  Auto-Selektions-Event schreibt schon „auto-selected …", mehr nicht.

---

## 6.7 — Ergebnis (abgeschlossen, a + b)

Der **Upgrade-Check** — „Gibt es was Besseres?" pro installiertem Modell. Das
lokale LLM ist nur ein Re-Ranker über echte HF-Treffer, die schon in den
VRAM-Etat passen. **ADR-025.** „Besser" = nur objektive, abrufbare Signale;
jede Ausgabe trägt „Qualität ist nicht lokal verifizierbar" (ADR-024).

**6.7a** (`056f965`) — `core::upgrade`:
- **`run(registry, reasoner, target, vram_budget_mb, free_ram_mb) -> UpgradeReport`**:
  zwei HF-Suchen auf die **Familie** des Ziels (`SearchSort::Trending` +
  `RecentlyUpdated`, `gguf_only` für LLM-Rollen), gemergt → **Spam-Guard**
  (`looks_like_spam`: unter 80 Downloads **und** 3 Likes raus; brandneues Repo
  mit „unmöglicher" Popularität raus — R9) → **Fit-Filter** (`compat::verdict`
  über eine grobe VRAM-Schätzung `param_count` × Bytes/Gewicht[Precision];
  **`Red` fliegt raus**) → **objektives Vor-Ranking** (`objective_score`:
  Recency + log-Popularität + Fit-Bonus; installierte Repos sinken) → Top 8.
- **Das LLM ist nur ein Re-Ranker, best-effort.** `build_prompt` listet die
  **echten** Kandidaten; `parse_ranking` nimmt **ausschließlich** ids, die exakt
  in der Kandidatenliste stehen (`BTreeSet`-Match) — ein erfundener Name wird
  verworfen. Leere / kaputte / fehlgeschlagene Antwort → objektive Reihenfolge
  bleibt, `note` sagt das.
- **`Reasoner`-Trait** (`async think(prompt, max_tokens) -> String`) —
  Prod-Impl auf `LlamaCppAdapter::complete`, Tests mit Canned-Antwort.
- `UpgradeTarget { label, family, params, is_llm, installed_ids }`,
  `UpgradeCandidate { id, why, downloads, likes, last_modified, param_count,
  format, gated, fit, installed, llm_ranked }`, `UpgradeReport { target, query,
  candidates, note, freshness }`.
- **+7 Unit** (`bytes_per_param`, Spam-Guard, `parse_ranking` [bekannte ids /
  Junk], `run` filtert+rankt+wendet die LLM-Ordnung an, `run` fällt auf die
  objektive Ordnung zurück, `run`s Hinweis wenn nichts passt).

**6.7b** (`<dieser Commit>`) — Job + API + UI:
- **`App.registry`** ist jetzt `Arc<Registry>` (der Job-Engine teilt ihn).
  **`JobEngine`**: `job_type == "upgrade_check"`-Zweig; `resolve_target` pic't
  das **Reasoning-Modell** (`chat`, Fallback `coding`, via 6.6-`select`), der
  Scheduler lädt es. `with_registry(Arc<Registry>)` + `set_registry` (letzterer
  für `App::with_registry` in Tests). Body: `upgrade_target(&job)` aus
  `params.target_model_id` (Familie aus `family`/`arch`/Namens-Tokens;
  `installed_ids` aus den HF-URLs der Download-History) → `upgrade::run` →
  `UpgradeReport`-JSON in `jobs.result`. Gemeinsame Helfer `pick_llm` /
  `llm_target` (chat + upgrade teilen sie).
- **Handler** `upgrade_check(app, model_id)` — `offline` → 400; Modell muss
  existieren → 400; submitted `NewJob::new("upgrade_check")` mit
  `params.target_model_id`. **HTTP** `POST /models/{id}/upgrade-check` (201 +
  `Job`). **Tauri** `upgrade_check(id)`.
- **UI**: `ipc.ts` `UpgradeCandidate` / `UpgradeReport` + `upgradeCheck(id)`.
  `Models.tsx` neue letzte Spalte **„Better?"** pro Zeile (`UpgradeCell` —
  `window.confirm` als per-Aktion-Consent, „checking…" solange ein Job läuft).
  `UpgradeChecks.tsx` (neu) — ein Panel über `<Discover>` das laufende +
  fertige Checks zeigt: Ziel, `note`, Kandidatenzeilen (Fit-Punkt, id-Link,
  Params/Downloads/Datum, `why`, „Download & import" → `registryModel` →
  bestes GGUF-File → `enqueueDownload`; „installed" / „gated"-Badges).
  `dev-mock`: `upgrade_check` + `progressUpgradeJobs()` (canned Report nach ~3 s).
- **+1 Integration** (`core/tests/upgrade.rs`: echter `JobEngine` +
  `aiwm-fake-llama` [Prosa → objektive Ordnung] + Stub-`ModelSource` → 70B auf
  Fit gefiltert, Spam gefiltert, Report in `jobs.result`) **+1 api-Test**
  (offline → 400, online → 201 mit `params.target_model_id`, Ghost → 400).
  **→ 359 Lib + 57 integ.** `check.ps1` grün; Browser-Smoke: „Better?" →
  „checking…" → Panel mit 2 Kandidaten + Note → „Download & import" reiht einen
  Download ein. Konsole fehlerfrei.

**Nicht in 6.7 (bewusst verschoben):**
- **Pro-Rolle-Check** — der Handler nimmt bewusst nur eine `model_id`; „bester
  chat-Modell allgemein" käme über dieselbe Maschinerie mit dem Rollen-Referenz-
  Modell, aber ohne UI-Einstieg. Später.
- **`base_model:`-Lineage-Suche** — die Familie kommt aus Freitext (`family`);
  der `filter=base_model:<id>`-Pfad braucht das Ziel *auf* HF gematcht, was
  unzuverlässig ist (R9). Freitext + Vor-Ranking + LLM reichen für den MVP.
- **Externer Score** (6.5-Fetch) im Ranking — ADR-024, kommt wenn eine Quelle
  auftaucht.
- Gated-Repos / Split-GGUFs one-click — der Knopf zeigt dann „retry" / den
  HF-Link (wie 6.4).
