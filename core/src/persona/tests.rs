//! Unit tests for the persona limits and the resolution rule.

use super::*;

async fn db() -> Database {
    Database::connect_in_memory().await.unwrap()
}

// --- validate ---------------------------------------------------------------

#[test]
fn validate_trims_and_accepts_an_ordinary_persona() {
    let v = validate(
        "  Blunt  ",
        " 🪓 ",
        "  Answer in at most three sentences.  ",
    )
    .unwrap();
    assert_eq!(
        v,
        ValidPersona {
            name: "Blunt".into(),
            icon: "🪓".into(),
            system_prompt: "Answer in at most three sentences.".into(),
        }
    );
}

#[test]
fn validate_rejects_empty_or_whitespace_only_fields() {
    for (name, icon, prompt) in [
        ("", "🙂", "p"),
        ("   ", "🙂", "p"),
        ("N", "", "p"),
        ("N", "   ", "p"),
        ("N", "🙂", ""),
        ("N", "🙂", " \n\t "),
    ] {
        let err = validate(name, icon, prompt).unwrap_err();
        assert!(
            matches!(err, CoreError::Config(_)),
            "{name:?}/{icon:?}/{prompt:?} → {err}"
        );
    }
}

/// The boundary, counted in characters after trimming — not bytes, so a name of
/// 60 multi-byte characters is still fine.
#[test]
fn validate_accepts_a_60_char_name_and_rejects_61() {
    assert!(validate(&"a".repeat(60), "🙂", "p").is_ok());
    assert!(validate(&"ä".repeat(60), "🙂", "p").is_ok());
    assert!(validate(&"🙂".repeat(60), "🙂", "p").is_ok());

    let err = validate(&"a".repeat(61), "🙂", "p").unwrap_err();
    assert!(err.to_string().contains("at most 60 characters"), "{err}");
    assert!(validate(&"🙂".repeat(61), "🙂", "p").is_err());
    // Trailing whitespace is trimmed before counting, so this still fits.
    assert!(validate(&format!("{}   ", "a".repeat(60)), "🙂", "p").is_ok());
}

/// A single emoji can be a long multi-codepoint ZWJ sequence — the family and
/// couple emoji run well past 20 bytes. All of these are *one* emoji a user may
/// reasonably pick, so all of them must pass; the limit only stops the field
/// being used as a second prompt.
#[test]
fn validate_accepts_long_zwj_emoji_icons_and_rejects_an_oversized_one() {
    for icon in ["🙂", "🪓", "🧑‍🏫", "👩‍🚀", "🏳️‍🌈", "👩‍❤️‍👨", "👨‍👩‍👧‍👦"]
    {
        assert!(
            validate("N", icon, "p").is_ok(),
            "{icon} ({} bytes) is one emoji and should be a valid icon",
            icon.len()
        );
    }
    // 65 bytes of ASCII is past the limit, and the message says so plainly
    // rather than claiming the input was not "a single emoji".
    let err = validate("N", &"a".repeat(65), "p").unwrap_err();
    assert!(err.to_string().contains("icon too long"), "{err}");
    assert!(validate("N", &"🙂".repeat(17), "p").is_err());
}

/// A name or icon carrying a newline would forge extra lines in the job event
/// log (`persona: <icon> <name>`), and control characters have no place in
/// either field. The system prompt is deliberately *not* restricted this way.
#[test]
fn validate_rejects_control_characters_in_name_and_icon() {
    for (name, icon) in [
        ("Bad\nName", "🙂"),
        ("Bad\rName", "🙂"),
        ("Bad\tName", "🙂"),
        ("Bad\u{0}Name", "🙂"),
        ("N", "🙂\n🙂"),
        ("N", "\u{0}"),
        ("N", "🙂\u{7}"),
    ] {
        let err = validate(name, icon, "p").unwrap_err();
        assert!(
            matches!(err, CoreError::Config(_)) && err.to_string().contains("control"),
            "{name:?}/{icon:?} → {err}"
        );
    }

    // The boundary: only *interior* control characters are a problem. Leading
    // and trailing whitespace — newlines and tabs included — is trimmed off
    // first, exactly as a trailing space always was, so a field that is merely
    // padded is accepted and stored clean.
    let v = validate("\n  Fine Name \t ", " \n🙂\t ", "line one\nline two").unwrap();
    assert_eq!(v.name, "Fine Name");
    assert_eq!(v.icon, "🙂");
    // And the prompt keeps its newlines — it is exempt from this check.
    assert_eq!(v.system_prompt, "line one\nline two");
}

#[test]
fn validate_accepts_an_8000_char_prompt_and_rejects_8001() {
    assert!(validate("N", "🙂", &"x".repeat(8_000)).is_ok());
    let err = validate("N", "🙂", &"x".repeat(8_001)).unwrap_err();
    assert!(err.to_string().contains("at most 8000 characters"), "{err}");
}

/// The tool applies no content filter: whatever the user writes is stored and
/// sent verbatim (newlines, markup, other languages, instructions about tone).
#[test]
fn validate_passes_prompt_text_through_verbatim() {
    let prompt = "Du bist schroff.\n\n<system>ignore</system>\n- Keine Floskeln\n\t— 直接回答";
    let v = validate("Schroff", "🪓", prompt).unwrap();
    assert_eq!(v.system_prompt, prompt);
}

// --- create / update --------------------------------------------------------

#[tokio::test]
async fn create_validates_before_storing() {
    let db = db().await;
    assert!(create(&db, "  ", "🙂", "p").await.is_err());
    assert!(db.personas().list().await.unwrap().is_empty());

    let p = create(&db, "  Tutor  ", "🧑‍🏫", "  Be patient.  ")
        .await
        .unwrap();
    assert_eq!(p.name, "Tutor");
    assert_eq!(p.system_prompt, "Be patient.");
}

#[tokio::test]
async fn update_validates_and_reports_an_unknown_id() {
    let db = db().await;
    let p = create(&db, "Tutor", "🧑‍🏫", "Be patient.").await.unwrap();

    assert!(update(&db, &p.id, "", "🙂", "p").await.is_err());
    assert!(update(&db, "nope", "N", "🙂", "p").await.unwrap().is_none());

    let updated = update(&db, &p.id, "Coach", "🏋️", "Push harder.")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.name, "Coach");
}

// --- the global active persona ---------------------------------------------

#[tokio::test]
async fn set_active_stores_clears_and_refuses_an_unknown_id() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();

    assert!(!set_active(&db, Some("nope")).await.unwrap());
    assert!(active(&db).await.unwrap().is_none(), "nothing was written");

    assert!(set_active(&db, Some(&p.id)).await.unwrap());
    assert_eq!(active(&db).await.unwrap().unwrap().id, p.id);

    assert!(set_active(&db, None).await.unwrap());
    assert!(active(&db).await.unwrap().is_none());
    // Cleared means the key is gone, not present-but-empty.
    assert_eq!(db.settings().get(ACTIVE_PERSONA_KEY).await.unwrap(), None);
}

/// An empty id is not a way to clear the global persona — that is what `null`
/// is for. It names no persona, so it is refused like any unknown id.
#[tokio::test]
async fn set_active_treats_an_empty_id_as_unknown_rather_than_a_clear() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    set_active(&db, Some(&p.id)).await.unwrap();

    assert!(!set_active(&db, Some("")).await.unwrap());
    assert_eq!(
        active(&db).await.unwrap().unwrap().id,
        p.id,
        "the active persona must be untouched"
    );
}

/// The heal must not clobber a persona set concurrently: by the time the reader
/// notices id A is stale, the key may already hold a valid id B.
#[tokio::test]
async fn active_heals_only_a_key_that_still_holds_the_stale_id() {
    let db = db().await;
    let fresh = create(&db, "Fresh", "🙂", "fresh prompt").await.unwrap();
    db.settings()
        .set(ACTIVE_PERSONA_KEY, "long-gone")
        .await
        .unwrap();

    // Someone sets a valid persona between the stale read and the heal.
    db.settings()
        .set(ACTIVE_PERSONA_KEY, &fresh.id)
        .await
        .unwrap();
    heal_stale_active(&db, "long-gone").await.unwrap();

    assert_eq!(
        active(&db).await.unwrap().unwrap().id,
        fresh.id,
        "the concurrently set persona must survive the heal"
    );
}

/// A key left over from a persona that has since been deleted by some other
/// path must read as "no persona" *and* be cleaned up.
#[tokio::test]
async fn active_heals_a_dangling_global_key() {
    let db = db().await;
    db.settings()
        .set(ACTIVE_PERSONA_KEY, "long-gone")
        .await
        .unwrap();

    assert!(active(&db).await.unwrap().is_none());
    assert_eq!(
        db.settings().get(ACTIVE_PERSONA_KEY).await.unwrap(),
        None,
        "the dangling key must be cleared, not just ignored"
    );
}

// --- the session override ---------------------------------------------------

#[tokio::test]
async fn set_session_persona_stores_each_mode() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();

    assert_eq!(
        set_session_persona(&db, &s.id, PersonaMode::Persona, Some(&p.id))
            .await
            .unwrap(),
        SetSessionPersona::Stored
    );
    assert_eq!(
        set_session_persona(&db, &s.id, PersonaMode::None, None)
            .await
            .unwrap(),
        SetSessionPersona::Stored
    );
    assert_eq!(
        set_session_persona(&db, &s.id, PersonaMode::Inherit, None)
            .await
            .unwrap(),
        SetSessionPersona::Stored
    );
}

#[tokio::test]
async fn set_session_persona_reports_unknown_ids_and_refuses_a_missing_persona_id() {
    let db = db().await;
    // A real persona exists, so "unknown persona" below is about *that* id
    // being wrong, not about the table being empty.
    let existing = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();

    assert_eq!(
        set_session_persona(&db, "nope", PersonaMode::Inherit, None)
            .await
            .unwrap(),
        SetSessionPersona::UnknownSession
    );
    assert_eq!(
        set_session_persona(&db, &s.id, PersonaMode::Persona, Some("nope"))
            .await
            .unwrap(),
        SetSessionPersona::UnknownPersona
    );
    // Mode `persona` with no id at all is a malformed request, not a 404.
    let err = set_session_persona(&db, &s.id, PersonaMode::Persona, None)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Config(_)), "{err}");

    // None of the refusals changed the session…
    let stored = db.sessions().get(&s.id).await.unwrap().unwrap();
    assert_eq!(stored.persona_mode, PersonaMode::Inherit);
    assert_eq!(stored.persona_id, None);
    // …and the valid id still works, so nothing above left the repo wedged.
    assert_eq!(
        set_session_persona(&db, &s.id, PersonaMode::Persona, Some(&existing.id))
            .await
            .unwrap(),
        SetSessionPersona::Stored
    );
}

// --- what a chat job does with the resolved persona ---------------------------

#[tokio::test]
async fn prepare_for_job_returns_nothing_and_touches_nothing_without_a_persona() {
    let db = Database::connect_in_memory().await.unwrap();
    let mut new = crate::db::NewJob::new("chat");
    new.params = serde_json::json!({ "prompt": "hi" });
    let job = db.jobs().insert(new).await.unwrap();

    assert!(prepare_for_job(&db, &job.id, None).await.unwrap().is_none());

    let stored = db.jobs().get(&job.id).await.unwrap().unwrap();
    assert!(
        stored.params.get("persona").is_none(),
        "no persona params: {}",
        stored.params
    );
    // Only the "job queued" event `insert` itself wrote.
    let events: Vec<String> = db
        .jobs()
        .events(&job.id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert!(!events.iter().any(|m| m.contains("persona")), "{events:?}");
}

#[tokio::test]
async fn prepare_for_job_returns_the_prompt_and_pins_the_persona_on_the_job() {
    let db = Database::connect_in_memory().await.unwrap();
    let persona = create(&db, "Blunt", "🪓", "Answer in three sentences.")
        .await
        .unwrap();
    set_active(&db, Some(&persona.id)).await.unwrap();
    let mut new = crate::db::NewJob::new("chat");
    new.params = serde_json::json!({ "prompt": "hi" });
    let job = db.jobs().insert(new).await.unwrap();

    let system = prepare_for_job(&db, &job.id, None).await.unwrap();
    assert_eq!(system.as_deref(), Some("Answer in three sentences."));

    let stored = db.jobs().get(&job.id).await.unwrap().unwrap();
    assert_eq!(stored.params["persona"]["id"], persona.id);
    assert_eq!(stored.params["persona"]["name"], "Blunt");
    assert_eq!(stored.params["persona"]["icon"], "🪓");
    assert_eq!(stored.params["prompt"], "hi", "other params survive");
    assert!(
        !stored.params.to_string().contains("three sentences"),
        "the prompt text stays out of the job: {}",
        stored.params
    );

    let events: Vec<String> = db
        .jobs()
        .events(&job.id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(events.last().map(String::as_str), Some("persona: 🪓 Blunt"));
}

/// A session that opted out must not get the global persona.
#[tokio::test]
async fn prepare_for_job_honours_a_session_override_of_none() {
    let db = Database::connect_in_memory().await.unwrap();
    let persona = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    set_active(&db, Some(&persona.id)).await.unwrap();
    let session = db.sessions().create("chat", "Plain").await.unwrap();
    set_session_persona(&db, &session.id, PersonaMode::None, None)
        .await
        .unwrap();
    let job = db
        .jobs()
        .insert(crate::db::NewJob::new("chat"))
        .await
        .unwrap();

    assert!(prepare_for_job(&db, &job.id, Some(&session.id))
        .await
        .unwrap()
        .is_none());
}

/// Bookkeeping is not worth failing a chat over: once the persona has resolved,
/// a failure to write the event or the params must be logged and stepped over,
/// not propagated. `job_events.job_id` is a real FK, so an id with no job behind
/// it makes the event append fail for real — no mocking needed.
#[tokio::test]
async fn prepare_for_job_still_returns_the_prompt_when_the_bookkeeping_fails() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    set_active(&db, Some(&p.id)).await.unwrap();

    let system = prepare_for_job(&db, "no-such-job", None).await.unwrap();
    assert_eq!(
        system.as_deref(),
        Some("be brief"),
        "the chat must still get its system prompt"
    );
}

// --- resolve ----------------------------------------------------------------

#[tokio::test]
async fn resolve_without_a_session_uses_the_global_persona() {
    let db = db().await;
    assert!(resolve(&db, None).await.unwrap().is_none());
    assert_eq!(
        resolve_effective(&db, None).await.unwrap().origin,
        PersonaOrigin::None
    );

    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    set_active(&db, Some(&p.id)).await.unwrap();

    assert_eq!(resolve(&db, None).await.unwrap().unwrap().id, p.id);
    let eff = resolve_effective(&db, None).await.unwrap();
    assert_eq!(eff.origin, PersonaOrigin::Global);
    assert_eq!(eff.persona.unwrap().id, p.id);
}

#[tokio::test]
async fn resolve_inherit_follows_the_global_persona() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    set_active(&db, Some(&p.id)).await.unwrap();

    let eff = resolve_effective(&db, Some(&s.id)).await.unwrap();
    assert_eq!(eff.origin, PersonaOrigin::Global);
    assert_eq!(eff.persona.unwrap().id, p.id);
}

#[tokio::test]
async fn resolve_none_overrides_the_global_persona() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    set_active(&db, Some(&p.id)).await.unwrap();
    set_session_persona(&db, &s.id, PersonaMode::None, None)
        .await
        .unwrap();

    assert!(resolve(&db, Some(&s.id)).await.unwrap().is_none());
    assert_eq!(
        resolve_effective(&db, Some(&s.id)).await.unwrap().origin,
        PersonaOrigin::None
    );
}

#[tokio::test]
async fn resolve_persona_mode_wins_over_the_global_persona() {
    let db = db().await;
    let global_persona = create(&db, "Global", "🌍", "global prompt").await.unwrap();
    let own = create(&db, "Own", "🎯", "own prompt").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    set_active(&db, Some(&global_persona.id)).await.unwrap();
    set_session_persona(&db, &s.id, PersonaMode::Persona, Some(&own.id))
        .await
        .unwrap();

    let eff = resolve_effective(&db, Some(&s.id)).await.unwrap();
    assert_eq!(eff.origin, PersonaOrigin::Session);
    assert_eq!(eff.persona.unwrap().id, own.id);
}

#[tokio::test]
async fn resolve_treats_an_unknown_session_id_as_ungrouped() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    set_active(&db, Some(&p.id)).await.unwrap();

    let eff = resolve_effective(&db, Some("no-such-session"))
        .await
        .unwrap();
    assert_eq!(eff.origin, PersonaOrigin::Global);
    assert_eq!(eff.persona.unwrap().id, p.id);
}

/// A session override pointing at a persona that has been removed behind the
/// repo's back must heal to `inherit` and fall through to the global choice —
/// never fail the chat.
#[tokio::test]
async fn resolve_heals_a_session_pointing_at_a_deleted_persona() {
    let db = db().await;
    let gone = create(&db, "Gone", "💀", "gone prompt").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    set_session_persona(&db, &s.id, PersonaMode::Persona, Some(&gone.id))
        .await
        .unwrap();
    // Remove the row only, bypassing the healing delete, to simulate a stale
    // pointer from any other source.
    sqlx::query("DELETE FROM personas WHERE id = $1")
        .bind(&gone.id)
        .execute(db.pool())
        .await
        .unwrap();

    let eff = resolve_effective(&db, Some(&s.id)).await.unwrap();
    assert_eq!(eff.origin, PersonaOrigin::None);
    assert!(eff.persona.is_none());

    let healed = db.sessions().get(&s.id).await.unwrap().unwrap();
    assert_eq!(healed.persona_mode, PersonaMode::Inherit);
    assert_eq!(healed.persona_id, None);
}

/// The session heal must be just as conditional as the global one: if the
/// session was re-pointed at a valid persona between the stale read and the
/// heal, that choice must survive.
#[tokio::test]
async fn resolve_does_not_heal_a_session_that_was_re_pointed_in_the_meantime() {
    let db = db().await;
    let fresh = create(&db, "Fresh", "🙂", "fresh prompt").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    set_session_persona(&db, &s.id, PersonaMode::Persona, Some(&fresh.id))
        .await
        .unwrap();

    // A heal aimed at an id the session no longer holds must be a no-op.
    heal_stale_session(&db, &s.id, Some("long-gone"))
        .await
        .unwrap();

    let kept = db.sessions().get(&s.id).await.unwrap().unwrap();
    assert_eq!(kept.persona_mode, PersonaMode::Persona);
    assert_eq!(kept.persona_id.as_deref(), Some(fresh.id.as_str()));
    let eff = resolve_effective(&db, Some(&s.id)).await.unwrap();
    assert_eq!(eff.origin, PersonaOrigin::Session);
    assert_eq!(eff.persona.unwrap().id, fresh.id);
}

/// Healing a dangling session override still honours a *valid* global persona.
#[tokio::test]
async fn resolve_heals_then_falls_back_to_a_valid_global_persona() {
    let db = db().await;
    let gone = create(&db, "Gone", "💀", "gone prompt").await.unwrap();
    let global_persona = create(&db, "Global", "🌍", "global prompt").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    set_active(&db, Some(&global_persona.id)).await.unwrap();
    set_session_persona(&db, &s.id, PersonaMode::Persona, Some(&gone.id))
        .await
        .unwrap();
    sqlx::query("DELETE FROM personas WHERE id = $1")
        .bind(&gone.id)
        .execute(db.pool())
        .await
        .unwrap();

    let eff = resolve_effective(&db, Some(&s.id)).await.unwrap();
    assert_eq!(eff.origin, PersonaOrigin::Global);
    assert_eq!(eff.persona.unwrap().id, global_persona.id);
}

/// Mode `persona` with a NULL id is corrupt state that only a direct SQL write
/// can produce; it must still resolve, not error.
#[tokio::test]
async fn resolve_heals_mode_persona_with_no_id_at_all() {
    let db = db().await;
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    sqlx::query("UPDATE sessions SET persona_mode = 'persona', persona_id = NULL WHERE id = $1")
        .bind(&s.id)
        .execute(db.pool())
        .await
        .unwrap();

    let eff = resolve_effective(&db, Some(&s.id)).await.unwrap();
    assert_eq!(eff.origin, PersonaOrigin::None);
    let healed = db.sessions().get(&s.id).await.unwrap().unwrap();
    assert_eq!(healed.persona_mode, PersonaMode::Inherit);
}

/// The whole point of the delete transaction plus the healing read: deleting the
/// persona a chat uses leaves that chat working, with no persona.
#[tokio::test]
async fn deleting_the_persona_a_session_uses_leaves_the_chat_working() {
    let db = db().await;
    let p = create(&db, "Blunt", "🪓", "be brief").await.unwrap();
    let s = db.sessions().create("chat", "Chat").await.unwrap();
    set_session_persona(&db, &s.id, PersonaMode::Persona, Some(&p.id))
        .await
        .unwrap();
    set_active(&db, Some(&p.id)).await.unwrap();

    assert!(db.personas().delete(&p.id).await.unwrap());

    assert!(resolve(&db, Some(&s.id)).await.unwrap().is_none());
    assert!(resolve(&db, None).await.unwrap().is_none());
}

#[test]
fn origin_serialises_lowercase() {
    assert_eq!(
        serde_json::to_string(&PersonaOrigin::Session).unwrap(),
        r#""session""#
    );
    assert_eq!(
        serde_json::to_string(&EffectivePersona::none()).unwrap(),
        r#"{"persona":null,"origin":"none"}"#
    );
}
