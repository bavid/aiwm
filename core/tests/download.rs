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
        })
        .await
        .unwrap();

    let failed = wait_for(&m, &d.id, DownloadState::Failed).await;
    assert_eq!(failed.state, DownloadState::Failed);
    assert!(failed.error_text.unwrap().contains("SHA-256"));
    assert!(!std::path::Path::new(&d.dest_path).exists());
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
        })
        .await
        .unwrap_err();
    assert!(err.to_string().contains("offline"));
}
