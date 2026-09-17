//! Personas: the rules around the stored rows in [`crate::db::personas`].
//!
//! Three things live here, all deliberately away from storage and transport:
//!
//! * [`validate`] — the only limits a persona has. They are technical, not
//!   editorial: the system prompt is passed to the model verbatim and this tool
//!   applies no content filter.
//! * [`resolve`] / [`resolve_effective`] — which persona a chat actually gets:
//!   the session's own override wins, otherwise the globally active one. Both
//!   heal a dangling id on the spot, so a chat can never fail because a persona
//!   was deleted out from under it.
//! * [`set_active`] / [`set_session_persona`] — the two write paths the API
//!   exposes, with the "does it exist?" checks the 404s are made of.

use serde::Serialize;

use crate::db::{Database, EventLevel, Persona, PersonaMode, ACTIVE_PERSONA_KEY};
use crate::{CoreError, Result};

/// Name length limits, in characters (not bytes) after trimming.
const NAME_MAX_CHARS: usize = 60;
/// An icon is one emoji. A single emoji can be a multi-codepoint ZWJ sequence
/// (🧑‍🏫 is 11 bytes, 👩‍❤️‍👨 is 20+), so the limit is generous and in bytes —
/// enough for any ordinary emoji, small enough that the column never becomes a
/// second prompt field.
const ICON_MAX_BYTES: usize = 16;
/// System-prompt length limit, in characters after trimming.
const PROMPT_MAX_CHARS: usize = 8_000;

/// A validated persona's three fields, trimmed and ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidPersona {
    pub name: String,
    pub icon: String,
    pub system_prompt: String,
}

/// Where the persona a chat will use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PersonaOrigin {
    /// This chat session overrides the global choice with its own persona.
    Session,
    /// No session override — the globally active persona applies.
    Global,
    /// No persona at all: nothing is active globally, or this chat opted out.
    None,
}

/// The persona a chat will use, plus where it came from, so the UI can label the
/// chip ("global" / "this chat") without re-implementing the rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectivePersona {
    pub persona: Option<Persona>,
    pub origin: PersonaOrigin,
}

impl EffectivePersona {
    fn none() -> Self {
        Self {
            persona: None,
            origin: PersonaOrigin::None,
        }
    }
}

/// What [`set_session_persona`] found. Two of the three arms are 404s at the
/// API; a missing `persona_id` for mode `persona` is a malformed request and
/// comes back as [`CoreError::Config`] (400) instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetSessionPersona {
    Stored,
    UnknownSession,
    UnknownPersona,
}

/// The only limits a persona has. Returns the trimmed fields to store.
pub fn validate(name: &str, icon: &str, system_prompt: &str) -> Result<ValidPersona> {
    let name = name.trim();
    let icon = icon.trim();
    let system_prompt = system_prompt.trim();

    if name.is_empty() {
        return Err(CoreError::Config("persona name must not be empty".into()));
    }
    let name_chars = name.chars().count();
    if name_chars > NAME_MAX_CHARS {
        return Err(CoreError::Config(format!(
            "persona name must be at most {NAME_MAX_CHARS} characters ({name_chars} given)"
        )));
    }
    if icon.is_empty() {
        return Err(CoreError::Config("persona icon must not be empty".into()));
    }
    if icon.len() > ICON_MAX_BYTES {
        return Err(CoreError::Config(format!(
            "persona icon must be a single emoji (at most {ICON_MAX_BYTES} bytes, {} given)",
            icon.len()
        )));
    }
    if system_prompt.is_empty() {
        return Err(CoreError::Config(
            "persona system prompt must not be empty".into(),
        ));
    }
    let prompt_chars = system_prompt.chars().count();
    if prompt_chars > PROMPT_MAX_CHARS {
        return Err(CoreError::Config(format!(
            "persona system prompt must be at most {PROMPT_MAX_CHARS} characters \
             ({prompt_chars} given)"
        )));
    }

    Ok(ValidPersona {
        name: name.to_string(),
        icon: icon.to_string(),
        system_prompt: system_prompt.to_string(),
    })
}

/// Validate, then store.
pub async fn create(db: &Database, name: &str, icon: &str, system_prompt: &str) -> Result<Persona> {
    let v = validate(name, icon, system_prompt)?;
    db.personas()
        .create(&v.name, &v.icon, &v.system_prompt)
        .await
}

/// Validate, then overwrite. `None` = no such persona (404).
pub async fn update(
    db: &Database,
    id: &str,
    name: &str,
    icon: &str,
    system_prompt: &str,
) -> Result<Option<Persona>> {
    let v = validate(name, icon, system_prompt)?;
    db.personas()
        .update(id, &v.name, &v.icon, &v.system_prompt)
        .await
}

/// The globally active persona, healing the settings key if it names a persona
/// that no longer exists.
pub async fn active(db: &Database) -> Result<Option<Persona>> {
    let stored = db.settings().get(ACTIVE_PERSONA_KEY).await?;
    let Some(id) = stored.filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    match db.personas().get(&id).await? {
        Some(persona) => Ok(Some(persona)),
        None => {
            db.settings().clear(ACTIVE_PERSONA_KEY).await?;
            Ok(None)
        }
    }
}

/// Set (`Some`) or clear (`None`) the globally active persona. `false` = the id
/// names no persona (404); nothing is written in that case.
pub async fn set_active(db: &Database, id: Option<&str>) -> Result<bool> {
    let Some(id) = id.filter(|id| !id.is_empty()) else {
        db.settings().clear(ACTIVE_PERSONA_KEY).await?;
        return Ok(true);
    };
    if db.personas().get(id).await?.is_none() {
        return Ok(false);
    }
    db.settings().set(ACTIVE_PERSONA_KEY, id).await?;
    Ok(true)
}

/// Store a chat session's persona override.
pub async fn set_session_persona(
    db: &Database,
    session_id: &str,
    mode: PersonaMode,
    persona_id: Option<&str>,
) -> Result<SetSessionPersona> {
    if mode == PersonaMode::Persona {
        let Some(persona_id) = persona_id else {
            return Err(CoreError::Config(
                "persona mode \"persona\" needs a persona_id".into(),
            ));
        };
        if db.personas().get(persona_id).await?.is_none() {
            return Ok(SetSessionPersona::UnknownPersona);
        }
    }
    if db
        .sessions()
        .set_persona(session_id, mode, persona_id)
        .await?
    {
        Ok(SetSessionPersona::Stored)
    } else {
        Ok(SetSessionPersona::UnknownSession)
    }
}

/// What a chat job does at its start: [`resolve`] the persona for its session
/// and, when there is one, record it on the job — one info event
/// `persona: <icon> <name>` plus `persona: { id, name, icon }` written back into
/// the job's params, so the history still shows which persona answered after it
/// has been renamed or deleted. Returns its system prompt, which is deliberately
/// *not* stored on the job.
///
/// No persona means no event, no params change, and no system message — the
/// request stays exactly what it was before personas existed. Shared by the
/// llama.cpp chat body ([`crate::capability::chat`]) and the Colibri one
/// ([`crate::capability::colibri`]).
pub async fn prepare_for_job(
    db: &Database,
    job_id: &str,
    session_id: Option<&str>,
) -> Result<Option<String>> {
    let Some(persona) = resolve(db, session_id).await? else {
        return Ok(None);
    };

    db.jobs()
        .append_event(
            job_id,
            EventLevel::Info,
            &format!("persona: {} {}", persona.icon, persona.name),
        )
        .await?;

    if let Some(job) = db.jobs().get(job_id).await? {
        let mut params = job.params;
        persona.apply_to(&mut params);
        db.jobs().set_params(job_id, &params).await?;
    }

    Ok(Some(persona.system_prompt))
}

/// The persona a chat in `session_id` gets, or `None` for "no system prompt".
/// `None` for `session_id` is the ungrouped case, where only the global persona
/// applies.
pub async fn resolve(db: &Database, session_id: Option<&str>) -> Result<Option<Persona>> {
    Ok(resolve_effective(db, session_id).await?.persona)
}

/// [`resolve`] plus where the answer came from.
///
/// A dangling id — a session pointing at a deleted persona, or a global key
/// naming one — is *healed* here rather than reported: the stale pointer is
/// cleared and resolution carries on. That is deliberate; a chat must never fail
/// because a persona was deleted between two messages.
pub async fn resolve_effective(
    db: &Database,
    session_id: Option<&str>,
) -> Result<EffectivePersona> {
    let Some(session_id) = session_id else {
        return global(db).await;
    };
    // An unknown session id behaves like the ungrouped case rather than an
    // error — the job may outlive the session it was submitted from.
    let Some(session) = db.sessions().get(session_id).await? else {
        return global(db).await;
    };

    match session.persona_mode {
        PersonaMode::None => Ok(EffectivePersona::none()),
        PersonaMode::Inherit => global(db).await,
        PersonaMode::Persona => {
            if let Some(persona) = session_persona(db, &session.persona_id).await? {
                return Ok(EffectivePersona {
                    persona: Some(persona),
                    origin: PersonaOrigin::Session,
                });
            }
            // The override pointed at nothing (deleted, or never set). Heal the
            // session back to `inherit` and fall back to the global choice.
            db.sessions()
                .set_persona(session_id, PersonaMode::Inherit, None)
                .await?;
            global(db).await
        }
    }
}

async fn session_persona(db: &Database, persona_id: &Option<String>) -> Result<Option<Persona>> {
    let Some(id) = persona_id else {
        return Ok(None);
    };
    db.personas().get(id).await
}

async fn global(db: &Database) -> Result<EffectivePersona> {
    match active(db).await? {
        Some(persona) => Ok(EffectivePersona {
            persona: Some(persona),
            origin: PersonaOrigin::Global,
        }),
        None => Ok(EffectivePersona::none()),
    }
}

#[cfg(test)]
mod tests;
