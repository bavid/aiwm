# Personas — Design

Status: written 2026-09-18 under the user's standing "work through the backlog" mandate. Backlog
entry (`docs/TODO.md`, "Ideen aus `locally-uncensored`"): "Personas — einfache Presets
(Name/Icon/System-Prompt), global aktiv + Override pro Chat." Idea only; no code is taken from the
AGPL project.

## Ziel

Ein Persona ist ein benanntes Preset aus Name, Icon (ein Emoji) und System-Prompt. Eine Persona
kann global aktiv sein; eine Chat-Session kann sie überschreiben (andere Persona oder ausdrücklich
„keine"). Der Chat-Job stellt den System-Prompt der aufgelösten Persona der Anfrage voran.

## Was heute gilt

Ein Chat-Job schickt genau eine User-Nachricht (`ChatRequest { prompt, max_tokens }`,
`core/src/capability/chat.rs`), optional mit RAG-Kontext aus angehängten Dokumenten; es gibt
keinen System-Prompt. `chat_body` (`core/src/runtime/llamacpp/client.rs`) baut den Request, sein
Default-Body ist per Test byte-genau festgenagelt. Sessions: `sessions`-Tabelle
(`core/src/db/sessions.rs`), Einstellungen: `core/src/db/settings.rs`.

## Entscheidungen

| Frage | Entscheidung |
|---|---|
| Datenmodell | Neue Tabelle `personas (id, name, icon, system_prompt, created_at, updated_at)`; `sessions.persona_mode TEXT NOT NULL DEFAULT 'inherit'` (`inherit` \| `none` \| `persona`) + `sessions.persona_id TEXT NULL`. Global aktive Persona als Settings-Schlüssel `chat.active_persona_id`. Migration 0018, rein additiv. |
| Auflösung | Serverseitig beim Job-Start, nicht im Client: Session `persona` → diese; Session `none` → keine; sonst global aktive; sonst keine. Eine gelöschte Persona fällt still auf „keine" zurück (Session wird auf `inherit` gesetzt, globaler Schlüssel geleert) — ein Chat darf nie an einer verwaisten Id scheitern. |
| Nachvollziehbarkeit | Der Job schreibt `persona: { id, name, icon }` in seine aufgelösten Params zurück (wie `ImageRequest::apply_to`), damit der Verlauf zeigt, welche Persona geantwortet hat — auch nachdem sie umbenannt oder gelöscht wurde. Der Prompt-Text selbst wird nicht in den Job kopiert. |
| Request | `chat_body` bekommt eine optionale System-Nachricht vor der User-Nachricht. Ohne Persona bleibt der Body **byte-identisch** (bestehender Test bleibt grün). RAG-Kontext bleibt in der User-Nachricht wie heute. |
| Reichweite | Chat-Jobs über llama.cpp; Colibri-Chats ebenfalls, wenn dessen Client eine System-Nachricht tragen kann — sonst sagt die UI dort ehrlich „Persona wird von dieser Laufzeit nicht unterstützt". Agents, Story Studio, Benchmarks, Upgrade-Check: unberührt. |
| Inhalt | Der System-Prompt wird unverändert durchgereicht (das Tool filtert nichts). Grenzen nur technisch: Name 1–60 Zeichen, Icon nicht leer, ≤ 64 Bytes (ein Emoji, ZWJ-Sequenzen inklusive), Prompt 1–8000 Zeichen. |
| Vorlagen | Keine geseedeten Zeilen. Der „Neue Persona"-Dialog bietet drei Startvorlagen (knapp & direkt / Coding-Partner / geduldiger Tutor) als reine UI-Konstanten an. |

## Abschnitt 1 — Core

- Migration `0018_personas.sql`; `db::personas` Repo (`create/get/list/update/delete`), Validierung
  in einem `persona::validate`-Modul (Grenzen oben, getrimmt, leere Eingaben → `CoreError::Config`
  = 400).
- `SessionRepo`: `set_persona(id, mode, persona_id)`; `Session` trägt `persona_mode`, `persona_id`.
- `persona::resolve(db, session_id) -> Option<Persona>` mit der Auflösungsregel oben, inklusive
  Selbstheilung bei verwaister Id.
- `chat_body(.., system: Option<&str>)` bzw. ein Feld in `GenerationOptions`; `capability::chat::run`
  ruft `resolve`, reicht den Prompt durch und schreibt `persona` in die Params zurück.
- API: `GET/POST /personas`, `PUT/DELETE /personas/{id}`, `GET/PUT /personas/active` (`{ id | null }`),
  `PUT /sessions/{id}/persona` (`{ mode, persona_id? }`). Tauri-Commands spiegeln das.

## Abschnitt 2 — UI (Chat-Tab)

Persona-Chip im Chat-Kopf (Icon + Name der *wirksamen* Persona, mit Herkunft „global" /
„dieser Chat"); Klick öffnet ein Menü: global aktive Persona wählen, Override für diesen Chat
(erben / keine / bestimmte), „Personas verwalten…". Verwalten-Dialog: Liste, anlegen (mit
Vorlagen), bearbeiten, löschen (mit Bestätigung; Hinweis, wie viele Chats sie nutzen ist nicht
nötig). Jede Antwort im Verlauf zeigt klein Icon + Name der Persona aus den Job-Params. Ohne
Session („Ungrouped") wirkt nur die globale Persona.

## Abschnitt 3 — Tests / Beweis

Unit: Validierung, Repo-Roundtrip, Auflösungsregel (alle vier Fälle + verwaiste Id),
`chat_body` mit/ohne System-Nachricht (Default byte-identisch). Integration
(`core/tests/chat_job.rs` über `aiwm-fake-llama` `/__test/last_request`): mit aktiver Persona trägt
der Request die System-Nachricht vor der User-Nachricht und der Job die `persona`-Params; mit
Session-Override `none` nicht; gelöschte Persona → Chat läuft ohne. API-Tests (CRUD, 400er,
404er). UI live über dev-mock. Ein echter Lauf mit einem installierten Modell: gleiche Frage mit
und ohne Persona, Unterschied sichtbar.

## Nicht enthalten

Personas für Agents/Story Studio, Variablen/Platzhalter im Prompt, Import/Export, Persona pro
Modell, mehrstufiger Gesprächsverlauf (der Chat schickt weiterhin nur die aktuelle Nachricht —
das ist ein eigener, größerer Slice: „Kompaktions-Records für lange Chats").
