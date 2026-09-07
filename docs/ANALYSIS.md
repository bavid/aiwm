# Analyse des Projekt-Briefs

Kritische Durchsicht der Anforderungen vor der Architekturfestlegung. Ziel: Lücken,
Widersprüche und unrealistische Erwartungen früh sichtbar machen, damit wir sie
bewusst entscheiden statt später dagegen zu laufen.

---

## 1. Zentrale Spannungsfelder

### 1.1 „Maximale Leistung bei minimaler UI-Komplexität" + „Manage-first"

Die Frustration mit Stability Matrix (zu viele Parameter, zu viel manuelles
Model-Management) und die Entscheidung für **Manage-first** (das Tool installiert
und besitzt alle Runtimes) zeigen in **entgegengesetzte Richtungen**.

Stability Matrix *ist* im Kern ein Manage-first-Tool. Seine Komplexität entsteht
nicht primär in der UI, sondern in der Logik dahinter: unterschiedliche
Ordnerstrukturen, Formatkonflikte, Versionsdrift zwischen Runtime und Modell,
kaputte Python-Umgebungen, CUDA-/Torch-Mismatch. Die Komplexität verschwindet
nicht — sie wandert in unseren Code und in unsere Fehlermeldungen.

**Konsequenz für die Architektur:** Manage-first ist tragbar, aber nur mit
*radikal reduzierten Wahlmöglichkeiten*. Eine Runtime = genau eine unterstützte
Version = eine getestete Installationsart. Kein „wähle deine Torch-Version". Wenn
etwas nicht in dieses schmale, getestete Profil passt, wird es nicht automatisch
gemacht, sondern klar als „manueller Modus" markiert.

> ✅ **Bestätigt (B):** Manage-first bedeutet *wenige, fest kuratierte
> Runtime-Versionen*, kein frei konfigurierbares Environment (ADR-002).

### 1.2 „Coding-Agents lange autonom" + 16 GB VRAM

Der Brief fordert (Abschnitt 4), dass Agents lange laufen ohne an Context-Limits
zu scheitern, und bittet ausdrücklich um die *echten* Trade-offs statt der
Behauptung „großes Context-Window löst das".

Die ehrliche Antwort: **Lokal + langes Context + schnell + hohe Qualität sind auf
16 GB nicht gleichzeitig erreichbar.** Man wählt drei von vier.

- Ein großes Context-Window kostet VRAM über den **KV-Cache**. Bei 16 GB
  konkurriert der KV-Cache direkt mit den Modellgewichten. 128k+ Kontext bei
  einem 14–24B-Modell ist nur mit KV-Quantisierung (Q8/Q4) und Flash-Attention
  realistisch, und selbst dann knapp.
- Ein größeres Context-Window macht das Problem oft *schlimmer*: mehr Tokens =
  langsamere Verarbeitung pro Schritt, „lost in the middle"-Effekte, das Modell
  verliert bei sehr langen Kontexten an Präzision.
- Was tatsächlich hilft, ist **Context-Engineering**, nicht Context-Größe:
  Zusammenfassung/Kompaktierung älterer Turns, Retrieval statt „alles reinladen",
  Repository-Index statt vollständigem Datei-Dump, Sub-Agents mit eigenem
  frischem Kontext für Teilaufgaben, Checkpoints zum Wiederaufsetzen, Trennung
  von „Tool-Historie" und „Konversation".

**Konsequenz:** Das Tool sollte Agent-Autonomie über ein *Session-/Memory-/
Kompaktierungssystem* adressieren, nicht über die Jagd nach dem größten
Context-Window. Details in [ARCHITECTURE.md](ARCHITECTURE.md) und
[HARDWARE.md](HARDWARE.md).

### 1.3 „Ein Modell, eine Datei auf der SSD" (Abschnitt 10.8 / 10.17)

Nur teilweise erreichbar.

| Runtime | Modell-Speicherung | Teilen möglich? |
|---|---|---|
| llama.cpp / LM Studio | GGUF-Datei im HF-Cache-Layout bzw. freiem Pfad | Ja, gemeinsame Datei via Pfad/Junction |
| ComfyUI | Rohdatei (`.safetensors`, `.gguf`) in festen Ordnern | Ja, via Junction/Symlink auf kanonischen Store |
| Ollama | Content-addressed Blob-Store mit eigenem Manifest | **Nein** — Ollama importiert/kopiert; keine Fremd-Dateien im Store |

Das heißt: Ein GGUF-Coding-Modell, das sowohl in Ollama als auch in einem Agent
über llama.cpp genutzt wird, liegt praktisch **doppelt** auf der Platte, solange
Ollama im Spiel ist. Optionen:

1. Für LLMs primär auf **llama.cpp/llama-server** setzen (ein Store, alle
   Konsumenten via Junction), Ollama nur als optionalen Zusatz.
2. Ollama als LLM-Runtime akzeptieren und die Duplikate transparent anzeigen
   (Dedup-Report zeigt „21 GB durch Ollama-Import gebunden").

> ✅ **Entschieden (A):** llama.cpp/llama-server ist primäre LLM-Runtime
> (ADR-006). Ollama optionaler Zweitadapter, Duplikate transparent im
> Dedup-Report.

### 1.4 Auto-Unload/Reload während laufender Agent-Session

Der Brief-Beispiel (Abschnitt 12) beschreibt: Coding-Modell entladen → Bild-Modell
laden → Job → Coding-Modell zurückladen. Für einen **aktiven Agent** bricht das
die Session (State im Modell-Server, laufender Tool-Call, KV-Cache weg).

**Konsequenz:** Der Hybrid-Scheduler braucht *Session-Awareness*. Ein Modell mit
aktiver Agent-Session ist „pinned" und wird nicht evakuiert. Konkurrierende Jobs
werden gequeued oder der Nutzer wird explizit gefragt („Coding-Agent pausieren,
um Bild zu generieren?"). Reines VRAM-Accounting reicht nicht.

### 1.5 Offline-first + zentrale Online-Model-Discovery

Kein echter Widerspruch (der Brief erkennt das in 10.19 an), aber eine harte
Design-Anforderung: **jede** Funktion, die Online-Metadaten nutzt
(VRAM-Schätzung, Kompatibilität, Benchmark-Score, Update-Check), braucht einen
lokal gecachten Fallback und muss ohne Netz sinnvoll degradieren — nicht mit
Fehler abbrechen. Das ist Aufwand, der in jeder betroffenen Komponente anfällt.

---

## 2. Unrealistische bzw. neu zu rahmende Erwartungen

| Erwartung im Brief | Realität | Empfohlene Rahmung |
|---|---|---|
| Video-Generierung lokal (Abschnitt 6) | 16 GB + 32 GB RAM → 2–5 Sek. Clips, 480–720p, 2–10 Min. pro Clip, viel Offloading | Als „Kurzclip-/Experiment-Feature" positionieren, nicht als Video-Studio. RAM-Upgrade auf 64 GB deutlich empfohlen. |
| Objektive Model-Quality-Scores (Abschnitt 33) | Es gibt keinen billigen, lokalen, objektiven Qualitäts-Benchmark. Lokal messbar sind nur Speed/VRAM/Load-Time/Stabilität | Qualitäts-Score = extern gepflegte Benchmarks (SWE-bench etc.) online ziehen + lokal gemessene Performance. „Overall Score" ist eine gewichtete Heuristik, klar als solche gekennzeichnet. |
| Benchmark-gesteuerte Auto-Model-Auswahl (Abschnitt 32/15) | Braucht erst eine Datenbasis; im MVP nicht vorhanden | Auto-Auswahl startet regelbasiert (Aufgabe + VRAM-Budget + installierte Modelle). Benchmark-Feedback fließt erst ab Phase 6 ein. |
| „One-Click" für beliebige Modelle jeder Quelle (Abschnitt 10.6) | Nur für Modelle in bekannten, getesteten Formaten/Quellen zuverlässig automatisierbar | One-Click für kuratierte Quellen (HF GGUF, Ollama-Library, bekannte ComfyUI-Modelle). Alles andere: „Assistierter Import" mit Nutzer-Bestätigung. |
| Vollautomatische „passende Pipeline + Modell" für Foto-Enhancement (Abschnitt 7) | Für einige Fälle gut lösbar (Upscale-Faktor → Modell, Face-Restore), für „Muttermal entfernen / Person entfernen" braucht es Segmentierung + Inpainting mit Kuratierung | MVP: feste, getestete Pipelines pro Checkbox. „Auto" wählt nur zwischen diesen. Keine generische Pipeline-Synthese. |
| „Modell nie doppelt auf der SSD" | Siehe 1.3 — mit Ollama nicht vollständig möglich | Duplikate transparent machen + Dedup-Vorschläge; Zero-Copy wo technisch machbar. |
| Shared Model Cache über alle Runtimes (10.17) | Junctions funktionieren für GGUF (llama.cpp/LM Studio/ComfyUI), nicht für Ollama, nicht immer für Diffusion-Formate mit runtimespezifischen Konventionen | Kanonischer Store + Adapter entscheiden pro Runtime: Junction, Kopie oder Import. |

---

## 3. Fehlende / unspezifizierte Anforderungen

Diese Punkte kommen im Brief nicht vor und sollten entschieden werden (Vorschläge
als Default in Klammern):

1. **App-Selbst-Update** offline-fähig? (Default: manueller Download + Signatur-Check, kein Auto-Update-Zwang)
2. **Backup/Restore** von Config + DB + Agent-Memory + Prompt-Historie (Default: „Export/Import"-Funktion ab Phase 5, DB liegt an einem bekannten Pfad)
3. **Parallele Jobs**: Darf je ein kleines LLM + ein Upscale gleichzeitig laufen? (Default: ja, wenn VRAM-Budget passt; Scheduler entscheidet)
4. **Multi-User / Remote-Zugriff** (Brief Abschnitt 34 fragt, beantwortet nie): (Default: **Single-User, nur `127.0.0.1`**, kein Netz-Listener. LAN-/Remote-Zugriff frühestens Phase 6, opt-in.)
5. **Zukünftige 2. GPU** (Brief 34): (Default: Architektur sieht `GpuId` im Datenmodell vor, Scheduler ist Single-GPU im MVP, Multi-GPU nicht blockiert)
6. **Lizenz des Tools selbst** (Open Source? nur privat?) — offen
7. **UI-Sprache**: Deutsch, Englisch, beides? (Default: Englisch als Basissprache, i18n-fähig aufgebaut) — **zu bestätigen**
8. **Ausgabe-Verwaltung** für Bilder/Videos: Galerie, Metadaten (Prompt/Seed/Modell im Bild), Speicherort, Retention (Default: fester Output-Ordner + SQLite-Index + PNG-Metadaten, Galerie ab Phase 3)
9. **Prompt-Bibliothek / History** — im Brief erwähnt, nie spezifiziert (Default: pro Capability gespeicherte History in SQLite ab Phase 3)
10. **GPU-Treiber-/CUDA-Annahmen**: Welche Treiber-Mindestversion? Wer installiert CUDA-Runtime? (Default: Tool prüft Treiber-Version beim Start, CUDA-Runtime kommt gebündelt mit den Runtimes, kein System-CUDA nötig)
11. **Windows-Firewall**: gebündelte Runtimes lösen beim ersten Start Firewall-Dialoge aus (Default: alle Runtimes binden `127.0.0.1`, Doku erklärt den Dialog)
12. **„Hermes" als Agent** (Abschnitt 4): ✅ geklärt — gemeint ist **Hermes Agent von Nous Research** (MIT, Python 3.11, kein Docker, lokaler Endpoint, persistentes Memory + Sub-Agents). Zusammen mit OpenCode als Agent-Runtimes gesetzt (ADR-010). Windows-Vorbehalt: `bash -l`-Kontext vor Phase 5 prüfen.
13. **Modell-Integrität**: Echte Signaturprüfung gibt es kaum. Verfügbar: SHA256 von HF. Das reale Risiko sind nicht `.gguf`/`.safetensors` (reine Daten), sondern Pickle-`.bin` und **Custom ComfyUI-Nodes** (beliebiger Python-Code). (Default: `.safetensors`/`.gguf` bevorzugen, Pickle-Formate warnen, Custom-Nodes nie automatisch installieren)

---

## 4. Zu treffende Architekturentscheidungen

Bereits entschieden (siehe [DECISIONS.md](DECISIONS.md)):

| ADR | Thema | Entscheidung |
|---|---|---|
| 001 | Tech-Stack | Tauri + Rust-Core + Python-Sidecar |
| 002 | Runtime-Verhältnis | Manage-first (mit Attach als Fallback) |
| 003 | VRAM-Strategie | Hybrid-Scheduler (session-aware) |
| 004 | MVP-Scope | Plattform-Kern zuerst |

Im Review geklärt (Details in [DECISIONS.md](DECISIONS.md)):

| # | Frage | Entscheidung |
|---|---|---|
| A | Primäre LLM-Runtime | ✅ llama.cpp/llama-server (ADR-006), Ollama optional |
| B | Manage-first = fest kuratierte Versionen, keine freie Env-Konfiguration | ✅ Ja (ADR-002) |
| C | UI-Sprache | Default Englisch, i18n-fähig |
| D | Netzwerk-Exposure | ✅ nur `127.0.0.1` (ADR-008) |
| E | „Hermes" — welcher Agent | ✅ Hermes Agent (Nous Research) + OpenCode (ADR-010) |
| F | Tool-Lizenz | ✅ privat, non-commercial (ADR-011) |
| G | Store-Pfad / Kapazität | ✅ `E:\AI\models`, ~1,5 TB frei (ADR-012) |

---

## 5. Scope-Warnung

Der Brief beschreibt in Summe ein System, das für ein kleines Team **mehrere
Jahre** Arbeit ist: Model-Lifecycle inkl. Dedup/Benchmark/Quality-Scoring,
Pipeline-Abstraktion, Plugin-Plattform, Multi-Provider, Agent-Sandbox,
Observability, drei komplette Capability-Domänen.

Das ist als *Vision* gut. Als *Bauplan* muss es hart sequenziert werden. Die
gewählte Antwort „Plattform-Kern zuerst" ist richtig, bedeutet aber: auch der
Kern muss abgespeckt werden (siehe [ROADMAP.md](ROADMAP.md) — der MVP enthält
**keine** Discovery-Suche, **keinen** Benchmark, **kein** Quality-Scoring, **kein**
Plugin-System).
