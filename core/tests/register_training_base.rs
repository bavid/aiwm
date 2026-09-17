//! Maintenance harness: register an already-downloaded base-weights snapshot
//! as a Model Library directory entry with its training role and family.
//!
//! **Why this exists as an ignored test rather than a route.** The HTTP API
//! has exactly one directory-registration endpoint, `POST /models/colibri`,
//! and it is hard-wired to the Colibri chat catalog: it looks the request up
//! in `COLIBRI_MODELS`, forces `format = "colibri"` and `roles = ["chat"]`,
//! and has no way to set `family`. A FLUX.2 [klein] training base needs a
//! different format, a `training_base_*` role and a size-specific family, so
//! there is nothing to call. Until the Models tab grows a general "register a
//! folder" action (tracked in `docs/TODO.md`), this is the supported path —
//! the same `register_directory_model` + `set_family_and_source` the app's
//! own code uses, driven against the real database.
//!
//! Ignored by default because it writes to a real, configured AIWM data
//! directory. Run it deliberately:
//!
//! ```text
//! AIWM_DATA_DIR=E:\AI\data \
//! AIWM_REGISTER_BASE_DIR=E:\AI\models\training\flux2-klein-4b \
//! AIWM_REGISTER_BASE_NAME="FLUX.2 [klein] 4B base (training)" \
//! AIWM_REGISTER_BASE_FAMILY=flux2-klein-4b \
//! AIWM_REGISTER_BASE_ROLE=training_base_flux2_klein_4b \
//!   cargo test -p aiwm-core --test register_training_base -- --ignored --nocapture
//! ```
//!
//! Re-running it for a directory that is already registered is a no-op that
//! reports the existing row rather than inserting a duplicate.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use aiwm_core::db::Database;
use aiwm_core::paths::AppPaths;
use aiwm_core::training::bases::{find_base, verify_base_dir};

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} must be set for this harness"))
}

#[tokio::test]
#[ignore = "writes to a real AIWM data directory; run deliberately (see the module docs)"]
async fn register_a_downloaded_training_base() {
    let dir = PathBuf::from(env("AIWM_REGISTER_BASE_DIR"));
    let name = env("AIWM_REGISTER_BASE_NAME");
    let family = env("AIWM_REGISTER_BASE_FAMILY");
    let role = env("AIWM_REGISTER_BASE_ROLE");
    let format = std::env::var("AIWM_REGISTER_BASE_FORMAT").unwrap_or_else(|_| "diffusers".into());

    assert!(dir.is_dir(), "{} is not a directory", dir.display());

    // Refuse to register a snapshot the runner would then refuse to start on.
    // Full verification here: this runs once, by hand, and reading the whole
    // 16 GB is the point -- it is the only moment the complete download is
    // ever checked end to end.
    if let Some(base) = find_base(&family) {
        verify_base_dir(&dir, base, true).expect("the snapshot must match its pinned manifest");
        println!(
            "verified {} against the pinned manifest (full)",
            dir.display()
        );
    } else {
        println!("no pinned manifest for {family} -- registering without verification");
    }

    let paths = AppPaths::for_app().expect("resolve the data directory");
    println!("data dir: {}", paths.root().display());
    let db = Database::connect(&paths.db_file())
        .await
        .expect("open the database");

    let wanted = dir.to_string_lossy().into_owned();
    let existing = db
        .models()
        .list()
        .await
        .expect("list models")
        .into_iter()
        .find(|m| same_dir(Path::new(m.file_path.as_str()), &dir));

    let id = match existing {
        Some(model) => {
            println!("already registered as {} ({})", model.id, model.name);
            model.id
        }
        None => {
            let model = aiwm_core::model::register_directory_model(
                &db,
                &dir,
                &name,
                &format,
                vec![role.clone()],
                None,
            )
            .await
            .expect("register the directory");
            println!(
                "registered {} as {} ({} bytes)",
                wanted, model.id, model.size_bytes
            );
            model.id
        }
    };

    // `register_directory_model` cannot set `family` -- the training profile
    // registry resolves a target model by it, so the run would never find its
    // profile without this.
    db.models()
        .set_family_and_source(&id, Some(&family), "manual")
        .await
        .expect("set the family");
    db.models()
        .set_roles(&id, std::slice::from_ref(&role))
        .await
        .expect("set the role");

    let after = db
        .models()
        .get(&id)
        .await
        .expect("re-read")
        .expect("the row must still be there");
    println!(
        "id={} name={:?} family={:?} format={} roles={:?} path={}",
        after.id, after.name, after.family, after.format, after.roles, after.file_path
    );
    assert_eq!(after.family.as_deref(), Some(family.as_str()));
    assert!(after.roles.contains(&role));
    db.close().await;
}

/// Windows paths differ by separator and case without differing at all.
fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    norm(a) == norm(b)
}
