//! Locating the ComfyUI entrypoint and turning it into a [`SpawnSpec`]. Kept
//! apart from the adapter's state machine in `mod.rs`.
//!
//! Real ComfyUI is `<venv python> <checkout>/main.py`; the test fixture
//! (`aiwm-fake-comfy`) is a single self-contained executable. [`ComfyLaunch`]
//! covers both — `main` is `None` for the fixture.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

#[cfg(test)]
use super::VramMode;
use super::{ComfyOptions, RUNTIME_ID};
use crate::model::ModelKind;
use crate::runtime::SpawnSpec;
use crate::{CoreError, Result};

/// Env override for the ComfyUI interpreter (or a standalone fixture exe).
pub(super) const PYTHON_ENV: &str = "AIWM_COMFYUI_PYTHON";
/// Env override for the ComfyUI checkout directory (the one holding `main.py`).
pub(super) const DIR_ENV: &str = "AIWM_COMFYUI_DIR";

const PYTHON_EXE: &str = if cfg!(windows) {
    ".venv\\Scripts\\python.exe"
} else {
    ".venv/bin/python"
};

/// How to start the ComfyUI server.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComfyLaunch {
    /// The interpreter, or a self-contained executable when `main` is `None`.
    pub program: PathBuf,
    /// `main.py` for a real checkout; `None` for the fixture.
    pub main: Option<PathBuf>,
    /// Extra raw arguments, appended verbatim (Settings-configurable flags in
    /// 3.7; the test fixture's `--fake-ready-ms`).
    pub extra_args: Vec<String>,
}

/// The directories ComfyUI is told to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComfyDirs {
    /// `--base-directory` — ComfyUI's `models/`, `input/`, `temp/`.
    pub base: PathBuf,
    /// `--output-directory` — where generated images land.
    pub output: PathBuf,
    /// Root of the canonical model store (`Config::store_path`). ComfyUI is
    /// pointed at its `image/*` subfolders via `extra_model_paths.yaml` (3.3).
    pub models_store: PathBuf,
}

impl ComfyDirs {
    /// Where the generated `extra_model_paths.yaml` lives.
    pub(super) fn model_paths_yaml(&self) -> PathBuf {
        self.base.join("aiwm-model-paths.yaml")
    }

    /// ComfyUI's `input/` folder — where `LoadImage` reads from. Image→video
    /// (4.2) stages a start frame here.
    pub(super) fn input(&self) -> PathBuf {
        self.base.join("input")
    }

    /// Create the base + output directories, junction `custom_nodes` in from
    /// the real install (see below), and (re)write the model-paths YAML.
    /// Idempotent; called before every server start so a changed store path or
    /// a fresh install is picked up.
    ///
    /// `install_dir` is the ComfyUI checkout (`main.py`'s parent) when known —
    /// `None` for the test fixture, which has no real `custom_nodes` to link.
    pub(super) fn ensure(&self, install_dir: Option<&Path>) -> Result<()> {
        for dir in [&self.base, &self.output] {
            std::fs::create_dir_all(dir).map_err(|e| dir_err(dir, e))?;
        }
        if let Some(install_dir) = install_dir {
            self.ensure_custom_nodes_link(install_dir)?;
        }
        let yaml = self.model_paths_yaml();
        std::fs::write(&yaml, model_paths_yaml_body(&self.models_store))
            .map_err(|e| dir_err(&yaml, e))
    }

    /// `--base-directory` redirects `custom_nodes` there too (confirmed
    /// against the real ComfyUI's own `--help` — its doc comment above only
    /// mentions models/input/temp because this was never run against a real
    /// checkout before, only the fake fixture). Without this, ComfyUI crashes
    /// on startup (`os.listdir` on a `custom_nodes` that doesn't exist under
    /// `base`) and the installed nodes (`ComfyUI-GGUF`) would never load even
    /// if it didn't. A junction keeps the real, single copy authoritative —
    /// same volume only, which holds by default (both live under the app's
    /// data root, ADR-026) but not if `runtimes_path` was moved to another
    /// drive in Settings.
    fn ensure_custom_nodes_link(&self, install_dir: &Path) -> Result<()> {
        let real_nodes = install_dir.join("custom_nodes");
        if !real_nodes.is_dir() {
            return Ok(()); // nothing installed yet — `ensure_server_locked` won't get this far anyway
        }
        crate::link::create_junction(&real_nodes, &self.base.join("custom_nodes")).map_err(|e| {
            CoreError::Runtime {
                runtime: RUNTIME_ID.into(),
                message: format!(
                    "couldn't expose ComfyUI's custom_nodes under its data directory: {e}. \
                     The data directory and the ComfyUI install need to be on the same drive \
                     (see Settings → Data locations)."
                ),
            }
        })
    }
}

fn dir_err(path: &Path, e: std::io::Error) -> CoreError {
    CoreError::Runtime {
        runtime: RUNTIME_ID.into(),
        message: format!("comfyui setup: {}: {e}", path.display()),
    }
}

/// The `extra_model_paths.yaml` body: point ComfyUI at `<store>/image/<folder>/`
/// for every model kind (ADR-019). A junction can't reach the store — it's on a
/// different volume from the ComfyUI install — but this config can. One block
/// per store area (`image/`, `video/`); ComfyUI searches all blocks.
fn model_paths_yaml_body(store_root: &Path) -> String {
    let mut yaml = String::from(
        "# Generated by AI Workstation Manager — do not edit; it is rewritten on each launch.\n",
    );
    write_paths_block(
        &mut yaml,
        "aiwm",
        store_root,
        "image",
        &ModelKind::IMAGE_KINDS,
    );
    write_paths_block(
        &mut yaml,
        "aiwm_video",
        store_root,
        "video",
        &ModelKind::VIDEO_KINDS,
    );
    // LTX-Video ships as one bundled checkpoint (model + VAE), loaded via
    // ComfyUI's `CheckpointLoaderSimple` -- unlike Wan's bare UNet, loaded via
    // `UNETLoader`. There's only one video-model import kind, so both land in
    // the same folder above under `diffusion_models:` -- register that same
    // folder under `checkpoints:` too, or an imported LTX file is invisible
    // to the node that actually needs to load it.
    let video_dir = store_root
        .join("video")
        .join("diffusion_models")
        .to_string_lossy()
        .replace('\\', "/");
    let _ = writeln!(yaml, "aiwm_video_checkpoints:");
    let _ = writeln!(yaml, "  base_path: {video_dir}");
    let _ = writeln!(yaml, "  checkpoints: ./");
    yaml
}

fn write_paths_block(
    yaml: &mut String,
    name: &str,
    store_root: &Path,
    area: &str,
    kinds: &[ModelKind],
) {
    let base = store_root.join(area).to_string_lossy().replace('\\', "/");
    let _ = writeln!(yaml, "{name}:");
    let _ = writeln!(yaml, "  base_path: {base}");
    let prefix = format!("{area}/");
    for kind in kinds {
        let Some(folder) = kind.comfy_folder() else {
            continue;
        };
        let sub = kind.store_subdir().trim_start_matches(&prefix);
        let _ = writeln!(yaml, "  {folder}: {sub}/");
    }
}

/// Build the launch command. Always pins `--listen 127.0.0.1` (ADR-008) and
/// keeps ComfyUI headless + quiet. `opts` adds the VRAM-mode flag and any extra
/// args from the `[comfyui]` config table.
pub(super) fn build_spawn_spec(
    launch: &ComfyLaunch,
    port: u16,
    dirs: &ComfyDirs,
    opts: &ComfyOptions,
) -> SpawnSpec {
    let mut spec = SpawnSpec::new(&launch.program);
    if let Some(main) = &launch.main {
        spec = spec.arg(main.to_string_lossy().into_owned());
        if let Some(parent) = main.parent() {
            spec.cwd = Some(parent.to_path_buf());
        }
    }
    spec = spec
        .arg("--listen")
        .arg(Ipv4Addr::LOCALHOST.to_string())
        .arg("--port")
        .arg(port.to_string())
        .arg("--base-directory")
        .arg(dirs.base.to_string_lossy().into_owned())
        .arg("--output-directory")
        .arg(dirs.output.to_string_lossy().into_owned())
        .arg("--extra-model-paths-config")
        .arg(dirs.model_paths_yaml().to_string_lossy().into_owned())
        .arg("--disable-auto-launch")
        .arg("--dont-print-server");
    for extra in launch.extra_args.iter().chain(opts.args().iter()) {
        spec = spec.arg(extra.clone());
    }
    spec
}

/// Resolve how to launch ComfyUI: `AIWM_COMFYUI_PYTHON` (+ optional
/// `AIWM_COMFYUI_DIR`), then a managed install under
/// `<runtimes_dir>/comfyui/<version>/`. `lookup` reads env vars (injected for
/// tests). `None` when nothing is installed.
pub(super) fn resolve_launch(
    runtimes_dir: &Path,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<ComfyLaunch> {
    if let Some(raw) = lookup(PYTHON_ENV).filter(|s| !s.is_empty()) {
        let program = PathBuf::from(raw);
        if program.is_file() {
            let main = lookup(DIR_ENV)
                .filter(|s| !s.is_empty())
                .map(|d| PathBuf::from(d).join("main.py"))
                .filter(|p| p.is_file());
            return Some(ComfyLaunch {
                program,
                main,
                extra_args: Vec::new(),
            });
        }
    }

    // Managed: <runtimes_dir>/comfyui/<version>/main.py + .venv python.
    let managed_root = runtimes_dir.join(RUNTIME_ID);
    let entries = std::fs::read_dir(&managed_root).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = entry.path();
        let main = dir.join("main.py");
        let python = dir.join(PYTHON_EXE);
        if main.is_file() && python.is_file() {
            return Some(ComfyLaunch {
                program: python,
                main: Some(main),
                extra_args: Vec::new(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> ComfyDirs {
        ComfyDirs {
            base: PathBuf::from("C:\\aiwm\\comfyui-data"),
            output: PathBuf::from("C:\\aiwm\\outputs"),
            models_store: PathBuf::from("E:\\AI\\models"),
        }
    }

    #[test]
    fn model_paths_yaml_maps_image_and_video_kinds() {
        let body = model_paths_yaml_body(Path::new("E:\\AI\\models"));
        assert!(body.contains("aiwm:"));
        assert!(body.contains("base_path: E:/AI/models/image"));
        assert!(body.contains("checkpoints: checkpoints/"));
        assert!(body.contains("diffusion_models: diffusion_models/"));
        assert!(body.contains("vae: vae/"));
        assert!(body.contains("loras: loras/"));
        assert!(body.contains("text_encoders: text_encoders/"));
        // Story Studio Phase 2's character-consistency companions.
        assert!(body.contains("clip_vision: clip_vision/"));
        assert!(body.contains("ipadapter: ipadapter/"));
        // second block for the video store
        assert!(body.contains("aiwm_video:"));
        assert!(body.contains("base_path: E:/AI/models/video"));
    }

    #[test]
    fn model_paths_yaml_also_registers_the_video_folder_as_checkpoints_for_ltx() {
        // LTX-Video ships as one bundled checkpoint (model + VAE), loaded via
        // ComfyUI's `CheckpointLoaderSimple` -- unlike Wan's bare UNet, loaded
        // via `UNETLoader`. Both land in the same imported video folder
        // (there's only one video-model import kind), so ComfyUI's
        // "checkpoints" category needs to see that folder too, or an LTX file
        // imported through AIWM is invisible to the node that loads it
        // ("ckpt_name: '...' not in [...]").
        let body = model_paths_yaml_body(Path::new("E:\\AI\\models"));
        assert!(body.contains("base_path: E:/AI/models/video/diffusion_models"));
        assert!(body.contains("checkpoints: ./"));
    }

    #[test]
    fn ensure_writes_the_yaml_and_makes_the_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let d = ComfyDirs {
            base: tmp.path().join("data"),
            output: tmp.path().join("out"),
            models_store: tmp.path().join("store"),
        };
        d.ensure(None).unwrap();
        assert!(d.base.is_dir() && d.output.is_dir());
        let yaml = std::fs::read_to_string(d.model_paths_yaml()).unwrap();
        assert!(yaml.contains("base_path:") && yaml.contains("checkpoints:"));
        d.ensure(None).unwrap(); // idempotent
    }

    #[test]
    fn ensure_exposes_the_real_custom_nodes_under_the_data_directory() {
        // `--base-directory` redirects ComfyUI's own `custom_nodes` lookup
        // there too (real ComfyUI, not just the fake fixture) — without this,
        // ComfyUI crashes on startup instead of finding the installed nodes
        // (e.g. ComfyUI-GGUF) at all.
        let tmp = tempfile::tempdir().unwrap();
        let install_dir = tmp.path().join("checkout");
        let real_nodes = install_dir.join("custom_nodes");
        std::fs::create_dir_all(&real_nodes).unwrap();
        std::fs::write(real_nodes.join("marker.txt"), b"comfyui-gguf").unwrap();

        let d = ComfyDirs {
            base: tmp.path().join("data"),
            output: tmp.path().join("out"),
            models_store: tmp.path().join("store"),
        };
        d.ensure(Some(&install_dir)).unwrap();

        let linked = d.base.join("custom_nodes").join("marker.txt");
        assert_eq!(std::fs::read(linked).unwrap(), b"comfyui-gguf");

        // Idempotent: a second `ensure` (every server start) must not error.
        d.ensure(Some(&install_dir)).unwrap();
    }

    #[test]
    fn spawn_spec_for_a_real_checkout_runs_python_with_main_and_flags() {
        let launch = ComfyLaunch {
            program: PathBuf::from("C:\\c\\.venv\\Scripts\\python.exe"),
            main: Some(PathBuf::from("C:\\c\\ComfyUI\\main.py")),
            extra_args: vec!["--reserve-vram".into()],
        };
        let opts = ComfyOptions {
            vram_mode: VramMode::LowVram,
            extra_args: Vec::new(),
        };
        let spec = build_spawn_spec(&launch, 48311, &dirs(), &opts);
        assert_eq!(spec.program, launch.program);
        assert_eq!(spec.cwd, Some(PathBuf::from("C:\\c\\ComfyUI")));
        let joined = spec.args.join(" ");
        assert!(joined.starts_with("C:\\c\\ComfyUI\\main.py"));
        assert!(joined.contains("--listen 127.0.0.1"));
        assert!(joined.contains("--port 48311"));
        assert!(joined.contains("--base-directory C:\\aiwm\\comfyui-data"));
        assert!(joined.contains("--output-directory C:\\aiwm\\outputs"));
        assert!(joined
            .contains("--extra-model-paths-config C:\\aiwm\\comfyui-data\\aiwm-model-paths.yaml"));
        assert!(joined.contains("--disable-auto-launch"));
        assert!(joined.contains("--dont-print-server"));
        // launch.extra_args, then the options' VRAM flag, last.
        assert!(
            joined.ends_with("--reserve-vram --lowvram"),
            "options args come last: {joined}"
        );
    }

    #[test]
    fn spawn_spec_for_the_fixture_has_no_main_arg_and_no_vram_flag_on_auto() {
        let launch = ComfyLaunch {
            program: PathBuf::from("aiwm-fake-comfy.exe"),
            ..ComfyLaunch::default()
        };
        let spec = build_spawn_spec(&launch, 1, &dirs(), &ComfyOptions::default());
        assert!(spec.cwd.is_none());
        assert!(!spec.args.iter().any(|a| a.ends_with("main.py")));
        let joined = spec.args.join(" ");
        assert!(joined.contains("--port 1"));
        assert!(!joined.contains("vram"), "Auto adds no flag: {joined}");
    }

    #[test]
    fn resolve_prefers_python_env_and_pairs_it_with_a_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let python = tmp.path().join("python.exe");
        std::fs::write(&python, b"x").unwrap();
        let comfy = tmp.path().join("ComfyUI");
        std::fs::create_dir_all(&comfy).unwrap();
        std::fs::write(comfy.join("main.py"), b"x").unwrap();

        let got = resolve_launch(Path::new("Z:\\nope"), |k| match k {
            PYTHON_ENV => Some(python.clone().into()),
            DIR_ENV => Some(comfy.clone().into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(got.program, python);
        assert_eq!(got.main, Some(comfy.join("main.py")));
    }

    #[test]
    fn resolve_allows_a_standalone_exe_without_a_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("fake-comfy.exe");
        std::fs::write(&exe, b"x").unwrap();

        let got = resolve_launch(Path::new("Z:\\nope"), |k| {
            (k == PYTHON_ENV).then(|| exe.clone().into())
        })
        .unwrap();
        assert_eq!(got.program, exe);
        assert_eq!(got.main, None);
    }

    #[test]
    fn resolve_finds_a_managed_install() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(RUNTIME_ID).join("0.34.0");
        std::fs::create_dir_all(dir.join(".venv").join(if cfg!(windows) {
            "Scripts"
        } else {
            "bin"
        }))
        .unwrap();
        std::fs::write(dir.join("main.py"), b"x").unwrap();
        std::fs::write(dir.join(PYTHON_EXE), b"x").unwrap();

        let got = resolve_launch(tmp.path(), |_| None).unwrap();
        assert_eq!(got.main, Some(dir.join("main.py")));
    }

    #[test]
    fn resolve_is_none_when_nothing_is_installed() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolve_launch(tmp.path(), |_| None), None);
    }
}
