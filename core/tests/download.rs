//! `DownloadManager` driven against an in-process file server: the full
//! enqueue → transfer → verify → import flow, HTTP-Range resume after a
//! truncated response, and a SHA-256 mismatch.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use sha2::{Digest, Sha256};

use aiwm_core::db::{Database, DownloadState};
use aiwm_core::download::{DownloadManager, EnqueueRequest};

/// A tiny but valid GGUF v3 (no tensors) — enough for `import_model` to accept.
fn gguf_body() -> Vec<u8> {
    let gstr = |b: &mut Vec<u8>, s: &str| {
        b.extend_from_slice(&(s.len() as u64).to_le_bytes());
        b.extend_from_slice(s.as_bytes());
    };
    let mut kv = Vec::new();
    gstr(&mut kv, "general.architecture");
    kv.extend_from_slice(&8u32.to_le_bytes());
    gstr(&mut kv, "llama");
    gstr(&mut kv, "general.name");
    kv.extend_from_slice(&8u32.to_le_bytes());
    gstr(&mut kv, "Downloaded 7B");

    let mut out = Vec::new();
    out.extend_from_slice(&0x4655_4747u32.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes()); // tensor count
    out.extend_from_slice(&2u64.to_le_bytes()); // kv count
    out.extend_from_slice(&kv);
    // Pad so a partial transfer is a meaningful fraction.
    out.resize(40_000, 0);
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let mut s = String::new();
    for b in h.finalize() {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[derive(Clone)]
struct Fx {
    body: Arc<Vec<u8>>,
    saw_range: Arc<AtomicBool>,
    corrupt: Arc<AtomicBool>,
    /// ms of delay between each 1 KB chunk (0 = send it all at once).
    slow_ms: Arc<AtomicU32>,
}

/// A body that trickles `body` out in 1 KB chunks, `slow_ms` apart.
fn slow_body(body: Vec<u8>, slow_ms: u32) -> Body {
    if slow_ms == 0 {
        return Body::from(body);
    }
    let chunks: Vec<Vec<u8>> = body.chunks(1024).map(<[u8]>::to_vec).collect();
    let stream = futures_util::stream::unfold(chunks.into_iter(), move |mut it| async move {
        let next = it.next()?;
        tokio::time::sleep(std::time::Duration::from_millis(slow_ms as u64)).await;
        Some((Ok::<_, std::io::Error>(next), it))
    });
    Body::from_stream(stream)
}

async fn serve(State(fx): State<Fx>, headers: HeaderMap) -> Response {
    let mut body = (*fx.body).clone();
    if fx.corrupt.load(Ordering::SeqCst) {
        body[0] ^= 0xff;
    }
    let total = body.len();
    let slow = fx.slow_ms.load(Ordering::SeqCst);

    if let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        fx.saw_range.store(true, Ordering::SeqCst);
        let start: usize = range
            .trim_start_matches("bytes=")
            .trim_end_matches('-')
            .parse()
            .unwrap_or(0);
        let rest = body[start.min(total)..].to_vec();
        return (
            StatusCode::PARTIAL_CONTENT,
            [
                (
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{}/{total}", total - 1),
                ),
                (header::CONTENT_LENGTH, rest.len().to_string()),
            ],
            slow_body(rest, slow),
        )
            .into_response();
    }
    (
        [(header::CONTENT_LENGTH, total.to_string())],
        slow_body(body, slow),
    )
        .into_response()
}

struct Server {
    base: String,
    saw_range: Arc<AtomicBool>,
    corrupt: Arc<AtomicBool>,
    slow_ms: Arc<AtomicU32>,
    _task: tokio::task::JoinHandle<()>,
}

async fn start_server(body: Vec<u8>) -> Server {
    let fx = Fx {
        body: Arc::new(body),
        saw_range: Arc::new(AtomicBool::new(false)),
        corrupt: Arc::new(AtomicBool::new(false)),
        slow_ms: Arc::new(AtomicU32::new(0)),
    };
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/model.gguf", get(serve))
        .with_state(fx.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Server {
        base: format!("http://{addr}"),
        saw_range: fx.saw_range,
        corrupt: fx.corrupt,
        slow_ms: fx.slow_ms,
        _task: task,
    }
}

/// A manager wired to a temp store + staging. Call `spawn_worker` to start it.
async fn make_manager(tmp: &std::path::Path) -> (Arc<DownloadManager>, Database) {
    let db = Database::connect(&tmp.join("aiwm.db")).await.unwrap();
    let m = Arc::new(DownloadManager::new(
        db.clone(),
        tmp.join("store"),
        tmp.join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    ));
    (m, db)
}

fn spawn_worker(m: &Arc<DownloadManager>) {
    tokio::spawn(m.clone().run());
}

/// Manager + a spawned worker.
async fn manager(tmp: &std::path::Path) -> (Arc<DownloadManager>, Database) {
    let (m, db) = make_manager(tmp).await;
    spawn_worker(&m);
    (m, db)
}

async fn wait_for(m: &DownloadManager, id: &str, want: DownloadState) -> aiwm_core::db::Download {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let d = m.get(id).await.unwrap().unwrap();
        if d.state == want || d.state.is_terminal() {
            return d;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "stuck in {:?}",
            d.state
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

#[tokio::test]
async fn downloads_verifies_and_imports_a_model() {
    let tmp = tempfile::tempdir().unwrap();
    let body = gguf_body();
    let server = start_server(body.clone()).await;
    let (m, db) = manager(tmp.path()).await;

    let d = m
        .enqueue(EnqueueRequest {
            url: format!("{}/model.gguf", server.base),
            filename: "qwen.Q4_K_M.gguf".into(),
            model_type: Some("chat".into()),
            sha256: Some(sha256_hex(&body).to_uppercase()),
            size_bytes: Some(body.len() as u64),
            roles: vec![],
        })
        .await
        .unwrap();

    let done = wait_for(&m, &d.id, DownloadState::Done).await;
    assert_eq!(done.state, DownloadState::Done, "{:?}", done.error_text);
    let model_id = done.model_id.expect("model_id set after import");

    let model = db.models().get(&model_id).await.unwrap().unwrap();
    assert_eq!(model.arch.as_deref(), Some("llama"));
    assert!(std::path::Path::new(&model.file_path).is_file());
    // The staging dir is cleaned up.
    assert!(!tmp.path().join(".downloads").join(&d.id).exists());
}

#[tokio::test]
async fn a_download_with_roles_stamps_them_on_the_imported_model() {
    let tmp = tempfile::tempdir().unwrap();
    let body = gguf_body();
    let server = start_server(body.clone()).await;
    let (m, db) = manager(tmp.path()).await;

    let d = m
        .enqueue(EnqueueRequest {
            url: format!("{}/model.gguf", server.base),
            filename: "coder.Q5_K_M.gguf".into(),
            model_type: Some("chat".into()),
            sha256: Some(sha256_hex(&body)),
            size_bytes: Some(body.len() as u64),
            roles: vec!["chat".into(), "coding".into()],
        })
        .await
        .unwrap();

    let done = wait_for(&m, &d.id, DownloadState::Done).await;
    assert_eq!(done.state, DownloadState::Done, "{:?}", done.error_text);
    let model_id = done.model_id.expect("model_id set after import");

    let model = db.models().get(&model_id).await.unwrap().unwrap();
    assert_eq!(model.roles, vec!["chat".to_string(), "coding".to_string()]);
}

#[tokio::test]
async fn a_partial_file_resumes_with_a_range_request() {
    let tmp = tempfile::tempdir().unwrap();
    let body = gguf_body();
    let server = start_server(body.clone()).await;
    let (m, _db) = make_manager(tmp.path()).await; // worker not started yet

    let d = m
        .enqueue(EnqueueRequest {
            url: format!("{}/model.gguf", server.base),
            filename: "m.gguf".into(),
            model_type: Some("chat".into()),
            sha256: Some(sha256_hex(&body)),
            size_bytes: Some(body.len() as u64),
            roles: vec![],
        })
        .await
        .unwrap();

    // Pretend a previous run got half the file, then died mid-transfer.
    let half = body.len() / 2;
    std::fs::create_dir_all(std::path::Path::new(&d.dest_path).parent().unwrap()).unwrap();
    std::fs::write(&d.dest_path, &body[..half]).unwrap();

    spawn_worker(&m);
    let done = wait_for(&m, &d.id, DownloadState::Done).await;
    assert_eq!(done.state, DownloadState::Done, "{:?}", done.error_text);
    assert!(
        server.saw_range.load(Ordering::SeqCst),
        "resume must send a Range header"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_stops_a_running_transfer_and_keeps_the_partial() {
    let tmp = tempfile::tempdir().unwrap();
    let body = gguf_body();
    let server = start_server(body.clone()).await;
    server.slow_ms.store(30, Ordering::SeqCst);
    let (m, _db) = manager(tmp.path()).await;

    let d = m
        .enqueue(EnqueueRequest {
            url: format!("{}/model.gguf", server.base),
            filename: "m.gguf".into(),
            model_type: Some("chat".into()),
            sha256: Some(sha256_hex(&body)),
            size_bytes: Some(body.len() as u64),
            roles: vec![],
        })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while m.get(&d.id).await.unwrap().unwrap().state != DownloadState::Running {
        assert!(tokio::time::Instant::now() < deadline, "never started");
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    m.pause(&d.id).await.unwrap();

    let paused = wait_for(&m, &d.id, DownloadState::Paused).await;
    assert_eq!(paused.state, DownloadState::Paused);
    let partial = std::fs::metadata(&d.dest_path).unwrap().len();
    assert!(
        partial > 0 && partial < body.len() as u64,
        "partial = {partial}"
    );

    server.slow_ms.store(0, Ordering::SeqCst);
    m.resume(&d.id).await.unwrap();
    let done = wait_for(&m, &d.id, DownloadState::Done).await;
    assert_eq!(done.state, DownloadState::Done, "{:?}", done.error_text);
}

#[tokio::test]
async fn a_sha_mismatch_fails_the_download_and_removes_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let body = gguf_body();
    let server = start_server(body.clone()).await;
    server.corrupt.store(true, Ordering::SeqCst);
    let (m, _db) = manager(tmp.path()).await;

    let d = m
        .enqueue(EnqueueRequest {
            url: format!("{}/model.gguf", server.base),
            filename: "m.gguf".into(),
            model_type: Some("chat".into()),
            sha256: Some(sha256_hex(&body)), // the *uncorrupted* hash
            size_bytes: Some(body.len() as u64),
            roles: vec![],
        })
        .await
        .unwrap();

    let failed = wait_for(&m, &d.id, DownloadState::Failed).await;
    assert_eq!(failed.state, DownloadState::Failed);
    assert!(failed.error_text.unwrap().contains("SHA-256"));
    assert!(!std::path::Path::new(&d.dest_path).exists());
}

/// `POST /downloads` + `GET /downloads` over the real loopback HTTP server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_endpoints_over_http() {
    use std::net::SocketAddr;
    let tmp = tempfile::tempdir().unwrap();
    let body = gguf_body();
    let server = start_server(body.clone()).await;

    let app = std::sync::Arc::new(
        aiwm_core::App::load(aiwm_core::AppPaths::rooted(tmp.path()))
            .await
            .unwrap(),
    );
    let api = aiwm_core::ApiServer::bind(app.clone(), SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    tokio::spawn(app.downloads.clone().run());
    let base = format!("http://{}", api.addr);

    let created: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/downloads"))
        .json(&serde_json::json!({
            "url": format!("{}/model.gguf", server.base),
            "filename": "sub/dir/qwen.Q4_K_M.gguf",
            "model_type": "chat",
            "sha256": sha256_hex(&body),
            "size_bytes": body.len(),
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["state"], "queued");
    assert_eq!(
        created["filename"], "qwen.Q4_K_M.gguf",
        "only the basename is kept"
    );
    let id = created["id"].as_str().unwrap().to_string();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let list: serde_json::Value = reqwest::get(format!("{base}/downloads"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let d = list
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .unwrap();
        if d["state"] == "done" {
            assert!(d["model_id"].is_string());
            break;
        }
        assert_ne!(d["state"], "failed", "{:?}", d["error_text"]);
        assert!(
            tokio::time::Instant::now() < deadline,
            "stuck: {}",
            d["state"]
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn enqueue_is_refused_in_offline_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = DownloadManager::new(
        db,
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(true)),
    );
    let err = m
        .enqueue(EnqueueRequest {
            url: "http://x/y".into(),
            filename: "y".into(),
            model_type: None,
            sha256: None,
            size_bytes: None,
            roles: vec![],
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("offline"));
}

fn req(name: &str) -> EnqueueRequest {
    EnqueueRequest {
        url: format!("http://x/{name}"),
        filename: name.into(),
        model_type: None,
        sha256: None,
        size_bytes: None,
        roles: vec![],
    }
}

#[tokio::test]
async fn delete_removes_a_finished_download() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = DownloadManager::new(
        db.clone(),
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    );
    let d = m.enqueue(req("a.gguf")).await.unwrap();
    db.downloads()
        .set_state(&d.id, DownloadState::Done, None)
        .await
        .unwrap();

    m.delete(&d.id).await.unwrap();

    assert!(m.get(&d.id).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_removes_a_failed_download() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = DownloadManager::new(
        db.clone(),
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    );
    let d = m.enqueue(req("a.gguf")).await.unwrap();
    db.downloads()
        .set_state(&d.id, DownloadState::Failed, Some("boom"))
        .await
        .unwrap();

    m.delete(&d.id).await.unwrap();

    assert!(m.get(&d.id).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_refuses_a_download_still_in_progress() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = DownloadManager::new(
        db,
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    );
    let d = m.enqueue(req("a.gguf")).await.unwrap();

    let err = m.delete(&d.id).await.unwrap_err();

    assert!(
        err.to_string().to_lowercase().contains("progress")
            || err.to_string().to_lowercase().contains("cancel")
    );
    assert!(
        m.get(&d.id).await.unwrap().is_some(),
        "still there — not deleted"
    );
}

#[tokio::test]
async fn delete_of_an_unknown_id_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = DownloadManager::new(
        db,
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(m.delete("ghost").await.is_err());
}

#[tokio::test]
async fn clear_finished_removes_every_done_and_failed_row_but_keeps_active_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = DownloadManager::new(
        db.clone(),
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    );
    let queued = m.enqueue(req("queued.gguf")).await.unwrap();
    let done = m.enqueue(req("done.gguf")).await.unwrap();
    let failed = m.enqueue(req("failed.gguf")).await.unwrap();
    db.downloads()
        .set_state(&done.id, DownloadState::Done, None)
        .await
        .unwrap();
    db.downloads()
        .set_state(&failed.id, DownloadState::Failed, Some("boom"))
        .await
        .unwrap();

    let removed = m.clear_finished().await.unwrap();

    assert_eq!(removed, 2);
    let remaining = m.list().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, queued.id);
}

#[tokio::test]
async fn clear_finished_is_a_noop_when_nothing_is_terminal() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = DownloadManager::new(
        db,
        tmp.path().join("store"),
        tmp.path().join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    );
    m.enqueue(req("queued.gguf")).await.unwrap();

    assert_eq!(m.clear_finished().await.unwrap(), 0);
    assert_eq!(m.list().await.unwrap().len(), 1);
}

fn idle_manager(db: &Database, tmp: &std::path::Path) -> DownloadManager {
    DownloadManager::new(
        db.clone(),
        tmp.join("store"),
        tmp.join(".downloads"),
        Arc::new(AtomicBool::new(false)),
    )
}

fn req_with_sha(name: &str, sha256: &str) -> EnqueueRequest {
    EnqueueRequest {
        sha256: Some(sha256.into()),
        ..req(name)
    }
}

/// Two tabs (or two clicks) installing the same catalog file must not queue
/// it twice: while a download of that SHA-256 is still active (queued,
/// running, paused or verifying), a second enqueue hands back that download.
#[tokio::test]
async fn an_enqueue_of_a_file_already_downloading_returns_the_active_download() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = idle_manager(&db, tmp.path());
    let sha = "ab".repeat(32);

    let first = m.enqueue(req_with_sha("model.onnx", &sha)).await.unwrap();
    for state in [
        DownloadState::Queued,
        DownloadState::Running,
        DownloadState::Paused,
        DownloadState::Verifying,
    ] {
        db.downloads()
            .set_state(&first.id, state, None)
            .await
            .unwrap();
        // Same file, hash in another case -- still the same file.
        let again = m
            .enqueue(req_with_sha("model.onnx", &sha.to_ascii_uppercase()))
            .await
            .unwrap();
        assert_eq!(again.id, first.id, "{state:?}");
        assert_eq!(again.state, state);
    }
    assert_eq!(m.list().await.unwrap().len(), 1, "queued exactly once");
}

/// A finished or failed earlier attempt is history, not an active download:
/// asking again queues a fresh one.
#[tokio::test]
async fn a_finished_or_failed_download_of_the_same_file_does_not_block_a_new_one() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = idle_manager(&db, tmp.path());
    let sha = "cd".repeat(32);

    let done = m.enqueue(req_with_sha("a.onnx", &sha)).await.unwrap();
    db.downloads()
        .set_state(&done.id, DownloadState::Done, None)
        .await
        .unwrap();
    let second = m.enqueue(req_with_sha("a.onnx", &sha)).await.unwrap();
    assert_ne!(second.id, done.id);

    db.downloads()
        .set_state(&second.id, DownloadState::Failed, Some("boom"))
        .await
        .unwrap();
    let third = m.enqueue(req_with_sha("a.onnx", &sha)).await.unwrap();
    assert_ne!(third.id, second.id);
    assert_eq!(m.list().await.unwrap().len(), 3);
}

/// Without a hash there is no way to tell two files apart -- nothing merges.
#[tokio::test]
async fn downloads_without_a_hash_are_never_merged() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Database::connect_in_memory().await.unwrap();
    let m = idle_manager(&db, tmp.path());

    let a = m.enqueue(req("same.gguf")).await.unwrap();
    let b = m.enqueue(req("same.gguf")).await.unwrap();
    assert_ne!(a.id, b.id);
}
