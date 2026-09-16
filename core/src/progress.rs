//! Live per-job render progress.
//!
//! ComfyUI's own `/ws?clientId=...` emits real `progress` / `executing` /
//! `executed` events while a workflow runs — `ComfyUiAdapter::generate_media`
//! listens on that socket (best-effort; a connect failure never fails the
//! render, it just means no live percentage) and publishes readings here.
//! `api::http`'s `GET /ws/jobs/{id}` route subscribes to this hub and forwards
//! them to the UI, so Image/Video can show a real percentage instead of
//! polling `jobDetail` for the last log line.
//!
//! Deliberately in-memory only, not persisted to the `jobs` table: a
//! percentage is only meaningful while a job is actively rendering, and every
//! terminal job state (`completed` / `failed` / `cancelled`) already carries
//! its own real status the DB does track.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tokio::sync::broadcast;

/// Capacity of the live-update channel. Generous relative to how often a
/// single ComfyUI render actually ticks (a handful of `progress` events per
/// second at most) so a slow subscriber doesn't miss readings under normal
/// load; a lagging subscriber just skips ahead rather than blocking anyone.
const CHANNEL_CAPACITY: usize = 256;

/// One reading of a job's render progress, straight from ComfyUI's own
/// websocket events.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JobProgress {
    pub job_id: String,
    /// 0.0..=100.0 — set only when ComfyUI's `progress` event carried both
    /// `value` and a positive `max`.
    pub percent: Option<f64>,
    pub step: Option<u32>,
    pub steps_total: Option<u32>,
    /// The node ComfyUI is currently executing (`executing` event's `node`),
    /// when the workflow reports one.
    pub node: Option<String>,
}

impl JobProgress {
    fn executing(job_id: &str, node: Option<String>) -> Self {
        Self {
            job_id: job_id.to_string(),
            percent: None,
            step: None,
            steps_total: None,
            node,
        }
    }

    fn step(job_id: &str, value: u32, max: u32) -> Self {
        let percent =
            (max > 0).then(|| (f64::from(value) / f64::from(max) * 100.0).clamp(0.0, 100.0));
        Self {
            job_id: job_id.to_string(),
            percent,
            step: Some(value),
            steps_total: Some(max),
            node: None,
        }
    }
}

/// Shared, cheaply-clonable-behind-`Arc` hub: the last reading per job, plus a
/// broadcast channel `api::http`'s `/ws/jobs/{id}` route subscribes to.
#[derive(Debug)]
pub struct ProgressHub {
    latest: Mutex<HashMap<String, JobProgress>>,
    tx: broadcast::Sender<JobProgress>,
}

impl Default for ProgressHub {
    fn default() -> Self {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            latest: Mutex::new(HashMap::new()),
            tx,
        }
    }
}

impl ProgressHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to every future reading (for every job — `api::http` filters
    /// by `job_id` itself).
    pub fn subscribe(&self) -> broadcast::Receiver<JobProgress> {
        self.tx.subscribe()
    }

    /// The last reading for `job_id`, if the render is (or was very recently)
    /// live. `None` once [`clear`](Self::clear) has run for it.
    pub fn get(&self, job_id: &str) -> Option<JobProgress> {
        self.latest_lock().get(job_id).cloned()
    }

    fn latest_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, JobProgress>> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, progress: JobProgress) {
        self.latest_lock()
            .insert(progress.job_id.clone(), progress.clone());
        // No subscribers is the common case (nobody has the tab open, or the
        // UI polls `get` instead) — a send error there is expected, not a bug.
        let _ = self.tx.send(progress);
    }

    /// A workflow started executing a node (ComfyUI's `executing` event).
    pub(crate) fn executing(&self, job_id: &str, node: Option<String>) {
        self.publish(JobProgress::executing(job_id, node));
    }

    /// A `progress` event — current/total step within the executing node.
    pub(crate) fn step(&self, job_id: &str, value: u32, max: u32) {
        self.publish(JobProgress::step(job_id, value, max));
    }

    /// The job is done (success, failure, or cancel) — drop the live reading
    /// so a stale percentage never lingers for a job that isn't rendering
    /// anymore.
    pub fn clear(&self, job_id: &str) {
        self.latest_lock().remove(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_computes_a_clamped_percent() {
        let p = JobProgress::step("j1", 5, 20);
        assert_eq!(p.percent, Some(25.0));
        assert_eq!(p.step, Some(5));
        assert_eq!(p.steps_total, Some(20));

        // A zero max (shouldn't happen, but defend anyway) must not divide by
        // zero or produce NaN/Infinity.
        let zero = JobProgress::step("j1", 0, 0);
        assert_eq!(zero.percent, None);
    }

    #[test]
    fn executing_carries_no_percent() {
        let p = JobProgress::executing("j1", Some("KSampler".into()));
        assert_eq!(p.percent, None);
        assert_eq!(p.node.as_deref(), Some("KSampler"));
    }

    #[test]
    fn get_reflects_the_latest_publish_per_job() {
        let hub = ProgressHub::new();
        assert_eq!(hub.get("j1"), None);

        hub.executing("j1", Some("CheckpointLoader".into()));
        hub.step("j1", 1, 10);
        hub.step("j2", 3, 3);

        assert_eq!(hub.get("j1").unwrap().step, Some(1));
        assert_eq!(hub.get("j2").unwrap().percent, Some(100.0));
    }

    #[test]
    fn clear_drops_the_reading() {
        let hub = ProgressHub::new();
        hub.step("j1", 1, 10);
        assert!(hub.get("j1").is_some());
        hub.clear("j1");
        assert_eq!(hub.get("j1"), None);
    }

    #[tokio::test]
    async fn subscribers_receive_every_publish() {
        let hub = ProgressHub::new();
        let mut rx = hub.subscribe();

        hub.step("j1", 2, 8);
        let got = rx.recv().await.unwrap();
        assert_eq!(got.job_id, "j1");
        assert_eq!(got.percent, Some(25.0));

        hub.executing("j1", Some("VAEDecode".into()));
        let got = rx.recv().await.unwrap();
        assert_eq!(got.node.as_deref(), Some("VAEDecode"));
    }

    #[test]
    fn publish_with_no_subscribers_does_not_panic() {
        let hub = ProgressHub::new();
        hub.step("j1", 1, 2); // no `subscribe()` call — send() sees zero receivers
        assert!(hub.get("j1").is_some());
    }
}
