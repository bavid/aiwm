//! The job lifecycle state machine.
//!
//! ```text
//! queued ─► scheduled ─► preparing ─► running ─► post ─► completed
//!               │  ▲          │           │        │
//!               ▼  │          ▼           ▼        ▼
//!            blocked          └────────► failed / cancelled
//! ```
//!
//! Transitions are an explicit table; anything not listed is a
//! [`CoreError::InvalidJobTransition`].

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Scheduled,
    Blocked,
    Preparing,
    Running,
    Post,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Scheduled => "scheduled",
            Self::Blocked => "blocked",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Post => "post",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Terminal states never transition again.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// States from which the engine may pick a job up for (re)processing.
    pub fn is_runnable(self) -> bool {
        matches!(self, Self::Queued | Self::Blocked)
    }

    /// Whether `self ─► next` is a legal transition.
    pub fn can_transition_to(self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Queued, Scheduled)
                | (Queued, Cancelled)
                | (Queued, Failed)
                | (Scheduled, Preparing)
                | (Scheduled, Blocked)
                | (Scheduled, Cancelled)
                | (Scheduled, Failed)
                | (Blocked, Scheduled)
                | (Blocked, Cancelled)
                | (Blocked, Failed)
                | (Preparing, Running)
                | (Preparing, Blocked)
                | (Preparing, Cancelled)
                | (Preparing, Failed)
                | (Running, Post)
                | (Running, Cancelled)
                | (Running, Failed)
                | (Post, Completed)
                | (Post, Failed)
        )
    }

    /// Validate a transition, returning an error naming both states.
    pub fn ensure_transition(self, next: JobState) -> Result<()> {
        if self.can_transition_to(next) {
            Ok(())
        } else {
            Err(CoreError::InvalidJobTransition {
                from: self.as_str().to_string(),
                to: next.as_str().to_string(),
            })
        }
    }
}

impl FromStr for JobState {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "queued" => Self::Queued,
            "scheduled" => Self::Scheduled,
            "blocked" => Self::Blocked,
            "preparing" => Self::Preparing,
            "running" => Self::Running,
            "post" => Self::Post,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            other => {
                return Err(CoreError::Db(format!("unknown job state {other:?}")));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::JobState::*;
    use super::*;

    #[test]
    fn str_roundtrip_for_every_state() {
        for st in [
            Queued, Scheduled, Blocked, Preparing, Running, Post, Completed, Failed, Cancelled,
        ] {
            assert_eq!(JobState::from_str(st.as_str()).unwrap(), st);
        }
    }

    #[test]
    fn happy_path_transitions_are_allowed() {
        let path = [Queued, Scheduled, Preparing, Running, Post, Completed];
        for pair in path.windows(2) {
            pair[0].ensure_transition(pair[1]).unwrap();
        }
    }

    #[test]
    fn blocked_can_be_resumed_or_cancelled() {
        assert!(Scheduled.can_transition_to(Blocked));
        assert!(Blocked.can_transition_to(Scheduled));
        assert!(Blocked.can_transition_to(Cancelled));
        assert!(!Blocked.can_transition_to(Running));
    }

    #[test]
    fn terminal_states_go_nowhere() {
        for term in [Completed, Failed, Cancelled] {
            assert!(term.is_terminal());
            for any in [Queued, Scheduled, Running, Completed] {
                assert!(!term.can_transition_to(any));
            }
        }
    }

    #[test]
    fn cannot_skip_from_queued_to_running() {
        let err = Queued.ensure_transition(Running).unwrap_err();
        assert!(matches!(err, CoreError::InvalidJobTransition { .. }));
    }

    #[test]
    fn running_cannot_go_back_to_preparing() {
        assert!(!Running.can_transition_to(Preparing));
    }
}
