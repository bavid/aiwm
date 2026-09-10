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
| **6.1** | **`core::registry` — HF-Hub-Quellen-Adapter** (read-only, gegen ein Fixture): `ModelSource`-Trait (`search(query, filters) -> Vec<RemoteModel>`, `details(id) -> RemoteModelDetails`) + `HuggingFaceSource`. `RemoteModelDetails` = Params, `ctx_max`, Architektur, Lizenz, Gated-Flag, Downloads/Likes/`lastModified`/`createdAt` + die Quant-Dateien mit Größe **und SHA-256 aus `lfs.oid`** (kein Download nötig). TTL-Cache als JSON unter `<data>/cache/registry/`, damit `search`/`details` offline den letzten Stand + „stale"-Marker liefern. `offline_mode` → harte Ablehnung. `aiwm-fake-hfhub`-Fixture + Integrationstest. | offen |
| **6.2** | **Discovery-UI** (`ui/src/features/models/` erweitert): „Discover"-Panel im Models-Tab — Suchfeld, Filter nach Rolle/Capability + „passt in mein VRAM-Budget", Ergebnis-Karten (Name, Params, Downloads, Lizenz, `🟢/🟡/🔴`-Fit via `core::compat`, Quant-Dropdown mit Größen). Aktion vorerst nur „Copy link" + „Set import type" (Auto-Download = 6.4). `useRegistrySearch` (debounced), dev-mock. | offen |
| **6.3** | **Kompatibilitäts-Engine v2** (`core::compat` erweitert — verlängert ADR-016): `.safetensors`-Header-Inspektion (Arch/Precision — der seit 3.3 vertagte TODO), Diffusions-/Video-Modell-VRAM-Heuristik statt der Datei-Namens-`+2,5 GB`-Faustregel, `FitVerdict { Green, Yellow(grund), Red(grund) }` das Gewichte + KV/Aktivierungen + Overhead gegen `vram_budget_mb` **und** freien System-RAM prüft. Flat-Overhead gegen echte `HARDWARE.md`-Messungen kalibrieren (R3). Genutzt von Discovery + Upgrade-Check + dem bestehenden Job-Preflight. | offen |
| **6.4** | **Download-Manager** (`core::download` erweitert — ADR-023): `downloads`-Tabelle (Migration `0006`: id, url, dest, sha256, size, bytes_done, state {queued\|running\|paused\|verifying\|done\|failed}), Queue mit einem aktiven Slot, **Resume über HTTP-Range**, Verify gegen die erwartete SHA-256 (aus 6.1), dann Übergabe an `import_model`. `GET/POST /downloads`, `POST /downloads/{id}/{pause,resume,cancel}`, Fortschritts-Events (`job_events`-Muster). `offline_mode` → Ablehnung. UI: „Download & import"-Knopf auf den Discovery-Karten + eine Downloads-Liste. Speicherplanung: freier Platz auf dem Store-Volume vs. Download-Größe, Klartext-Warnung. | offen |
| **6.5** | **`core::bench` — lokale Mikro-Benchmarks**: `benchmarks`-Tabelle (Migration `0006`: model_id, ts, tokens_per_sec, load_ms, vram_peak_mb, ram_peak_mb, stability_score, notes), ein „Test model"-Job pro Modell (kurzer Prompt → tok/s Prompt+Gen, Ladezeit vom Adapter, VRAM-Peak via NVML, RAM-Peak via sysinfo). Für Bild/Video analog: Generierungszeit + VRAM. Optionaler Fetch der externen Benchmark-Scores (Quelle aus 6.0), lokal gecacht. `Overall Score` = klar gekennzeichnete gewichtete Heuristik (lokale Perf + externer Score + Stabilität). UI: „Test"-Knopf + eine Score-Spalte in der Model Library. | offen |
| **6.6** | **Benchmark-gestützte `Auto`-Auswahl**: `ModelRepo::pick_for_role` bezieht Benchmark-Daten ein (statt nur `last_used_at` / `use_count`) — Fit zuerst, dann Score, dann Nutzung. Gewichtung + „bevorzuge schnell / bevorzuge Qualität" in `[models]`-Config. Betrifft Chat, Coding, `base_diffusion`, `base_video`. Regelbasierter Fallback bleibt, wenn keine Benchmark-Daten da sind. | offen |
| **6.7** | **Upgrade-Check** (die Backlog-Idee — `core::registry` + `core::compat` + lokales LLM): Knopf „Gibt es was Besseres?" pro installiertem Modell **und** pro Rolle. Ablauf: HF-Hub nach neueren/populäreren Modellen derselben Rolle+Familie fragen → `core::compat`-Fit-Filter auf „läuft in `vram_budget_mb`" **vor** der LLM-Bewertung (spart Tokens, hält die Liste ehrlich) → das lokale LLM (Rolle `chat`/`coding`) rankt die **echten API-Treffer** als JSON, schreibt eine Ein-Satz-Begründung, **darf keinen Modellnamen erfinden** (Antwort gegen die Kandidaten-IDs validieren) → Ausgabe: kurze Liste + „Download & import" (→ 6.4). „Besser" misst sich in der MVP-Fassung an **objektiven, abrufbaren** Signalen (Release-Datum, Downloads/Likes, größere/neuere Basis in der Familie, Fit) + ggf. externem Score (6.5); das LLM markiert „Qualität nicht lokal verifizierbar". HF-Query = externer Call → **per-Aktion-Consent**, im `offline_mode` gesperrt. ADR-025. | offen |
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
