//! Deterministic "is a newer version available upstream" check for the five
//! externally-sourced tools AIWM installs or resolves: ComfyUI, llama.cpp,
//! Colibri, Hermes, and OpenCode.
//!
//! **Not to be confused with** [`crate::api::handlers::upgrade_check`] (Phase
//! 6.7, job type `upgrade_check`) — that is an LLM-reasoning "is there a
//! better MODEL on Hugging Face" advisor for one imported model. This module
//! does something much simpler and fully deterministic: read the version each
//! tool's own install module already has pinned (never a duplicated copy —
//! see the `super::comfyui::install` / `super::llamacpp::install` /
//! `super::colibri::install` / `crate::agent::HERMES_PINNED_VERSION`
//! references below), fetch the latest version actually published upstream —
//! from wherever that tool's installer already sources its pinned archive —
//! and report whether they match.
//!
//! Upstream sources, read from each installer's own pinned archive URL:
//! - **ComfyUI** (`comfyanonymous/ComfyUI`), **llama.cpp** (`ggml-org/llama.cpp`)
//!   and **Colibri** (`JustVugg/colibri`) are all fetched from GitHub —
//!   `GET /repos/<owner>/<repo>/releases/latest`.
//! - **Hermes** (`hermes-agent`) is a **PyPI** package (`uv pip install
//!   hermes-agent==<version>` — see `agent::hermes::install`), not GitHub —
//!   `GET /pypi/hermes-agent/json`.
//! - **OpenCode** has no AIWM-managed pin at all (`AgentRuntimeDto::install`
//!   is `null` — "bring your own binary"; see `docs/DECISIONS.md` ADR-021).
//!   Its upstream repo (`sst/opencode`, referenced in
//!   `launcher::config`'s doc comment) is still checked so the Diagnostics
//!   tab can show the latest available tag, but it is never compared against
//!   a local version AIWM does not track.
//!
//! No semver parsing: none of the five tools uses semver consistently
//! (llama.cpp's tags are sequential build numbers like `b10855`; the others
//! are `vX.Y.Z` or plain `X.Y.Z`), and AIWM's pins only ever move forward when
//! a maintainer curates a new one. So "the upstream version differs from the
//! pinned one" is treated as "an update is available" — a plain string
//! comparison, not an ordering, which is both simpler and correct here.
//!
//! Every call refuses up front in offline mode (ADR-009), exactly like every
//! other networked runtime feature (`comfyui::install::install`,
//! `llamacpp::install::install`, ...). A failure fetching one tool's upstream
//! version never fails the others — each is reported independently.

use serde::Serialize;

use crate::{CoreError, Result};

const GITHUB_API_BASE: &str = "https://api.github.com";
const PYPI_API_BASE: &str = "https://pypi.org";

/// Where a tool's "latest version" is fetched from.
#[derive(Debug, Clone, Copy)]
enum Upstream {
    /// `<github_api_base>/repos/<owner>/<repo>/releases/latest` → `tag_name`.
    GithubReleases {
        owner: &'static str,
        repo: &'static str,
    },
    /// `<pypi_api_base>/pypi/<name>/json` → `info.version`.
    PyPi { name: &'static str },
}

/// One tool's update status — see the module doc for why this is a plain
/// equality check rather than a version ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Upstream's latest version matches AIWM's pin.
    UpToDate,
    /// Upstream has published a version AIWM has not pinned.
    UpdateAvailable { latest: String },
    /// AIWM does not pin a version for this tool (OpenCode only) — the
    /// upstream value is informational.
    Unmanaged { latest: String },
    /// The upstream check itself failed (network error, unexpected response
    /// shape, no release published, ...).
    CheckFailed { error: String },
}

/// One tool's version-check result — `GET /runtimes/versions`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolVersionCheck {
    /// `"comfyui"` | `"llamacpp"` | `"colibri"` | `"hermes"` | `"opencode"` —
    /// matches the ids used by `RuntimeStatusDto` / `AgentRuntimeDto`.
    pub id: &'static str,
    /// AIWM's pinned version, or `None` for the unmanaged OpenCode.
    pub current: Option<&'static str>,
    pub status: UpdateStatus,
}

struct ToolSpec {
    id: &'static str,
    current: Option<&'static str>,
    upstream: Upstream,
}

/// The five tools, each pointed at the exact upstream its own installer
/// already sources its pinned archive from (see the module doc).
fn specs() -> [ToolSpec; 5] {
    [
        ToolSpec {
            id: "comfyui",
            current: Some(super::comfyui::install::PINNED_TAG),
            upstream: Upstream::GithubReleases {
                owner: "comfyanonymous",
                repo: "ComfyUI",
            },
        },
        ToolSpec {
            id: "llamacpp",
            current: Some(super::llamacpp::install::PINNED_BUILD),
            upstream: Upstream::GithubReleases {
                owner: "ggml-org",
                repo: "llama.cpp",
            },
        },
        ToolSpec {
            id: "colibri",
            current: Some(super::colibri::install::PINNED_BUILD),
            upstream: Upstream::GithubReleases {
                owner: "JustVugg",
                repo: "colibri",
            },
        },
        ToolSpec {
            id: "hermes",
            current: Some(crate::agent::HERMES_PINNED_VERSION),
            upstream: Upstream::PyPi {
                name: "hermes-agent",
            },
        },
        ToolSpec {
            id: "opencode",
            // AIWM has no managed OpenCode install to pin a version for
            // ("bring your own binary" — `AgentRuntimeDto::install` is
            // `null`). Still worth surfacing the latest upstream tag.
            current: None,
            upstream: Upstream::GithubReleases {
                owner: "sst",
                repo: "opencode",
            },
        },
    ]
}

/// Check all five tools against their real upstream sources. Refuses in
/// offline mode before making any request (ADR-009).
pub async fn check_versions(offline: bool) -> Result<Vec<ToolVersionCheck>> {
    check_versions_with(offline, GITHUB_API_BASE, PYPI_API_BASE).await
}

/// The offline-gate → fetch → compare pipeline, parameterised on the API base
/// URLs so tests can substitute a local server.
async fn check_versions_with(
    offline: bool,
    github_api_base: &str,
    pypi_api_base: &str,
) -> Result<Vec<ToolVersionCheck>> {
    if offline {
        return Err(CoreError::Config(
            "offline mode is on — cannot check for tool updates".into(),
        ));
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("aiwm/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| CoreError::Config(format!("version check: build http client: {e}")))?;

    let mut out = Vec::with_capacity(5);
    for spec in specs() {
        let status =
            match fetch_latest(&client, github_api_base, pypi_api_base, &spec.upstream).await {
                Ok(latest) => match spec.current {
                    Some(current) if current == latest => UpdateStatus::UpToDate,
                    Some(_) => UpdateStatus::UpdateAvailable { latest },
                    None => UpdateStatus::Unmanaged { latest },
                },
                Err(e) => UpdateStatus::CheckFailed {
                    error: e.to_string(),
                },
            };
        out.push(ToolVersionCheck {
            id: spec.id,
            current: spec.current,
            status,
        });
    }
    Ok(out)
}

#[derive(Debug, serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Debug, serde::Deserialize)]
struct PyPiResponse {
    info: PyPiInfo,
}

#[derive(Debug, serde::Deserialize)]
struct PyPiInfo {
    version: String,
}

async fn fetch_latest(
    client: &reqwest::Client,
    github_api_base: &str,
    pypi_api_base: &str,
    upstream: &Upstream,
) -> Result<String> {
    match upstream {
        Upstream::GithubReleases { owner, repo } => {
            let url = format!("{github_api_base}/repos/{owner}/{repo}/releases/latest");
            let release: GithubRelease = client
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(|e| fetch_err(&url, e))?
                .error_for_status()
                .map_err(|e| fetch_err(&url, e))?
                .json()
                .await
                .map_err(|e| fetch_err(&url, e))?;
            Ok(release.tag_name)
        }
        Upstream::PyPi { name } => {
            let url = format!("{pypi_api_base}/pypi/{name}/json");
            let resp: PyPiResponse = client
                .get(&url)
                .send()
                .await
                .map_err(|e| fetch_err(&url, e))?
                .error_for_status()
                .map_err(|e| fetch_err(&url, e))?
                .json()
                .await
                .map_err(|e| fetch_err(&url, e))?;
            Ok(resp.info.version)
        }
    }
}

fn fetch_err(url: &str, e: reqwest::Error) -> CoreError {
    CoreError::Config(format!("version check: GET {url}: {e}"))
}

#[cfg(test)]
mod tests {
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn refuses_in_offline_mode() {
        // Short-circuits before any request, so the real base URLs are safe
        // to use here even in a sandboxed test run.
        let err = check_versions(true).await.unwrap_err();
        assert!(err.to_string().contains("offline mode"));
    }

    /// One local server standing in for both `api.github.com` and
    /// `pypi.org`, covering every branch of the comparison logic at once:
    /// a match (up to date), a mismatch (update available), a tool AIWM
    /// doesn't pin (unmanaged), and an unreachable one (check failed).
    #[tokio::test]
    async fn checks_every_tool_independently_against_its_real_upstream_source() {
        let app = Router::new()
            .route(
                "/repos/comfyanonymous/ComfyUI/releases/latest",
                get(|| async { Json(json!({ "tag_name": "v0.34.0" })) }),
            )
            .route(
                "/repos/ggml-org/llama.cpp/releases/latest",
                get(|| async { Json(json!({ "tag_name": "b10900" })) }),
            )
            // JustVugg/colibri deliberately has no route — a 404, standing in
            // for "no releases found" or a transient upstream failure.
            .route(
                "/repos/sst/opencode/releases/latest",
                get(|| async { Json(json!({ "tag_name": "v0.5.0" })) }),
            )
            .route(
                "/pypi/hermes-agent/json",
                get(|| async { Json(json!({ "info": { "version": "0.20.0" } })) }),
            );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{port}");

        let results = check_versions_with(false, &base, &base).await.unwrap();
        assert_eq!(results.len(), 5);
        let by_id = |id: &str| results.iter().find(|r| r.id == id).unwrap();

        let comfyui = by_id("comfyui");
        assert_eq!(comfyui.current, Some("v0.34.0"));
        assert_eq!(comfyui.status, UpdateStatus::UpToDate);

        let llamacpp = by_id("llamacpp");
        assert_eq!(llamacpp.current, Some("b10855"));
        assert_eq!(
            llamacpp.status,
            UpdateStatus::UpdateAvailable {
                latest: "b10900".into()
            }
        );

        let colibri = by_id("colibri");
        assert_eq!(colibri.current, Some("v1.10.2"));
        assert!(matches!(colibri.status, UpdateStatus::CheckFailed { .. }));

        let hermes = by_id("hermes");
        assert_eq!(hermes.current, Some("0.19.0"));
        assert_eq!(
            hermes.status,
            UpdateStatus::UpdateAvailable {
                latest: "0.20.0".into()
            }
        );

        let opencode = by_id("opencode");
        assert_eq!(opencode.current, None, "OpenCode has no AIWM-managed pin");
        assert_eq!(
            opencode.status,
            UpdateStatus::Unmanaged {
                latest: "v0.5.0".into()
            }
        );
    }

    #[tokio::test]
    async fn a_malformed_upstream_response_is_a_check_failure_not_a_panic() {
        let app = Router::new().route(
            "/repos/comfyanonymous/ComfyUI/releases/latest",
            // No `tag_name` field — GithubRelease fails to deserialize.
            get(|| async { Json(json!({ "unexpected": "shape" })) }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let base = format!("http://127.0.0.1:{port}");

        let results = check_versions_with(false, &base, &base).await.unwrap();
        let comfyui = results.iter().find(|r| r.id == "comfyui").unwrap();
        assert!(matches!(comfyui.status, UpdateStatus::CheckFailed { .. }));
    }
}
