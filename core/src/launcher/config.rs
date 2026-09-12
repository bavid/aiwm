//! Config builders for launching each tool against AIWM's local model server.
//!
//! Both builders point the tool at whichever `llama-server` endpoint
//! [`super::LlamaCodingRuntime`](crate::LlamaCodingRuntime) just pinned — the
//! same OpenAI-compatible `/v1` base URL the in-app OpenCode/Hermes adapters
//! already use ([`crate::agent::opencode`], the private `agent::hermes`).
//! Unlike those embedded sessions, a launched terminal is the user's own, so
//! these configs don't force AIWM's approval/sandbox settings — the user
//! approves each action directly in the tool's own interactive UI.

use crate::db::Model;

/// Hermes Agent refuses to start against a model declaring less than this —
/// found against a real 0.19.0 run ("context window ... below the minimum
/// 64,000 required by Hermes Agent"). Duplicated from the embedded-session
/// adapter's own constant (private to that module, and this is a different
/// config shape) rather than reused.
pub const HERMES_MIN_CONTEXT: i64 = 64_000;

/// `OPENCODE_CONFIG_CONTENT` JSON: a custom `openai-compatible` provider
/// pointed at `base_url`, pre-selected as the active model. Verified against
/// OpenCode's own docs (`config.mdx` + `providers.mdx` in `sst/opencode`) —
/// this env var sits above project/global config files in precedence, so it
/// fully determines the model without touching any file on disk.
pub fn opencode_config_content(base_url: &str, model: &Model) -> String {
    const PROVIDER_ID: &str = "aiwm-local";

    let mut models = serde_json::Map::new();
    models.insert(model.id.clone(), serde_json::json!({ "name": model.name }));

    let mut providers = serde_json::Map::new();
    providers.insert(
        PROVIDER_ID.to_string(),
        serde_json::json!({
            "npm": "@ai-sdk/openai-compatible",
            "name": "AIWM (local)",
            "options": { "baseURL": base_url },
            "models": models,
        }),
    );

    serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "provider": providers,
        "model": format!("{PROVIDER_ID}/{}", model.id),
    })
    .to_string()
}

/// The `config.yaml` written into a fresh, per-launch `HERMES_HOME`. Same
/// `provider: custom` shape the embedded Hermes adapter already
/// reverse-engineered, minus the sandbox restrictions (`permissions`,
/// `tools`, `privacy`) that only make sense for a session AIWM itself is
/// driving — here the user is at the keyboard, approving each action through
/// Hermes' own interactive prompts.
pub fn hermes_config_yaml(base_url: &str, model: &Model) -> String {
    let name = yaml_scalar(&model.name);
    let base = yaml_scalar(base_url);
    format!(
        "model:\n\
        \x20 default: {name}\n\
        \x20 provider: custom\n\
        \x20 base_url: {base}\n\
        \x20 context_length: {HERMES_MIN_CONTEXT}\n\
        providers:\n\
        \x20 aiwm-local:\n\
        \x20   api: {base}\n\
        \x20   api_key: aiwm-local\n\
        \x20   default_model: {name}\n"
    )
}

/// Quote a value if it needs it — enough for URLs and model names.
fn yaml_scalar(s: &str) -> String {
    if s.is_empty() || s.contains([':', '#', '\n', '"', '\'']) {
        format!("{s:?}")
    } else {
        s.to_string()
    }
}

/// Whether `model` meets Hermes' own hard context-length floor. `None`
/// (unknown `ctx_max`) is treated as "can't confirm it clears the floor" —
/// not a pass. Not a hard block on the caller's side (the user may know
/// better than AIWM's metadata), just a warning surfaced so a refusal to
/// start isn't a mystery.
pub fn hermes_context_warning(model: &Model) -> Option<String> {
    match model.ctx_max {
        Some(ctx) if ctx >= HERMES_MIN_CONTEXT => None,
        Some(ctx) => Some(format!(
            "{} declares a {ctx}-token context; Hermes Agent refuses to start below \
             {HERMES_MIN_CONTEXT} and will likely error immediately.",
            model.name
        )),
        None => Some(format!(
            "{}'s context length isn't known; Hermes Agent requires at least \
             {HERMES_MIN_CONTEXT} tokens and may refuse to start.",
            model.name
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewModel};

    async fn model(db: &Database, ctx_max: Option<i64>) -> Model {
        let id = db
            .models()
            .insert(NewModel {
                name: "Qwen2.5 Coder".into(),
                format: "gguf".into(),
                file_path: "E:\\models\\coder.gguf".into(),
                size_bytes: 4096,
                source: "manual".into(),
                ctx_max,
                roles: vec!["coding".into()],
                ..NewModel::default()
            })
            .await
            .unwrap()
            .id;
        db.models().get(&id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn opencode_config_content_points_the_custom_provider_at_the_endpoint() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, None).await;

        let json = opencode_config_content("http://127.0.0.1:41234/v1", &m);
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert_eq!(
            v["provider"]["aiwm-local"]["options"]["baseURL"],
            "http://127.0.0.1:41234/v1"
        );
        assert_eq!(
            v["provider"]["aiwm-local"]["npm"],
            "@ai-sdk/openai-compatible"
        );
        assert_eq!(
            v["provider"]["aiwm-local"]["models"][&m.id]["name"],
            "Qwen2.5 Coder"
        );
        assert_eq!(v["model"], format!("aiwm-local/{}", m.id));
    }

    #[tokio::test]
    async fn opencode_config_content_escapes_model_names_needing_it() {
        let db = Database::connect_in_memory().await.unwrap();
        let mut m = model(&db, None).await;
        m.name = "Weird \"Model\" \\ Name".into();

        let json = opencode_config_content("http://127.0.0.1:1/v1", &m);
        let v: serde_json::Value = serde_json::from_str(&json).expect("still valid JSON");
        assert_eq!(v["provider"]["aiwm-local"]["models"][&m.id]["name"], m.name);
    }

    #[tokio::test]
    async fn hermes_config_yaml_carries_the_custom_provider_and_context_floor() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, None).await;

        let yaml = hermes_config_yaml("http://127.0.0.1:41234/v1", &m);

        assert!(yaml.contains("provider: custom"));
        // Colons in the URL force `yaml_scalar` to quote it.
        assert!(yaml.contains("base_url: \"http://127.0.0.1:41234/v1\""));
        assert!(yaml.contains(&format!("context_length: {HERMES_MIN_CONTEXT}")));
        assert!(yaml.contains("default: Qwen2.5 Coder"));
        // No sandbox restrictions -- this is the user's own session.
        assert!(!yaml.contains("permissions"));
        assert!(!yaml.contains("tools"));
    }

    #[tokio::test]
    async fn hermes_context_warning_is_none_when_the_floor_is_met() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, Some(HERMES_MIN_CONTEXT)).await;
        assert_eq!(hermes_context_warning(&m), None);
    }

    #[tokio::test]
    async fn hermes_context_warning_fires_below_the_floor() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, Some(32_000)).await;
        let w = hermes_context_warning(&m).expect("should warn");
        assert!(w.contains("32000"), "{w}");
        assert!(w.contains("64_000") || w.contains("64000"), "{w}");
    }

    #[tokio::test]
    async fn hermes_context_warning_fires_when_unknown() {
        let db = Database::connect_in_memory().await.unwrap();
        let m = model(&db, None).await;
        let w = hermes_context_warning(&m).expect("should warn");
        assert!(w.contains("isn't known"), "{w}");
    }
}
