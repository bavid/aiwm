//! The download manager (Phase 6.4, ADR-023).
//!
//! One queue, one active slot. A download streams to a staging file
//! (`<data>/.downloads/<id>/<filename>`) with **HTTP-Range resume**, is verified
//! against the expected SHA-256, then handed to [`crate::model::import_model`]
//! which moves it into the canonical store. Progress is a polled field on the
//! [`Download`] row (`bytes_done` / `state`), not an event stream.
//!
//! Every enqueue is gated on the global `offline_mode` switch (ADR-009). A
//! transport failure is retried (with resume) up to [`MAX_RETRIES`]; the user
//! can pause / resume / cancel at any time.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{Mutex, Notify};

use crate::db::{Database, Download, DownloadState, NewDownload};
use crate::model::{import_model, ImportRequest};
use crate::runtime::download::hex;
use crate::{CoreError, Result};

/// Transport retries (each resumes from the partial file) before a download is
/// marked failed.
const MAX_RETRIES: i64 = 5;
/// How often the worker flushes `bytes_done` to the DB (the UI polls it) and
/// re-reads its row to notice a pause / cancel.
const TICK: Duration = Duration::from_millis(400);
/// Idle poll when the queue is empty and nothing has woken the worker.
const IDLE_POLL: Duration = Duration::from_secs(5);

fn err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Config(format!("download: {msg}"))
}

/// Bytes → a rough `"12.3 GB"` for user-facing messages.
fn gib(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// What a caller supplies to queue a download.
#[derive(Debug, Clone)]
pub struct EnqueueRequest {
    pub url: String,
    pub filename: String,
    /// Passed to `import_model` (`chat` | `checkpoint` | …); `None` → inferred.
    pub model_type: Option<String>,
    /// Expected SHA-256 (from the registry, 6.1). `None` → no hash check.
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
    /// Roles to stamp on the model once imported (e.g. `["chat", "coding"]`
    /// for an agent pick from the Models tab) — empty for a plain download.
    pub roles: Vec<String>,
}

/// The queue + the single-slot worker.
#[derive(Debug)]
pub struct DownloadManager {
    db: Database,
    /// `config.store_path` — where a finished download is imported.
    store_root: PathBuf,
    /// `AppPaths::downloads_dir()`.
    staging_root: PathBuf,
    offline: Arc<AtomicBool>,
    wake: Notify,
    /// Held across "is this file already downloading?" and the insert, so two
    /// concurrent enqueues of one file cannot both see "no" and both insert.
    enqueue_lock: Mutex<()>,
}

/// Why `transfer` stopped short of a finished file.
enum Stopped {
    /// The row is no longer `running` (paused or cancelled) — leave it alone.
    ByRequest,
    /// A transport error; the caller decides whether to retry.
    Transport(CoreError),
}

impl DownloadManager {
    pub fn new(
        db: Database,
        store_root: PathBuf,
        staging_root: PathBuf,
        offline: Arc<AtomicBool>,
    ) -> Self {
        Self {
            db,
            store_root,
            staging_root,
            offline,
            wake: Notify::new(),
            enqueue_lock: Mutex::new(()),
        }
    }

    fn is_offline(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    /// Queue a download. Refused in offline mode, or when the store volume
    /// clearly cannot hold the file (Phase 6.8 — the size only counts once, on
    /// the store volume, since `import_model` moves it there).
    ///
    /// Idempotent per file: while a download with the same SHA-256 is still
    /// active (queued, running, paused or verifying) the file is not queued a
    /// second time — two tabs, or two quick clicks, installing the same
    /// catalog stack must not fetch it twice. The request is merged into that
    /// download instead (see [`Self::merge_into`]) and the caller gets it back
    /// ("already downloading"). The check and the insert run under one lock,
    /// so concurrent enqueues of a file cannot both insert. Without a hash
    /// nothing is merged.
    pub async fn enqueue(&self, req: EnqueueRequest) -> Result<Download> {
        if self.is_offline() {
            return Err(err("offline mode is on — cannot download"));
        }
        let _guard = self.enqueue_lock.lock().await;
        if let Some(active) = self.active_with_sha256(req.sha256.as_deref()).await? {
            return self.merge_into(active, &req).await;
        }
        if let Some(size) = req.size_bytes {
            if let Some((free, _total)) = crate::cleanup::volume_free(&self.store_root) {
                let need = size.saturating_add(crate::cleanup::DOWNLOAD_FREE_MARGIN_BYTES);
                if free < need {
                    return Err(err(format!(
                        "not enough free space on the model store volume: needs ~{} \
                         (+{} margin), {} free",
                        gib(size),
                        gib(crate::cleanup::DOWNLOAD_FREE_MARGIN_BYTES),
                        gib(free),
                    )));
                }
            }
        }
        let d = self
            .db
            .downloads()
            .create(
                NewDownload {
                    url: req.url,
                    filename: req.filename,
                    model_type: req.model_type,
                    sha256: req.sha256,
                    size_bytes: req.size_bytes,
                    roles: req.roles,
                },
                &self.staging_root,
            )
            .await?;
        self.wake.notify_one();
        Ok(d)
    }

    /// A second request for a file that is already downloading:
    /// - a different `model_type` is refused ("already downloading as …") —
    ///   the import files it under one kind only;
    /// - the caller's `roles` are added to the download's (existing order
    ///   kept, no duplicates), so its intent survives the merge;
    /// - a **paused** download is resumed: asking for the file again is asking
    ///   to get it. Queued / running / verifying ones are left as they are.
    async fn merge_into(&self, active: Download, req: &EnqueueRequest) -> Result<Download> {
        fn norm(t: Option<&str>) -> Option<&str> {
            t.map(str::trim).filter(|t| !t.is_empty())
        }
        let (have, want) = (
            norm(active.model_type.as_deref()),
            norm(req.model_type.as_deref()),
        );
        if have != want {
            return Err(err(format!(
                "{} is already downloading as {} — cancel that download to fetch it as {}",
                active.filename,
                have.unwrap_or("an auto-detected type"),
                want.unwrap_or("an auto-detected type"),
            )));
        }

        let extra: Vec<&String> = req
            .roles
            .iter()
            .filter(|r| !active.roles.contains(r))
            .collect();
        if !extra.is_empty() {
            let roles: Vec<String> = active.roles.iter().chain(extra).cloned().collect();
            self.db.downloads().set_roles(&active.id, &roles).await?;
        }
        if active.state == DownloadState::Paused {
            self.db
                .downloads()
                .set_state(&active.id, DownloadState::Queued, None)
                .await?;
            self.wake.notify_one();
        }
        self.db
            .downloads()
            .get(&active.id)
            .await?
            .ok_or_else(|| err("the download vanished while merging"))
    }

    /// The newest not-yet-terminal download of the file with this SHA-256
    /// (compared case-insensitively), if any.
    async fn active_with_sha256(&self, sha256: Option<&str>) -> Result<Option<Download>> {
        let Some(want) = sha256.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        Ok(self.db.downloads().list().await?.into_iter().find(|d| {
            !d.state.is_terminal()
                && d.sha256
                    .as_deref()
                    .is_some_and(|have| have.trim().eq_ignore_ascii_case(want))
        }))
    }

    pub async fn list(&self) -> Result<Vec<Download>> {
        self.db.downloads().list().await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Download>> {
        self.db.downloads().get(id).await
    }

    /// Stop a running / queued download. No-op on a terminal one.
    pub async fn pause(&self, id: &str) -> Result<()> {
        self.transition(id, |s| {
            matches!(s, DownloadState::Queued | DownloadState::Running)
                .then_some(DownloadState::Paused)
        })
        .await
    }

    /// Put a paused / failed download back in the queue.
    pub async fn resume(&self, id: &str) -> Result<()> {
        if self.is_offline() {
            return Err(err("offline mode is on — cannot resume"));
        }
        self.transition(id, |s| {
            matches!(s, DownloadState::Paused | DownloadState::Failed)
                .then_some(DownloadState::Queued)
        })
        .await?;
        self.wake.notify_one();
        Ok(())
    }

    /// Cancel and discard the partial file. Terminal downloads are just removed.
    pub async fn cancel(&self, id: &str) -> Result<()> {
        let Some(d) = self.db.downloads().get(id).await? else {
            return Err(err("no such download"));
        };
        if d.state == DownloadState::Done {
            return Err(err("this download already finished"));
        }
        self.db
            .downloads()
            .set_state(id, DownloadState::Failed, Some("cancelled"))
            .await?;
        // The worker, on its next tick, sees `failed` and bails without a write.
        let _ = tokio::fs::remove_dir_all(self.staging_root.join(id)).await;
        Ok(())
    }

    /// Remove one finished (`done` or `failed`) download from the history.
    /// Refuses a still-active one — cancel it first.
    pub async fn delete(&self, id: &str) -> Result<()> {
        let Some(d) = self.db.downloads().get(id).await? else {
            return Err(err("no such download"));
        };
        if !d.state.is_terminal() {
            return Err(err("this download is still in progress — cancel it first"));
        }
        let _ = tokio::fs::remove_dir_all(self.staging_root.join(id)).await;
        self.db.downloads().delete(id).await?;
        Ok(())
    }

    /// Remove every `done` / `failed` download from the history in one go —
    /// clears the clutter left by imports whose model was since deleted.
    /// Active downloads are untouched. Returns how many were removed.
    pub async fn clear_finished(&self) -> Result<u64> {
        let mut removed = 0u64;
        for d in self.db.downloads().list().await? {
            if !d.state.is_terminal() {
                continue;
            }
            let _ = tokio::fs::remove_dir_all(self.staging_root.join(&d.id)).await;
            if self.db.downloads().delete(&d.id).await? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    async fn transition(
        &self,
        id: &str,
        pick: impl Fn(DownloadState) -> Option<DownloadState>,
    ) -> Result<()> {
        let Some(d) = self.db.downloads().get(id).await? else {
            return Err(err("no such download"));
        };
        match pick(d.state) {
            Some(next) => self.db.downloads().set_state(id, next, None).await,
            None => Ok(()),
        }
    }

    /// The worker loop — spawn exactly one (`api::spawn`).
    pub async fn run(self: Arc<Self>) {
        loop {
            match self.db.downloads().next_actionable().await {
                Ok(Some(d)) => {
                    if let Err(e) = self.process(&d).await {
                        tracing::error!(id = %d.id, error = %e, "download failed");
                        let _ = self
                            .db
                            .downloads()
                            .set_state(&d.id, DownloadState::Failed, Some(&e.to_string()))
                            .await;
                    }
                }
                Ok(None) => {
                    tokio::select! {
                        () = self.wake.notified() => {}
                        () = tokio::time::sleep(IDLE_POLL) => {}
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "download queue read failed");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    /// Transfer → verify → import one download.
    async fn process(&self, d: &Download) -> Result<()> {
        if self.is_offline() {
            self.db
                .downloads()
                .set_state(&d.id, DownloadState::Failed, Some("offline mode is on"))
                .await?;
            return Ok(());
        }
        let dest = PathBuf::from(&d.dest_path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(CoreError::Io)?;
        }
        self.db
            .downloads()
            .set_state(&d.id, DownloadState::Running, None)
            .await?;

        match self.transfer(d, &dest).await {
            Ok(()) => {}
            Err(Stopped::ByRequest) => return Ok(()),
            Err(Stopped::Transport(e)) => {
                let n = self.db.downloads().bump_retries(&d.id).await?;
                if n > MAX_RETRIES {
                    self.db
                        .downloads()
                        .set_state(&d.id, DownloadState::Failed, Some(&format!("gave up: {e}")))
                        .await?;
                } else {
                    tracing::warn!(id = %d.id, retry = n, error = %e, "download retrying");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    self.db
                        .downloads()
                        .set_state(&d.id, DownloadState::Queued, None)
                        .await?;
                }
                return Ok(());
            }
        }

        // --- verify ---
        self.db
            .downloads()
            .set_state(&d.id, DownloadState::Verifying, None)
            .await?;
        if let Some(reason) = verify(&dest, d.sha256.as_deref(), d.size_bytes).await? {
            let _ = tokio::fs::remove_file(&dest).await;
            self.db
                .downloads()
                .set_state(&d.id, DownloadState::Failed, Some(&reason))
                .await?;
            return Ok(());
        }

        // --- import ---
        let outcome = import_model(
            &self.db,
            &self.store_root,
            ImportRequest {
                source_path: dest,
                roles: d.roles.clone(),
                keep_original: false,
                model_type: d.model_type.clone(),
            },
        )
        .await?;
        self.finish_import(&d.id, &outcome.model.id).await?;
        let _ = tokio::fs::remove_dir_all(self.staging_root.join(&d.id)).await;
        tracing::info!(id = %d.id, model = %outcome.model.id, "download imported");
        Ok(())
    }

    /// Mark an imported download done and hand the imported model every role
    /// the download carries *now*. The import used the row as the worker read
    /// it before the transfer; an enqueue merged meanwhile (see
    /// [`Self::merge_into`]) may have added roles since. Runs under
    /// `enqueue_lock`, so no merge can slip in between re-reading the roles
    /// and the row turning `Done` (after which a new enqueue queues afresh).
    /// Database work only — no file I/O while holding the lock.
    async fn finish_import(&self, download_id: &str, model_id: &str) -> Result<()> {
        let _guard = self.enqueue_lock.lock().await;
        let downloads = self.db.downloads();
        downloads.set_model_id(download_id, model_id).await?;
        downloads
            .set_state(download_id, DownloadState::Done, None)
            .await?;
        let wanted = downloads
            .get(download_id)
            .await?
            .map(|d| d.roles)
            .unwrap_or_default();
        let have = self.db.models().roles(model_id).await?;
        let missing: Vec<&String> = wanted.iter().filter(|r| !have.contains(r)).collect();
        if !missing.is_empty() {
            let all: Vec<String> = have.iter().chain(missing).cloned().collect();
            self.db.models().set_roles(model_id, &all).await?;
        }
        Ok(())
    }

    /// Stream the URL into `dest`, resuming from whatever is already there.
    async fn transfer(&self, d: &Download, dest: &Path) -> std::result::Result<(), Stopped> {
        let offset = tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let client = reqwest::Client::builder()
            .user_agent(concat!("aiwm/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Stopped::Transport(err(e)))?;
        let mut req = client.get(&d.url);
        if offset > 0 {
            req = req.header("Range", format!("bytes={offset}-"));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Stopped::Transport(err(format!("GET {}: {e}", d.url))))?;
        let status = resp.status().as_u16();

        // 416 = the partial file is already >= the server's size → treat as done.
        if status == 416 {
            return Ok(());
        }
        if !(200..300).contains(&status) {
            return Err(Stopped::Transport(err(format!(
                "GET {} returned {status}",
                d.url
            ))));
        }
        // 206 → append after `offset`; anything else → the server sent the whole
        // file, restart from scratch.
        let resuming = status == 206 && offset > 0;
        let mut written = if resuming { offset } else { 0 };
        let total = content_total(&resp, written);

        let mut opts = tokio::fs::OpenOptions::new();
        opts.create(true).write(true);
        if resuming {
            opts.append(true);
        } else {
            opts.truncate(true);
        }
        let mut file = opts
            .open(dest)
            .await
            .map_err(|e| Stopped::Transport(err(format!("open {}: {e}", dest.display()))))?;

        self.db
            .downloads()
            .set_progress(&d.id, written, total)
            .await
            .map_err(Stopped::Transport)?;

        let mut resp = resp;
        let mut last_tick = Instant::now();
        loop {
            let chunk = match resp.chunk().await {
                Ok(Some(c)) => c,
                // A clean end short of the expected size is a truncated
                // response — retry (with resume), don't verify a half file.
                Ok(None) => {
                    let _ = file.flush().await;
                    if total.is_some_and(|t| written < t) {
                        return Err(Stopped::Transport(err(format!(
                            "stream ended at {written} of {} bytes",
                            total.unwrap_or(0)
                        ))));
                    }
                    break;
                }
                Err(e) => {
                    let _ = file.flush().await;
                    return Err(Stopped::Transport(err(format!("stream: {e}"))));
                }
            };
            file.write_all(&chunk)
                .await
                .map_err(|e| Stopped::Transport(err(format!("write: {e}"))))?;
            written += chunk.len() as u64;

            if last_tick.elapsed() >= TICK {
                last_tick = Instant::now();
                let _ = file.flush().await;
                let _ = self
                    .db
                    .downloads()
                    .set_progress(&d.id, written, total)
                    .await;
                if !self.still_running(&d.id).await {
                    return Err(Stopped::ByRequest);
                }
            }
        }
        file.flush().await.ok();
        let _ = self
            .db
            .downloads()
            .set_progress(&d.id, written, total)
            .await;
        Ok(())
    }

    async fn still_running(&self, id: &str) -> bool {
        matches!(
            self.db.downloads().get(id).await,
            Ok(Some(d)) if d.state == DownloadState::Running
        )
    }
}

/// Total file size from `Content-Range` (`bytes a-b/total`) or `Content-Length`
/// (+ the bytes already on disk).
fn content_total(resp: &reqwest::Response, already: u64) -> Option<u64> {
    if let Some(cr) = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(total) = cr
            .rsplit('/')
            .next()
            .and_then(|t| t.trim().parse::<u64>().ok())
        {
            return Some(total);
        }
    }
    resp.content_length().map(|len| len + already)
}

/// `None` when the file passes; `Some(reason)` otherwise.
async fn verify(
    dest: &Path,
    expected_sha: Option<&str>,
    expected_size: Option<i64>,
) -> Result<Option<String>> {
    let dest = dest.to_path_buf();
    let expected_sha = expected_sha.map(str::to_lowercase);
    tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        let mut file = std::fs::File::open(&dest).map_err(CoreError::Io)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        let mut size = 0u64;
        loop {
            let n = std::io::Read::read(&mut file, &mut buf).map_err(CoreError::Io)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if expected_sha.is_some() {
                hasher.update(&buf[..n]);
            }
        }
        if let Some(want) = expected_size {
            if want >= 0 && size != want as u64 {
                return Ok(Some(format!("expected {want} bytes, got {size}")));
            }
        }
        if let Some(want) = expected_sha {
            let got = hex(&hasher.finalize());
            if got != want {
                return Ok(Some(format!(
                    "SHA-256 mismatch (expected {want}, got {got})"
                )));
            }
        }
        Ok(None)
    })
    .await
    .map_err(|e| err(format!("verify worker panicked: {e}")))?
}
