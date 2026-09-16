# Trainings-Orchestrator (Teilsystem 2) — Design

Datum: 2026-09-16 · Status: vom User abgenommen (Abschnitte 1–5), noch nicht umgesetzt

## Ziel

AIWM soll eigene Daten jeder Art in ein lokales LoRA-Training verwandeln
können — angestoßen aus dem Manager, aber **gekapselt**: der Manager kann
geschlossen werden, das Training läuft weiter, beim nächsten Öffnen wird der
Stand wieder aufgenommen. Das Ergebnis landet automatisch in der
Modellbibliothek und ist sofort im LoRA-Stack-Picker der Bild-/Video-Tabs
wählbar.

Leitprinzip (User, 2026-09-15/16): AIWM bleibt ein lokales
All-in-one-Tool; Training auf eigenen Daten muss so leicht zugänglich sein
wie Bild-/Video-/Chat-Features — **egal welches Modell**, nicht ein
Sonderfall für eine Familie.

Konkrete Anwendungsfälle des Users:
- 20.000 Fotos → LoRA für Modell XY.
- Hunderte Stunden Videomaterial eines selbst animierten Anime (Rechte
  geklärt) → rekursiv einlesen → Standbilder alle X Sekunden → defekte
  automatisch und manuell aussortieren → Style-LoRA.
- Alternativ: ganze Videodateien direkt als Datensatz für ein Videomodell.

## Entscheidungen (mit User getroffen)

| Frage | Entscheidung |
|---|---|
| LoRA oder Full-Finetune? | **Nur LoRA.** Full-Finetunes von 4–9B-Modellen passen nicht auf 16 GB. |
| Zielmodell-Fokus | **Generisch** über eine Profil-Registry. Erster Profilsatz: FLUX.2 [klein] 4B **und** 9B, SDXL, Wan 2.2 5B. Der User nutzt klein 9B (unzensiert); SDXL/FLUX.1 sind für ihn zweitrangig, FLUX.1-dev lädt ohnehin nicht (RAM). |
| Trainer-Backend | **ostris/ai-toolkit** (MIT). Ein Backend für Bild *und* Video. |
| Prozessmodell | **Losgelöster Prozess** (Variante 2), nicht Job-Engine, nicht Sidecar. |
| Automatische Aussortierung | **Stufe C**: Unschärfe + Duplikate (bestehend) + tote Frames/Übergänge + Vielfalts-Kappe pro Clip. Keine Text-/Letterbox-Erkennung (Stufe D). |
| Captioning | **Optional**, standardmäßig aus, wenn kein Vision-Modell importiert ist. |

## Verifizierte Fakten (Quellen im Anhang, nicht aus dem Gedächtnis)

- **ai-toolkit** (MIT): headless `python run.py <config>.yaml`; registriert
  `flux2_klein_4b`, `flux2_klein_9b`, SDXL, FLUX.1, `wan22_5b` (u. a.);
  Checkpoints + Vorschaubilder in Intervallen; **Resume vom letzten
  Checkpoint** ist eingebaut; Low-VRAM-Knöpfe `quantize`/`qtype: qfloat8`,
  `low_vram`, `layer_offloading`; Python ≥ 3.10 (3.12 empfohlen), eigene
  Torch-Pinnung.
- **Datensätze:** Bilder ohne `.txt` bekommen `default_caption` des
  Datensatz-Ordners plus Trigger-Wort (Captioning ist für den Trainer
  optional). **Rohe Videodateien** (`.mp4 .avi .mov .webm .mkv .wmv .m4v
  .flv`) werden direkt geladen (OpenCV, PyAV-Fallback für AV1); Frames
  werden beim Training gesampelt (`num_frames`, `auto_frame_count`,
  `shrink_video_to_frames`).
- **FLUX.2 [klein] 4B Base:** Apache-2.0; offiziell LoRA-Training ab
  12 GB VRAM / 32 GB RAM; Inferenz ~13 GB. **9B Base:** FLUX
  Non-Commercial (ok nach ADR-011); offiziell 22 GB VRAM / 64 GB RAM;
  inoffiziell auf 16 GB mit fp8-Basis + Block-Swap (~14 GB), 32 GB RAM
  „empfohlen". Auf dem Zielrechner (RTX 4080 Super 16 GB, 32 GB RAM) ist
  4B „passt bequem", 9B „am Limit".
- **Basisgewichte fürs Training** sind die bf16-Originale + Qwen3-Text-
  Encoder (4B → Qwen3-4B, 9B → Qwen3-8B) — **nicht** die fp8-
  Inferenzdatei, die AIWM heute für 9B im Store hat.
- **AIWM heute:** `JobEngine::recover` (`core/src/orchestrator/engine.rs`)
  markiert beim Start jeden `running`-Job als fehlgeschlagen — kein Resume.
  `core::launcher::spawn::launch_detached` (`CREATE_NEW_CONSOLE`, kein
  Job-Object) ist der bestehende Präzedenzfall für Prozesse, die die App
  überleben. `capability::dataset` exportiert bereits `NNNN.png`+`NNNN.txt`;
  Captioning (Florence-2) ist dort heute Pflicht.
- **locally-uncensored „Character Studio"** (AGPL, nur Ideen): eigenes venv
  pro Trainer + Import-Probe + Selbst-Reparatur vor jedem Lauf;
  Log-Parsing muss an `\r` *und* `\n` splitten (In-place-Fortschrittsbalken);
  Abbruch = ganzen Prozessbaum töten, Blätter zuerst (`accelerate`-
  Enkelprozesse); vor dem Start Chat-Modell entladen + ComfyUI-Cache
  leeren; Festplatten-Check vor dem ersten Byte; OOM-Erkennung mit
  Klartext; „Umgebung kaputt"-Zustand. Ihr Training ist app-gebunden ohne
  Resume — unser Modell ist bewusst anders.

## Abschnitt 1 — Trainings-Profile

Eine Registry `TrainingProfile` in `core::training::profile`, ein Eintrag
pro trainierbarer Modellfamilie. Felder:

- `family` (matcht das `family`-Feld der Bibliothek) und `arch`
  (ai-toolkit-Architektur-ID, z. B. `flux2_klein_9b`, `wan22_5b`).
- `data_kind`: `Frames` (Bilder + optionale Captions) oder `Clips`
  (rohe Videodateien). Wan-Profil akzeptiert beides, Bild-Profile nur Frames.
- `vram_strategy`: feste ai-toolkit-Einstellungen (`quantize`, `qtype`,
  `low_vram`, `layer_offloading`) **und** `fit: Comfortable | AtTheEdge`
  (Klartext im UI, bevor gestartet wird). 4B = Comfortable, 9B = AtTheEdge,
  Wan 5B = AtTheEdge, SDXL = Comfortable.
- `base_weights`: Liste der Katalog-Einträge (`KNOWN_MODELS`, pinned
  SHA-256), die der Trainer braucht (bf16-Basis, Text-Encoder, ggf. VAE).
  Bereitstellung an ai-toolkit über einen **lokal gestellten HF-Cache**
  (derselbe Mechanismus wie `sidecar/dia.py::_stage_local_hub_cache`,
  `HF_HUB_OFFLINE=1`) — kein Netzzugriff zur Laufzeit.
- `defaults`: Rank, Lernrate, Auflösung, Schritt-Ziele der drei Presets
  (Schnell / Ausgewogen / Gründlich).

Ein Bibliotheksmodell ohne Profil ist im UI „noch nicht trainierbar
(Familie X)". Neue Familie = neuer Eintrag + Tests, kein neues Subsystem.

Nicht enthalten: frei editierbares YAML für den User, Full-Finetunes,
Wan-14B-Profile.

## Abschnitt 2 — Trainingslauf: Datenmodell und Prozess-Lebenszyklus

**Tabelle `training_runs`** (Migration `0015`, getrennt von `jobs`):
`id, name, profile_family, target_model_id, dataset_id, data_kind,
trigger_word, preset, hyperparams_json, sample_prompts_json, state,
step, total_steps, last_loss, last_checkpoint_at, pid, work_dir,
result_model_id, error_text, created_at, started_at, finished_at`.

Status-Automat: `preparing → running → paused | interrupted → resuming →
running → finishing → completed | failed | cancelled`. `env_broken` ist
kein Lauf-Status, sondern ein Zustand des Trainer-Runtimes.

**Trainer-Runtime `TrainingAdapter`** (`core::runtime::training`), nach dem
`RuntimeAdapter`-Muster: eigenes `uv`-venv unter `runtimes/ai-toolkit/`
(getrennt vom Sidecar-venv, eigene Torch-Pinnung), Installation über den
idempotenten Marker-Installer wie ComfyUI (pinned Commit + SHA-256 des
Archivs, real gehasht). Vor jedem Lauf: **Import-Probe** (Torch lädt, CUDA
sichtbar, VRAM-Zahl); schlägt sie fehl → `env_broken`, der Einrichten-Knopf
erscheint wieder, der Installer läuft als Selbst-Reparatur.

**Start eines Laufs:** AIWM erzeugt aus Profil + Preset + Feineinstellungen
+ Datensatz eine ai-toolkit-YAML im Arbeitsordner
`data/training/<run_id>/`, gibt die GPU frei (Chat-Modell entladen,
ComfyUI-Cache leeren), prüft Festplatte, meldet die VRAM-Reservierung des
Profils beim Scheduler an, und startet `run.py` **losgelöst** über den
bestehenden `launch_detached`-Mechanismus (eigene Prozessgruppe, kein
Job-Object). Stdout/Stderr gehen in `<work_dir>/train.log`, nicht in eine
Pipe.

**Fortschritt** kommt dateibasiert (überlebt Neustarts): (1) Checkpoint-
und Vorschau-Dateien in `<work_dir>` (Schritt aus dem Dateinamen,
Vorschaubilder pro Testprompt), (2) `train.log` — Parser splittet an `\r`
und `\n`, liest Schritt/Gesamt, Loss, ETA. Ein Poller (alle 3 s) prüft
zusätzlich die PID: **Prozess weg ohne Abschluss → `interrupted`**, nie
`failed`.

**Beim App-Start:** alle `running`/`resuming`-Zeilen gegen die PID prüfen;
lebt der Prozess, wird die Reservierung wiederhergestellt und der Poller
angehängt; sonst `interrupted`.

**Pausieren:** Prozessbaum beenden (Blätter zuerst), Checkpoint bleibt,
Status `paused`. **Fortsetzen:** YAML um ai-toolkits Resume-Angabe
ergänzen, neu starten. **Abbrechen:** beenden + `cancelled`; Arbeitsordner
bleibt bis zum manuellen Löschen. Ein Abbruch-Flag wird vor jedem
Kindprozess-Start geprüft.

**Abschluss:** letzte LoRA-Datei → Import in die Bibliothek als
`ModelKind::Lora`, Familie = Zielmodell, Name = Lauf-Name, `source =
"training:<run_id>"`; `result_model_id` gesetzt; Reservierung freigegeben.

**Scheduler:** die Reservierung ist ein normales „geladenes Modell" mit
synthetischer ID (`TRAINING_MODEL_ID`, wie beim Upscale-/Dataset-Job);
Bild-/Video-Jobs werden mit „Training belegt die GPU" blockiert und warten.

**Grenzen:** ein Lauf gleichzeitig; mindestens ein Testprompt Pflicht.

## Abschnitt 3 — Dataset-Erweiterungen

**Datensatz als eigenes Objekt:** Tabelle `datasets` (`id, name, mode:
Frames|Clips, source_root, export_dir, item_count, created_at`);
`dataset_frames` bekommt `dataset_id` und `rejection_reason`
(`null | blur | duplicate | black | transition | cap`). Ein kuratierter
Datensatz ist für beliebig viele Läufe wiederverwendbar.

**Captioning optional:** Extraktion + Filter + Sichten laufen ohne
Vision-Modell. Schalter „Auto-Beschriften", ausgegraut mit Hinweis, wenn
Florence-2/Qwen nicht importiert sind. Frames ohne Caption erhalten beim
Training nur das Trigger-Wort (`default_caption`). Captions bleiben im Grid
editierbar.

**Filter-Stufe C** (pro Clip, in dieser Reihenfolge):
1. *Tote Frames*: mittlere Helligkeit nahe 0 oder 255 bei niedriger Varianz.
2. *Übergänge*: Frame ist unscharf **und** unterscheidet sich stark von
   Vorgänger *und* Nachfolger (Perceptual-Hash-Distanz beidseitig hoch).
3. *Unschärfe + Beinahe-Duplikate*: wie heute.
4. *Vielfalts-Kappe*: höchstens N Frames pro Clip (Default 40,
   einstellbar), gewählt als untereinander unähnlichste über den
   vorhandenen Hash (Greedy: nächster Frame = größte minimale Distanz zu den
   bereits gewählten) — nicht die ersten N.

Jeder Verwerfungsgrund ist im Grid ein Filter-Chip; Einzelne können
zurückgeholt werden.

**Modus Clips:** derselbe rekursive Einlese-Schritt; Videodateien werden
selbst zum Datensatz. Sichtung: Dauer, Auflösung, Vorschau-Standbild,
Ausschließen per Klick; automatisch raus: nicht dekodierbar oder unter
Mindestlänge (Default 2 s). Optional Start-/Ende-Zeitmarke pro Clip. Export
= Clips in einen Ordner, den das Wan-Profil direkt liest.

Nicht enthalten: Stufe D (Text/Letterbox), Szenenerkennung mit eigenem
Modell, Ausschnitt-Editor.

## Abschnitt 4 — Bedienung

**Einstieg:** Knopf **„LoRA trainieren"** im Dataset-Tab neben
„Exportieren". Formular:
1. **Zielmodell** — Bibliothek, gefiltert auf Familien mit Profil; daneben
   die Passt-Einstufung im Klartext; profil-lose Modelle ausgegraut.
2. **Name + Trigger-Wort** (Trigger: Whitespace entfernt, ≤ 30 Zeichen).
3. **Preset** Schnell / Ausgewogen / Gründlich; eingeklappte
   „Fein-Einstellungen": Rank, Lernrate, Auflösung, Schritte.
4. **Testprompts** (1–3, Pflicht, vorausgefüllt mit dem Trigger-Wort).

**Vor dem Start, blockierend mit Klartext:** Trainer eingerichtet (sonst
„Einrichten" mit Download-Größe) · Basisgewichte in der Bibliothek (sonst
Ein-Klick-Download aus dem Katalog) · Festplatte · GPU-Freigabe.

**Neuer Tab „Training":** Liste aller Läufe; pro Lauf Fortschrittsbalken
(Schritt/Gesamt), Loss-Kurve, ETA, jüngstes Vorschaubild pro Testprompt,
letzte Logzeilen aufklappbar; Knöpfe Pausieren / Fortsetzen / Abbrechen.
`interrupted` wird als „unterbrochen — fortsetzen?" gezeigt. Diagnostics
zeigt ein laufendes Training als GPU-Belegung.

**Ergebnis:** Eintrag verlinkt auf das importierte LoRA; **„Jetzt testen"**
öffnet den Bild-/Video-Tab mit diesem LoRA vorgewählt und dem Trigger-Wort
im Prompt.

Nicht enthalten: parallele Läufe, Lauf-Vergleich, Live-Parameteränderung,
Export in fremde Ordner.

## Abschnitt 5 — Fehlerfälle und Tests

**Abgefangene Fehlerfälle:** OOM (Klartext mit Karte + Ursache, Angebot
kleineres Preset/Profil, Checkpoint bleibt) · App/PC-Neustart
(`interrupted`, Fortsetzen) · `env_broken` (Einrichten + Selbst-Reparatur)
· Festplatte (vor dem ersten Byte) · unbrauchbarer Datensatz (im Formular)
· Bild-/Video-Job während des Trainings (Scheduler blockiert sauber) ·
Abbruch zwischen Kindprozessen (Flag vor jedem Start, Baum Blätter-zuerst).

**Tests:**
- *Unit (Rust):* Profil-Registry; YAML-Erzeugung (exakter Inhalt);
  Log-Parser gegen **echte aufgezeichnete** ai-toolkit-Logzeilen;
  Status-Automat inkl. `interrupted`; neue Filter gegen kleine echte
  Bilddateien.
- *Integration (Rust):* Fixture **`aiwm-fake-trainer`** (liest YAML,
  schreibt Fortschritt/Checkpoints im echten Format, reagiert auf Signale)
  → voller Lebenszyklus ohne GPU: start → Fortschritt → „App zu" → Prozess
  lebt → wieder anhängen → Kill → `interrupted` → fortsetzen → Import.
  Plus: Reservierung blockiert einen Bild-Job.
- *Echter Lauf, einmal:* FLUX.2 klein **4B**, kleiner Frame-Datensatz aus
  echtem Material, Preset Schnell, auf der echten Karte; Vorschaubilder
  geprüft, LoRA im Bild-Tab benutzt; echte Zahlen (VRAM, s/Schritt) in
  `docs/TODO.md`. Danach 9B ausprobieren; Ergebnis ehrlich in den
  Profil-Text.
- *UI:* live gegen `dev-mock` (Formular, Verlauf mit simuliertem
  Fortschritt, `interrupted` → Fortsetzen), Screenshots.

Nicht getestet: Trainingsqualität — das ist der Blick des Users auf die
Vorschaubilder.

## Umsetzungsreihenfolge (Vorschlag für den Plan)

1. Dataset-Erweiterungen (Captioning optional, Stufe C, `datasets`-Tabelle,
   Clip-Modus) — sofort nützlich, auch ohne Trainer.
2. `TrainingAdapter` (Installer, Import-Probe, `env_broken`).
3. Profil-Registry + YAML-Erzeugung + Katalog-Einträge für Basisgewichte.
4. `training_runs` + losgelöster Lauf + Poller + `interrupted`/Resume +
   `aiwm-fake-trainer`.
5. Training-Tab + Formular + „Jetzt testen".
6. Echter Lauf 4B, dann 9B-Versuch; Kalibrierung in die Profile.

## Offene Punkte (bewusst nicht im Spec entschieden)

- Wan-2.2-5B-Profil: ob die 16-GB-Einstellung real läuft, entscheidet erst
  ein echter Versuch (nach Schritt 6).
- Ob der Dataset-Tab in „Datensätze" umbenannt wird, sobald Datensätze
  eigene Objekte sind — UI-Detail für den Plan.

## Anhang — Quellen

- BFL klein-Training: https://docs.bfl.ml/flux_2/flux2_klein_training
- ai-toolkit: https://github.com/ostris/ai-toolkit (README; `extensions_built_in/diffusion_models/flux2/flux2_klein_model.py`; `ui/src/app/jobs/new/options.tsx`; `toolkit/dataloader_mixins.py`)
- FLUX.2-klein-base-4B: https://huggingface.co/black-forest-labs/FLUX.2-klein-base-4B
- Fizgig (16-GB-9B-Erfahrungswerte): https://github.com/shootthesound/Fizgig
- locally-uncensored Character Studio: lokaler Checkout `E:\locally-uncensored` (Muster, kein Code)
