//! Read one Hermes turn's `POST /api/sessions/{id}/chat/stream` SSE response and
//! translate it to [`AgentEvent`]s. Runs as a spawned task per turn; ends when
//! the stream closes, errors, or the session receiver is dropped.
//!
//! Hermes uses standard SSE framing (`event:` + `data:` lines, blank-line
//! terminated). The event names are only partly documented, so [`map_event`]
//! accepts several plausible spellings for each [`AgentEvent`] variant.

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::{AgentEvent, ToolStatus};

type RunSlot = Arc<Mutex<Option<String>>>;

pub(super) async fn run(
    mut resp: reqwest::Response,
    tx: UnboundedSender<AgentEvent>,
    run: RunSlot,
) {
    let mut buf: Vec<u8> = Vec::new();
    let mut name = String::new();
    let mut data = String::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                buf.extend_from_slice(&chunk);
                while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
                    let raw: Vec<u8> = buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\r', '\n']);
                    if line.is_empty() {
                        if !dispatch(&name, &data, &run, &tx) {
                            return; // receiver gone
                        }
                        name.clear();
                        data.clear();
                    } else if let Some(v) = line.strip_prefix("event:") {
                        name = v.trim().to_string();
                    } else if let Some(v) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(v.strip_prefix(' ').unwrap_or(v));
                    }
                }
            }
            Ok(None) => {
                let _ = tx.send(AgentEvent::Idle); // safety net if the turn ends quietly
                return;
            }
            Err(e) => {
                let _ = tx.send(AgentEvent::Error {
                    message: format!("the turn stream dropped: {e}"),
                    terminal: false,
                });
                return;
            }
        }
    }
}

/// `false` = the receiver is gone, stop.
fn dispatch(name: &str, data: &str, run: &RunSlot, tx: &UnboundedSender<AgentEvent>) -> bool {
    if name.is_empty() && data.is_empty() {
        return true;
    }
    let v: Value = serde_json::from_str(data).unwrap_or(Value::Null);
    // Some servers put the type in the JSON rather than the SSE `event:` line.
    let ty = if name.is_empty() {
        v.get("type").and_then(Value::as_str).unwrap_or("")
    } else {
        name
    };
    for ev in map_event(ty, &v, run) {
        if tx.send(ev).is_err() {
            return false;
        }
    }
    true
}

fn set_run(run: &RunSlot, v: &Value) {
    if let Some(id) = v
        .get("run_id")
        .or_else(|| v.get("run"))
        .or_else(|| v.get("id"))
        .and_then(Value::as_str)
    {
        *run.lock().unwrap_or_else(PoisonError::into_inner) = Some(id.to_string());
    }
}

/// Map one Hermes SSE event to zero-or-more [`AgentEvent`]s.
fn map_event(ty: &str, v: &Value, run: &RunSlot) -> Vec<AgentEvent> {
    match ty {
        "run.started" | "run.created" | "run.submitted" => {
            set_run(run, v);
            vec![]
        }
        "assistant.delta" | "message.delta" | "response.output_text.delta" => v
            .get("delta")
            .or_else(|| v.get("text"))
            .or_else(|| v.pointer("/delta/text"))
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(|t| {
                vec![AgentEvent::Text {
                    text: t.to_string(),
                }]
            })
            .unwrap_or_default(),
        "tool.start" | "tool.started" => vec![tool(v, ToolStatus::Running)],
        "tool.progress" => vec![tool(v, ToolStatus::Running)],
        "tool.complete" | "tool.completed" => {
            let failed = v
                .get("error")
                .map(|e| !e.is_null())
                .or_else(|| v.get("ok").and_then(Value::as_bool).map(|ok| !ok))
                .unwrap_or(false);
            vec![tool(
                v,
                if failed {
                    ToolStatus::Error
                } else {
                    ToolStatus::Done
                },
            )]
        }
        "approval.request" | "sudo.request" | "clarify.request" | "run.approval_required" => {
            set_run(run, v);
            let id = v
                .get("run_id")
                .or_else(|| v.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| run.lock().unwrap_or_else(PoisonError::into_inner).clone())
                .unwrap_or_default();
            let summary = v
                .get("command")
                .or_else(|| v.pointer("/tool/command"))
                .or_else(|| v.get("prompt"))
                .or_else(|| v.get("summary"))
                .and_then(Value::as_str)
                .unwrap_or("(unspecified)")
                .to_string();
            vec![AgentEvent::Permission {
                id,
                kind: v
                    .get("tool")
                    .and_then(|t| t.as_str().or_else(|| t.pointer("/name")?.as_str()))
                    .unwrap_or("terminal")
                    .to_string(),
                summary,
                always_pattern: None,
            }]
        }
        "run.completed" | "run.complete" | "message.complete" | "session.idle"
        | "turn.complete" => {
            *run.lock().unwrap_or_else(PoisonError::into_inner) = None;
            vec![AgentEvent::Idle]
        }
        "run.failed" | "run.cancelled" | "error" => vec![AgentEvent::Error {
            message: v
                .get("message")
                .or_else(|| v.pointer("/error/message"))
                .or_else(|| v.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("the turn failed")
                .to_string(),
            terminal: false,
        }],
        _ => vec![],
    }
}

fn tool(v: &Value, status: ToolStatus) -> AgentEvent {
    AgentEvent::Tool {
        id: v
            .get("call_id")
            .or_else(|| v.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name: v
            .get("tool_name")
            .or_else(|| v.get("tool"))
            .or_else(|| v.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string(),
        status,
        input: v
            .get("input")
            .or_else(|| v.get("arguments"))
            .cloned()
            .unwrap_or_else(|| {
                v.get("command")
                    .and_then(Value::as_str)
                    .map(|c| serde_json::json!({ "command": c }))
                    .unwrap_or(Value::Null)
            }),
        output: v
            .get("output")
            .or_else(|| v.get("result"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run_slot() -> RunSlot {
        Arc::new(Mutex::new(None))
    }

    fn ev(ty: &str, v: Value, run: &RunSlot) -> Vec<AgentEvent> {
        map_event(ty, &v, run)
    }

    #[test]
    fn deltas_become_text_and_run_id_is_captured() {
        let run = run_slot();
        assert!(ev("run.started", json!({ "run_id": "run_9" }), &run).is_empty());
        assert_eq!(*run.lock().unwrap(), Some("run_9".to_string()));

        assert_eq!(
            ev("assistant.delta", json!({ "delta": "on it" }), &run),
            vec![AgentEvent::Text {
                text: "on it".into()
            }]
        );
    }

    #[test]
    fn tool_and_approval_and_idle_map_through() {
        let run = run_slot();
        let t = ev(
            "tool.start",
            json!({ "call_id": "c1", "tool_name": "terminal", "command": "ls" }),
            &run,
        );
        assert_eq!(
            t,
            vec![AgentEvent::Tool {
                id: "c1".into(),
                name: "terminal".into(),
                status: ToolStatus::Running,
                input: json!({ "command": "ls" }),
                output: None,
            }]
        );

        let p = ev(
            "approval.request",
            json!({ "run_id": "run_9", "tool": "terminal", "command": "rm -rf build" }),
            &run,
        );
        assert_eq!(
            p,
            vec![AgentEvent::Permission {
                id: "run_9".into(),
                kind: "terminal".into(),
                summary: "rm -rf build".into(),
                always_pattern: None,
            }]
        );

        assert_eq!(ev("run.completed", json!({}), &run), vec![AgentEvent::Idle]);
        assert_eq!(*run.lock().unwrap(), None);
    }

    #[test]
    fn tool_completion_with_an_error_flags_it() {
        let run = run_slot();
        let t = ev(
            "tool.complete",
            json!({ "call_id": "c1", "tool_name": "terminal", "error": "exit 1", "output": "boom" }),
            &run,
        );
        assert!(matches!(
            t.as_slice(),
            [AgentEvent::Tool {
                status: ToolStatus::Error,
                ..
            }]
        ));
    }

    #[test]
    fn unknown_events_are_ignored() {
        let run = run_slot();
        assert!(ev("gateway.ready", json!({}), &run).is_empty());
        assert!(ev("secret.expire", json!({}), &run).is_empty());
    }
}
