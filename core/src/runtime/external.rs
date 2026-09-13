//! Bring-your-own-engine detection (7.x): rather than installing AIWM's own
//! `llama-server`, a user who already has Ollama, LM Studio, or a standalone
//! OpenAI-compatible server running locally can attach to it instead. This
//! module only *detects* what is already running — attaching it to the
//! llama.cpp runtime slot is [`super::LlamaCppAdapter::attach_external`].
//!
//! Detection probes `GET /v1/models`, the one endpoint shape Ollama, LM
//! Studio and llama.cpp itself all implement, rather than anything
//! engine-specific — so this same probe also catches any other
//! OpenAI-compatible server someone happens to run on one of these ports.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

/// A `(label, port)` people commonly already have one of these listening on.
const WELL_KNOWN: &[(&str, u16)] = &[("Ollama", 11434), ("LM Studio", 1234)];

/// A `GET /v1/models` probe gets a short timeout — this only ever hits
/// loopback, so anything alive answers near-instantly, and a machine with
/// neither engine installed shouldn't make every page feel slow.
const PROBE_TIMEOUT: Duration = Duration::from_millis(600);

/// One already-running local server found by [`detect`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedEngine {
    pub label: String,
    pub port: u16,
    /// Model ids it reports serving, from `GET /v1/models`'s `data[].id`.
    pub models: Vec<String>,
}

/// Probe every well-known port in parallel; only the ones that actually
/// answer are returned.
pub async fn detect() -> Vec<DetectedEngine> {
    detect_ports(WELL_KNOWN).await
}

async fn detect_ports(candidates: &[(&str, u16)]) -> Vec<DetectedEngine> {
    let probes = candidates.iter().map(|&(label, port)| async move {
        list_models(port).await.map(|models| DetectedEngine {
            label: label.to_string(),
            port,
            models,
        })
    });
    futures_util::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// `None` when nothing answers on `port`, or the answer isn't a `/v1/models`
/// shape at all -- a closed port and a wrong-shaped service look the same to
/// a caller of [`detect`], which only cares "is there something I can attach".
async fn list_models(port: u16) -> Option<Vec<String>> {
    let resp = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/v1/models"))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let body: Value = resp.json().await.ok()?;
    let ids = body
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    Some(ids)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use axum::routing::get;
    use axum::{Json, Router};

    use super::*;

    /// Serve a minimal `GET /v1/models` on a fresh loopback port.
    async fn spawn_openai_models(model_ids: &'static [&'static str]) -> u16 {
        let router = Router::new().route(
            "/v1/models",
            get(move || async move {
                Json(serde_json::json!({
                    "data": model_ids.iter().map(|id| serde_json::json!({ "id": id })).collect::<Vec<_>>(),
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        port
    }

    /// A port nothing is listening on -- reserved then immediately released.
    async fn dead_port() -> u16 {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        listener.local_addr().unwrap().port()
    }

    #[tokio::test]
    async fn detect_ports_only_returns_the_ones_that_answer() {
        let alive = spawn_openai_models(&["llama3.1:8b"]).await;
        let dead = dead_port().await;

        let found = detect_ports(&[("Fake Engine", alive), ("Nothing Here", dead)]).await;

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "Fake Engine");
        assert_eq!(found[0].port, alive);
        assert_eq!(found[0].models, vec!["llama3.1:8b".to_string()]);
    }

    #[tokio::test]
    async fn detect_ports_lists_every_model_a_server_reports() {
        let alive = spawn_openai_models(&["a", "b", "c"]).await;

        let found = detect_ports(&[("Fake Engine", alive)]).await;

        assert_eq!(found[0].models, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn detect_ports_returns_nothing_when_every_candidate_is_dead() {
        let dead_a = dead_port().await;
        let dead_b = dead_port().await;

        let found = detect_ports(&[("A", dead_a), ("B", dead_b)]).await;

        assert!(found.is_empty());
    }
}
