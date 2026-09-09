//! Read OpenCode's `GET /event` SSE stream and translate it to [`AgentEvent`]s
//! for one session. Runs as a spawned task; ends when the stream closes, errors,
//! or the receiver on the other end of `tx` is dropped.

use std::collections::HashMap;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::{AgentEvent, ToolStatus};

pub(super) async fn run(
    http: reqwest::Client,
    base: String,
    session_id: String,
    tx: UnboundedSender<AgentEvent>,
) {
    let mut resp = match http.get(format!("{base}/event")).send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(AgentEvent::Error {
                message: format!("could not open the event stream: {e}"),
                terminal: true,
            });
            return;
        }
    };

    let mut buf: Vec<u8> = Vec::new();
    let mut roles: HashMap<String, String> = HashMap::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                buf.extend_from_slice(&chunk);
                while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=nl).collect();
                    let Some(payload) = sse_data(&line) else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_slice::<Value>(payload) else {
                        continue;
                    };
                    for ev in map_event(&session_id, &value, &mut roles) {
                        if tx.send(ev).is_err() {
                            return; // capability stopped draining
                        }
                    }
                }
            }
            Ok(None) => return, // stream ended
            Err(_) => return,
        }
    }
}

/// The JSON bytes after a `data: ` SSE prefix, trimmed of the trailing newline.
fn sse_data(line: &[u8]) -> Option<&[u8]> {
    let rest = line.strip_prefix(b"data:")?;
    Some(rest.trim_ascii())
}

/// Map one OpenCode event to zero-or-more [`AgentEvent`]s. `roles` caches
/// `messageID → role` so text/tool parts of a *user* message are not echoed
/// back as agent output.
fn map_event(session_id: &str, v: &Value, roles: &mut HashMap<String, String>) -> Vec<AgentEvent> {
    let props = v.get("properties").unwrap_or(&Value::Null);
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");

    // Ignore events for other sessions.
    let event_session = props
        .get("sessionID")
        .or_else(|| props.pointer("/part/sessionID"))
        .or_else(|| props.pointer("/info/sessionID"))
        .and_then(Value::as_str);
    if let Some(sid) = event_session {
        if sid != session_id {
            return vec![];
        }
    }

    match ty {
        "message.updated" => {
            if let (Some(id), Some(role)) = (
                props.pointer("/info/id").and_then(Value::as_str),
                props.pointer("/info/role").and_then(Value::as_str),
            ) {
                roles.insert(id.to_string(), role.to_string());
            }
            vec![]
        }
        "message.part.updated" => {
            let Some(part) = props.get("part") else {
                return vec![];
            };
            let from_assistant = part
                .get("messageID")
                .and_then(Value::as_str)
                .and_then(|m| roles.get(m))
                .map(|r| r == "assistant")
                .unwrap_or(true); // unknown → assume it's the agent's
            if !from_assistant {
                return vec![];
            }
            match part.get("type").and_then(Value::as_str) {
                Some("text") => part
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .map(|t| {
                        vec![AgentEvent::Text {
                            text: t.to_string(),
                        }]
                    })
                    .unwrap_or_default(),
                Some("tool") => {
                    let state = part.get("state").unwrap_or(&Value::Null);
                    vec![AgentEvent::Tool {
                        id: part
                            .get("callID")
                            .or_else(|| part.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: part
                            .get("tool")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string(),
                        status: tool_status(state.get("status").and_then(Value::as_str)),
                        input: state.get("input").cloned().unwrap_or(Value::Null),
                        output: state
                            .get("output")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    }]
                }
                _ => vec![],
            }
        }
        "permission.asked" | "permission.updated" => {
            let Some(id) = props.get("id").and_then(Value::as_str) else {
                return vec![];
            };
            let summary = props
                .pointer("/metadata/command")
                .and_then(Value::as_str)
                .or_else(|| props.pointer("/patterns/0").and_then(Value::as_str))
                .unwrap_or("(unspecified)")
                .to_string();
            vec![AgentEvent::Permission {
                id: id.to_string(),
                kind: props
                    .get("permission")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string(),
                summary,
                always_pattern: props
                    .pointer("/always/0")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }]
        }
        "session.idle" => vec![AgentEvent::Idle],
        "session.error" => vec![AgentEvent::Error {
            message: props
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| props.pointer("/error/message").and_then(Value::as_str))
                .unwrap_or("the session hit an error")
                .to_string(),
            terminal: false,
        }],
        _ => vec![],
    }
}

fn tool_status(s: Option<&str>) -> ToolStatus {
    match s {
        Some("pending") => ToolStatus::Pending,
        Some("running") => ToolStatus::Running,
        Some("completed") | Some("done") => ToolStatus::Done,
        Some("error") => ToolStatus::Error,
        _ => ToolStatus::Running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(v: Value, roles: &mut HashMap<String, String>) -> Vec<AgentEvent> {
        map_event("ses_1", &v, roles)
    }

    #[test]
    fn user_text_is_not_echoed_but_assistant_text_is() {
        let mut roles = HashMap::new();
        ev(
            json!({ "type": "message.updated", "properties": { "info": { "id": "msg_u", "role": "user", "sessionID": "ses_1" } } }),
            &mut roles,
        );
        let out = ev(
            json!({ "type": "message.part.updated", "properties": { "part": {
                "messageID": "msg_u", "sessionID": "ses_1", "type": "text", "text": "run the tests"
            }}}),
            &mut roles,
        );
        assert!(out.is_empty(), "the user's own prompt must not come back");

        ev(
            json!({ "type": "message.updated", "properties": { "info": { "id": "msg_a", "role": "assistant", "sessionID": "ses_1" } } }),
            &mut roles,
        );
        let out = ev(
            json!({ "type": "message.part.updated", "properties": { "part": {
                "messageID": "msg_a", "sessionID": "ses_1", "type": "text", "text": "Running them now."
            }}}),
            &mut roles,
        );
        assert_eq!(
            out,
            vec![AgentEvent::Text {
                text: "Running them now.".into()
            }]
        );
    }

    #[test]
    fn tool_and_permission_and_idle_map_through() {
        let mut roles = HashMap::new();
        let tool = ev(
            json!({ "type": "message.part.updated", "properties": { "part": {
                "messageID": "msg_a", "sessionID": "ses_1", "type": "tool", "tool": "bash", "callID": "call_1",
                "state": { "status": "running", "input": { "command": "cargo test" } }
            }}}),
            &mut roles,
        );
        assert_eq!(
            tool,
            vec![AgentEvent::Tool {
                id: "call_1".into(),
                name: "bash".into(),
                status: ToolStatus::Running,
                input: json!({ "command": "cargo test" }),
                output: None,
            }]
        );

        let perm = ev(
            json!({ "type": "permission.asked", "properties": {
                "id": "per_1", "sessionID": "ses_1", "permission": "bash",
                "patterns": ["cargo test"], "metadata": { "command": "cargo test" }, "always": ["cargo *"]
            }}),
            &mut roles,
        );
        assert_eq!(
            perm,
            vec![AgentEvent::Permission {
                id: "per_1".into(),
                kind: "bash".into(),
                summary: "cargo test".into(),
                always_pattern: Some("cargo *".into()),
            }]
        );

        assert_eq!(
            ev(
                json!({ "type": "session.idle", "properties": { "sessionID": "ses_1" } }),
                &mut roles
            ),
            vec![AgentEvent::Idle]
        );
    }

    #[test]
    fn other_sessions_events_are_dropped() {
        let mut roles = HashMap::new();
        let out = ev(
            json!({ "type": "session.idle", "properties": { "sessionID": "ses_other" } }),
            &mut roles,
        );
        assert!(out.is_empty());
    }
}
