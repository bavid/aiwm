//! The persona HTTP surface driven end-to-end over a real loopback server:
//! CRUD, the limits (400), unknown ids (404), the globally active persona
//! (including clearing it), the per-session override, and the resolved
//! "effective persona" the Chat tab's chip reads. No llama.cpp is involved —
//! what a chat job then *does* with a persona is covered by `chat_job.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use aiwm_core::{ApiServer, App, AppOptions, AppPaths};

/// A live server over a fresh store. The `TempDir` must outlive the test (it
/// backs the `App`'s data directory); the training poller is off — nothing here
/// touches training runs.
async fn fixture() -> (ApiServer, tempfile::TempDir, Arc<App>) {
    let tmp = tempfile::tempdir().unwrap();
    let app = Arc::new(
        App::load_with(
            AppPaths::rooted(tmp.path()),
            AppOptions {
                training_poller: false,
            },
        )
        .await
        .unwrap(),
    );
    let server = ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    (server, tmp, app)
}

fn body(name: &str, icon: &str, prompt: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "icon": icon, "system_prompt": prompt })
}

async fn create(base: &str, name: &str, icon: &str, prompt: &str) -> serde_json::Value {
    let resp = reqwest::Client::new()
        .post(format!("{base}/personas"))
        .json(&body(name, icon, prompt))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    resp.json().await.unwrap()
}

#[tokio::test]
async fn crud_round_trip_over_http() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);

    // Nothing to begin with — no seeded rows.
    let list: serde_json::Value = reqwest::get(format!("{base}/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.as_array().unwrap().is_empty(), "{list}");

    let created = create(&base, "  Blunt  ", " 🪓 ", "  Answer in three sentences.  ").await;
    let id = created["id"].as_str().unwrap().to_string();
    // The stored fields are trimmed, and the row carries both timestamps.
    assert_eq!(created["name"], "Blunt");
    assert_eq!(created["icon"], "🪓");
    assert_eq!(created["system_prompt"], "Answer in three sentences.");
    assert!(!created["created_at"].as_str().unwrap().is_empty());
    assert!(!created["updated_at"].as_str().unwrap().is_empty());

    create(&base, "Apple", "🍎", "be fruity").await;
    let list: serde_json::Value = reqwest::get(format!("{base}/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Apple", "Blunt"], "listed by name");

    let resp = reqwest::Client::new()
        .put(format!("{base}/personas/{id}"))
        .json(&body("Coach", "🏋️", "Push harder."))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let updated: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(updated["id"], id);
    assert_eq!(updated["name"], "Coach");
    assert_eq!(updated["system_prompt"], "Push harder.");

    let resp = reqwest::Client::new()
        .delete(format!("{base}/personas/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let list: serde_json::Value = reqwest::get(format!("{base}/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn invalid_fields_are_refused_with_400() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();

    let cases = [
        body("", "🙂", "p"),
        body("   ", "🙂", "p"),
        body("N", "", "p"),
        // Past the 64-byte icon cap (real ZWJ emoji reach ~25 bytes, so the cap
        // only stops the field being used as a second prompt).
        body("N", &"a".repeat(65), "p"),
        // A control character in a field that is rendered on one line.
        body("Bad\nName", "🙂", "p"),
        body("N", "🙂\n🙂", "p"),
        body("N", "🙂", ""),
        body(&"a".repeat(61), "🙂", "p"),
        body("N", "🙂", &"x".repeat(8_001)),
    ];
    for case in &cases {
        let resp = client
            .post(format!("{base}/personas"))
            .json(case)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "{case}");
        let err: serde_json::Value = resp.json().await.unwrap();
        assert!(
            err["error"].as_str().unwrap().contains("persona"),
            "{err} for {case}"
        );
    }

    // Nothing was stored by any of them.
    let list: serde_json::Value = reqwest::get(format!("{base}/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.as_array().unwrap().is_empty(), "{list}");
}

/// The same limits apply on update, and an invalid body must not modify the row.
#[tokio::test]
async fn update_validates_too_and_leaves_the_row_alone() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let created = create(&base, "Blunt", "🪓", "be brief").await;
    let id = created["id"].as_str().unwrap();

    let resp = reqwest::Client::new()
        .put(format!("{base}/personas/{id}"))
        .json(&body("", "🙂", "p"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let list: serde_json::Value = reqwest::get(format!("{base}/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["name"], "Blunt");
}

#[tokio::test]
async fn unknown_ids_give_404_and_deleting_one_is_idempotent() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();

    let resp = client
        .put(format!("{base}/personas/no-such-id"))
        .json(&body("N", "🙂", "p"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let resp = client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({ "id": "no-such-id" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Deleting something that is already gone is a success, like sessions and
    // documents — the caller's intent is satisfied either way.
    let resp = client
        .delete(format!("{base}/personas/no-such-id"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

#[tokio::test]
async fn the_active_persona_can_be_set_read_and_cleared_with_null() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let created = create(&base, "Blunt", "🪓", "be brief").await;
    let id = created["id"].as_str().unwrap().to_string();

    let active: serde_json::Value = reqwest::get(format!("{base}/personas/active"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(active["id"].is_null(), "nothing active to begin with");

    let resp = client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({ "id": id }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let active: serde_json::Value = reqwest::get(format!("{base}/personas/active"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(active["id"], id);

    let resp = client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({ "id": serde_json::Value::Null }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let active: serde_json::Value = reqwest::get(format!("{base}/personas/active"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(active["id"].is_null());
}

/// Clearing the global persona has exactly one spelling: `{"id": null}`. An
/// absent key is a malformed body, and an empty string names no persona — both
/// must be refused rather than silently clearing the user's choice.
#[tokio::test]
async fn only_an_explicit_null_clears_the_active_persona() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let created = create(&base, "Blunt", "🪓", "be brief").await;
    let id = created["id"].as_str().unwrap().to_string();
    client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({ "id": id }))
        .send()
        .await
        .unwrap();

    // No `id` key at all: the DTO cannot be built.
    let resp = client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);

    // An empty string is an unknown id, not a clear.
    let resp = client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({ "id": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Neither of them changed anything.
    let active: serde_json::Value = reqwest::get(format!("{base}/personas/active"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(active["id"], id);
}

/// `/personas/active` and `/personas/effective` are literal segments, so they
/// never fall through to `/personas/{id}`: `DELETE /personas/active` has no
/// handler at all rather than deleting a persona whose id happens to be
/// "active".
#[tokio::test]
async fn the_literal_persona_routes_do_not_collide_with_the_id_route() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);

    let resp = reqwest::Client::new()
        .delete(format!("{base}/personas/active"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405, "method not allowed, not a delete");
}

/// Deleting the globally active persona must clear the key, not leave a
/// dangling id behind.
#[tokio::test]
async fn deleting_the_active_persona_clears_the_active_key() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let created = create(&base, "Blunt", "🪓", "be brief").await;
    let id = created["id"].as_str().unwrap().to_string();
    client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({ "id": id }))
        .send()
        .await
        .unwrap();

    client
        .delete(format!("{base}/personas/{id}"))
        .send()
        .await
        .unwrap();

    let active: serde_json::Value = reqwest::get(format!("{base}/personas/active"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(active["id"].is_null(), "{active}");
}

async fn new_session(base: &str, name: &str) -> String {
    let resp = reqwest::Client::new()
        .post(format!("{base}/sessions"))
        .json(&serde_json::json!({ "capability": "chat", "name": name }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let session: serde_json::Value = resp.json().await.unwrap();
    // A fresh session inherits, and the two new fields are part of its JSON.
    assert_eq!(session["persona_mode"], "inherit");
    assert!(session["persona_id"].is_null());
    session["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn the_session_override_stores_each_mode_and_shows_up_in_the_session_json() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let created = create(&base, "Blunt", "🪓", "be brief").await;
    let persona_id = created["id"].as_str().unwrap().to_string();
    let session_id = new_session(&base, "Override").await;

    let put = |payload: serde_json::Value| {
        let req = client
            .put(format!("{base}/sessions/{session_id}/persona"))
            .json(&payload);
        async move { req.send().await.unwrap() }
    };

    assert_eq!(
        put(serde_json::json!({ "mode": "persona", "persona_id": persona_id }))
            .await
            .status(),
        204
    );
    let sessions: serde_json::Value = reqwest::get(format!("{base}/sessions?capability=chat"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sessions[0]["persona_mode"], "persona");
    assert_eq!(sessions[0]["persona_id"], persona_id);

    assert_eq!(
        put(serde_json::json!({ "mode": "none" })).await.status(),
        204
    );
    let sessions: serde_json::Value = reqwest::get(format!("{base}/sessions?capability=chat"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sessions[0]["persona_mode"], "none");
    assert!(sessions[0]["persona_id"].is_null());

    assert_eq!(
        put(serde_json::json!({ "mode": "inherit" })).await.status(),
        204
    );
}

#[tokio::test]
async fn the_session_override_refuses_unknown_ids_and_a_missing_persona_id() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let session_id = new_session(&base, "Override").await;

    // Unknown session → 404.
    let resp = client
        .put(format!("{base}/sessions/no-such-session/persona"))
        .json(&serde_json::json!({ "mode": "inherit" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Unknown persona → 404.
    let resp = client
        .put(format!("{base}/sessions/{session_id}/persona"))
        .json(&serde_json::json!({ "mode": "persona", "persona_id": "no-such-persona" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Mode `persona` without an id at all is a malformed request → 400.
    let resp = client
        .put(format!("{base}/sessions/{session_id}/persona"))
        .json(&serde_json::json!({ "mode": "persona" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // An unknown mode cannot become the DTO at all.
    let resp = client
        .put(format!("{base}/sessions/{session_id}/persona"))
        .json(&serde_json::json!({ "mode": "sometimes" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
}

/// The one endpoint the Chat tab's chip reads, so the UI never re-implements the
/// resolution rule: the resolved persona plus where it came from.
#[tokio::test]
async fn the_effective_persona_reports_the_resolved_persona_and_its_origin() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let global = create(&base, "Global", "🌍", "global prompt").await;
    let own = create(&base, "Own", "🎯", "own prompt").await;
    let session_id = new_session(&base, "Chat").await;

    let effective = |query: String| {
        let url = format!("{base}/personas/effective{query}");
        async move {
            let resp = reqwest::get(url).await.unwrap();
            assert_eq!(resp.status(), 200);
            resp.json::<serde_json::Value>().await.unwrap()
        }
    };

    // Nothing set anywhere.
    let eff = effective(String::new()).await;
    assert!(eff["persona"].is_null());
    assert_eq!(eff["origin"], "none");

    // Global only — and the ungrouped case (no session_id) sees it too.
    client
        .put(format!("{base}/personas/active"))
        .json(&serde_json::json!({ "id": global["id"] }))
        .send()
        .await
        .unwrap();
    let eff = effective(String::new()).await;
    assert_eq!(eff["origin"], "global");
    assert_eq!(eff["persona"]["id"], global["id"]);
    let eff = effective(format!("?session_id={session_id}")).await;
    assert_eq!(eff["origin"], "global");

    // The session's own persona wins.
    client
        .put(format!("{base}/sessions/{session_id}/persona"))
        .json(&serde_json::json!({ "mode": "persona", "persona_id": own["id"] }))
        .send()
        .await
        .unwrap();
    let eff = effective(format!("?session_id={session_id}")).await;
    assert_eq!(eff["origin"], "session");
    assert_eq!(eff["persona"]["id"], own["id"]);
    assert_eq!(eff["persona"]["system_prompt"], "own prompt");

    // Opting out beats the global persona.
    client
        .put(format!("{base}/sessions/{session_id}/persona"))
        .json(&serde_json::json!({ "mode": "none" }))
        .send()
        .await
        .unwrap();
    let eff = effective(format!("?session_id={session_id}")).await;
    assert!(eff["persona"].is_null());
    assert_eq!(eff["origin"], "none");
    // …only for that session; ungrouped still gets the global one.
    let eff = effective(String::new()).await;
    assert_eq!(eff["origin"], "global");
}

/// Deleting the persona a session points at heals the session rather than
/// leaving it broken — visible through both the session JSON and the chip.
#[tokio::test]
async fn deleting_a_sessions_persona_heals_the_session() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let client = reqwest::Client::new();
    let persona = create(&base, "Doomed", "💀", "be doomed").await;
    let session_id = new_session(&base, "Chat").await;
    client
        .put(format!("{base}/sessions/{session_id}/persona"))
        .json(&serde_json::json!({ "mode": "persona", "persona_id": persona["id"] }))
        .send()
        .await
        .unwrap();

    client
        .delete(format!(
            "{base}/personas/{}",
            persona["id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap();

    let sessions: serde_json::Value = reqwest::get(format!("{base}/sessions?capability=chat"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sessions[0]["persona_mode"], "inherit");
    assert!(sessions[0]["persona_id"].is_null());

    let eff: serde_json::Value =
        reqwest::get(format!("{base}/personas/effective?session_id={session_id}"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert!(eff["persona"].is_null());
    assert_eq!(eff["origin"], "none");
}

/// A persona's prompt is passed through verbatim — no filtering, no escaping
/// surprises, other scripts and newlines intact.
#[tokio::test]
async fn the_system_prompt_is_stored_verbatim() {
    let (server, _tmp, _app) = fixture().await;
    let base = format!("http://{}", server.addr);
    let prompt = "Du bist schroff.\n\n- Keine Floskeln\n- 直接回答\n\"quoted\" & <tagged>";

    let created = create(&base, "Schroff", "🪓", prompt).await;
    assert_eq!(created["system_prompt"], prompt);

    let list: serde_json::Value = reqwest::get(format!("{base}/personas"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["system_prompt"], prompt);
}
