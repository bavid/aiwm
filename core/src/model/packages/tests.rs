use super::*;
use crate::model::family::{family_by_id, FAMILIES};

fn known(id: &str) -> &'static KnownModel {
    KNOWN_MODELS.iter().find(|k| k.id == id).unwrap()
}

fn model(id: &str, name: &str, roles: &[&str]) -> Model {
    Model {
        id: id.into(),
        publisher: None,
        name: name.into(),
        family: None,
        base_family: None,
        family_source: None,
        format: "safetensors".into(),
        quant: None,
        arch: None,
        param_count: None,
        file_path: format!("E:\\AI\\models\\{name}"),
        sha256: None,
        size_bytes: 1,
        ctx_max: None,
        vram_estimate_mb: None,
        ram_estimate_mb: None,
        source: "manual".into(),
        source_revision: None,
        imported_at: "2026-09-21T00:00:00Z".into(),
        last_used_at: None,
        use_count: 0,
        n_layers: None,
        n_embd: None,
        n_heads: None,
        n_kv_heads: None,
        roles: roles.iter().map(|r| (*r).to_string()).collect(),
        runtimes: vec![],
    }
}

/// A library model whose family is already decided.
fn lib(id: &str, roles: &[&str], family: Option<(&str, FamilySource)>) -> LibraryModel {
    LibraryModel {
        model: model(id, id, roles),
        family: family.map(|(f, s)| (family_by_id(f).unwrap(), s)),
    }
}

/// The catalog file `known_id`, installed (matched by its SHA-256 only).
fn installed_file(id: &str, known_id: &str, roles: &[&str]) -> LibraryModel {
    let mut m = lib(id, roles, None);
    m.model.sha256 = Some(known(known_id).sha256.to_ascii_uppercase());
    m
}

fn item(kind: ItemKind, family: Option<&str>, label: Option<&str>) -> PackageItem {
    PackageItem {
        name: "picked".into(),
        kind,
        base_label: label.map(str::to_string),
        model_id: None,
        size_bytes: Some(100),
        family: family.map(|f| (family_by_id(f).unwrap(), FamilySource::Civitai)),
    }
}

fn resolve(item: PackageItem, library: &[LibraryModel]) -> Package {
    resolve_package(item, library, FAMILIES, &Catalog::builtin())
}

fn installed_id(n: &Need) -> Option<(&str, bool)> {
    match &n.status {
        NeedStatus::Installed {
            model_id, made_for, ..
        } => Some((model_id.as_str(), *made_for)),
        _ => None,
    }
}

/// A need as (catalog id, installed (model id, made for)).
type Shape<'a> = (Option<&'static str>, Option<(&'a str, bool)>);

fn shape(n: &Need) -> Shape<'_> {
    (catalog_id(n), installed_id(n))
}

fn catalog_id(n: &Need) -> Option<&'static str> {
    match n.status {
        NeedStatus::Catalog { known_model_id, .. } => Some(known_model_id),
        _ => None,
    }
}

fn klein_stack_installed() -> Vec<LibraryModel> {
    vec![
        installed_file("klein", "flux2-klein-9b-q4", &["base_diffusion"]),
        installed_file("qwen", "qwen3-8b-flux2-encoder", &["text_encoder"]),
        installed_file("vae", "flux2-vae", &["vae"]),
        installed_file("edit-vae", "flux2-klein-edit-vae", &["vae"]),
    ]
}

#[test]
fn a_lora_with_its_whole_stack_installed_is_ready() {
    let p = resolve(
        item(
            ItemKind::Lora,
            Some("flux2-klein-9b"),
            Some("Flux.2 Klein 9B"),
        ),
        &klein_stack_installed(),
    );
    assert_eq!(p.verdict, Verdict::Ready);
    assert_eq!(p.missing_bytes, 0);
    let got: Vec<(NeedRole, Option<(&str, bool)>)> =
        p.needs.iter().map(|n| (n.role, installed_id(n))).collect();
    assert_eq!(
        got,
        [
            (NeedRole::Base, Some(("klein", true))),
            (NeedRole::TextEncoder, Some(("qwen", true))),
            (NeedRole::Vae, Some(("vae", true))),
            (NeedRole::Vae, Some(("edit-vae", true))),
        ]
    );
}

#[test]
fn a_missing_base_with_a_catalog_stack_comes_from_the_catalog() {
    let p = resolve(item(ItemKind::Lora, Some("sdxl"), Some("SDXL 1.0")), &[]);
    assert_eq!(p.verdict, Verdict::NeedsDownload);
    assert_eq!(p.needs.len(), 1);
    assert_eq!(catalog_id(&p.needs[0]), Some("sdxl-base-1.0"));
    assert!(!p.needs[0].optional);
    assert_eq!(p.missing_bytes, known("sdxl-base-1.0").size_bytes);
}

#[test]
fn a_base_without_a_stack_is_findable_by_its_label() {
    let p = resolve(item(ItemKind::Lora, Some("pony"), Some("Pony")), &[]);
    assert_eq!(p.verdict, Verdict::NeedsDownload);
    assert_eq!(p.missing_bytes, 0, "a findable base has no size yet");
    assert_eq!(p.needs.len(), 1);
    assert!(!p.needs[0].optional);
    assert_eq!(
        p.needs[0].status,
        NeedStatus::Findable {
            base_label: "Pony".into(),
            candidates: vec![],
            note: None,
        }
    );
}

#[test]
fn a_pony_lora_works_with_sdxl_and_suggests_its_own_checkpoint() {
    let library = [lib(
        "sdxl-base",
        &["base_diffusion"],
        Some(("sdxl", FamilySource::Catalog)),
    )];
    let p = resolve(item(ItemKind::Lora, Some("pony"), Some("Pony")), &library);
    assert_eq!(p.verdict, Verdict::Ready, "no false \"missing\"");
    assert_eq!(p.needs.len(), 2);
    assert_eq!(installed_id(&p.needs[0]), Some(("sdxl-base", false)));
    assert!(!p.needs[0].optional);
    assert!(p.needs[1].optional, "the Pony checkpoint is a suggestion");
    assert!(matches!(
        &p.needs[1].status,
        NeedStatus::Findable { base_label, .. } if base_label == "Pony"
    ));

    // With a Pony checkpoint installed it is made for it — nothing else.
    let library = [
        lib(
            "sdxl-base",
            &["base_diffusion"],
            Some(("sdxl", FamilySource::Catalog)),
        ),
        lib(
            "pony-ckpt",
            &["base_diffusion"],
            Some(("pony", FamilySource::Header)),
        ),
    ];
    let p = resolve(item(ItemKind::Lora, Some("pony"), Some("Pony")), &library);
    assert_eq!(p.needs.len(), 1);
    assert_eq!(installed_id(&p.needs[0]), Some(("pony-ckpt", true)));
}

#[test]
fn an_sdxl_lora_on_a_pony_checkpoint_suggests_the_catalog_sdxl_base() {
    let library = [lib(
        "pony-ckpt",
        &["base_diffusion"],
        Some(("pony", FamilySource::Civitai)),
    )];
    let p = resolve(item(ItemKind::Lora, Some("sdxl"), None), &library);
    assert_eq!(p.verdict, Verdict::Ready);
    assert_eq!(installed_id(&p.needs[0]), Some(("pony-ckpt", false)));
    assert_eq!(catalog_id(&p.needs[1]), Some("sdxl-base-1.0"));
    assert!(p.needs[1].optional);
    assert_eq!(p.missing_bytes, 0, "an optional suggestion is not missing");
}

#[test]
fn a_wan_14b_lora_is_not_runnable_even_with_a_14b_base_installed() {
    let library = [lib(
        "wan14",
        &["base_video"],
        Some(("wan-14b", FamilySource::Header)),
    )];
    let p = resolve(
        item(
            ItemKind::Lora,
            Some("wan-14b"),
            Some("Wan Video 2.2 I2V-A14B"),
        ),
        &library,
    );
    let Verdict::NotRunnable(reason) = p.verdict else {
        panic!("{:?}", p.verdict);
    };
    assert!(reason.contains("16 GB"), "{reason}");
    assert_eq!(p.needs.len(), 1);
    assert_eq!(p.needs[0].status, NeedStatus::NotRunnable { reason });
}

#[test]
fn an_unknown_base_says_so_and_needs_nothing() {
    let p = resolve(item(ItemKind::Lora, None, Some("ZImageBase")), &[]);
    assert_eq!(p.verdict, Verdict::UnknownBase);
    assert!(p.needs.is_empty());
    assert!(p.family.is_none());
}

#[test]
fn a_checkpoint_needs_its_companions_not_a_base() {
    let library = [installed_file("vae", "flux2-vae", &["vae"])];
    let p = resolve(
        item(ItemKind::Checkpoint, Some("flux2-klein-9b"), None),
        &library,
    );
    assert!(p.needs.iter().all(|n| n.role != NeedRole::Base));
    let got: Vec<Shape> = p.needs.iter().map(shape).collect();
    assert_eq!(
        got,
        [
            (Some("qwen3-8b-flux2-encoder"), None),
            (None, Some(("vae", true))),
            (Some("flux2-klein-edit-vae"), None),
        ]
    );
    // The edit VAE is listed but optional: only the text encoder counts.
    assert_eq!(p.missing_bytes, known("qwen3-8b-flux2-encoder").size_bytes);
    let edit_vae = p
        .needs
        .iter()
        .find(|n| shape(n).0 == Some("flux2-klein-edit-vae"))
        .expect("the edit VAE is still offered");
    assert!(edit_vae.optional);
    assert_eq!(p.verdict, Verdict::NeedsDownload);
}

#[test]
fn a_text_to_image_klein_setup_without_the_edit_vae_is_ready() {
    // Regression: the edit VAE is only for editing a photo, so a library
    // with the diffusion model, its encoder and its VAE is complete.
    let library: Vec<LibraryModel> = klein_stack_installed()
        .into_iter()
        .filter(|m| m.model.id != "edit-vae")
        .collect();

    let p = resolve(
        item(ItemKind::Checkpoint, Some("flux2-klein-9b"), None),
        &library,
    );

    assert_eq!(p.missing_bytes, 0);
    assert_eq!(p.verdict, Verdict::Ready);
}

#[test]
fn a_family_without_a_stack_borrows_its_architectures_companions() {
    let p = resolve(
        item(ItemKind::Lora, Some("flux1-krea"), Some("Flux.1 Krea")),
        &[],
    );
    let ids: Vec<Option<&str>> = p.needs.iter().map(catalog_id).collect();
    assert_eq!(
        ids,
        [None, Some("t5xxl-fp8"), Some("clip-l"), Some("flux-vae")]
    );
    assert!(matches!(
        &p.needs[0].status,
        NeedStatus::Findable { base_label, .. } if base_label == "Flux.1 Krea"
    ));
}

#[test]
fn the_sha256_beats_a_matching_name_and_family() {
    // "sd_xl_base_1.0" looks like the catalog base by name and family; the
    // renamed file is the catalog base by content.
    let by_name = lib(
        "by-name",
        &["base_diffusion"],
        Some(("sdxl", FamilySource::Name)),
    );
    let by_hash = installed_file("by-hash", "sdxl-base-1.0", &["base_diffusion"]);
    let p = resolve(
        item(ItemKind::Lora, Some("sdxl"), None),
        &[by_name, by_hash],
    );
    assert_eq!(installed_id(&p.needs[0]), Some(("by-hash", true)));
}

#[test]
fn the_catalog_id_recorded_at_import_matches_before_the_role() {
    let by_role = lib(
        "by-role",
        &["vae"],
        Some(("flux2-klein-9b", FamilySource::Name)),
    );
    let mut by_catalog = lib("by-catalog", &["vae"], None);
    by_catalog.model.source_revision = Some("catalog:flux2-vae".into());
    let p = resolve(
        item(ItemKind::Checkpoint, Some("flux2-klein-9b"), None),
        &[by_role, by_catalog],
    );
    assert_eq!(installed_id(&p.needs[1]), Some(("by-catalog", true)));
}

// ---- library grouping --------------------------------------------------------

fn recorded(id: &str, roles: &[&str], base_family: &str, source: &str) -> LibraryModel {
    let mut m = model(id, id, roles);
    m.base_family = Some(base_family.into());
    m.family_source = Some(source.into());
    LibraryModel::infer(m, None)
}

#[test]
fn the_library_groups_by_family_with_orphans() {
    let mut library = vec![
        recorded("sdxl-base", &["base_diffusion"], "sdxl", "catalog"),
        recorded("sdxl-lora", &["lora"], "sdxl", "civitai"),
        recorded("pony-1", &["lora"], "pony", "civitai"),
        recorded("pony-2", &["lora"], "pony", "header"),
        recorded("wan-lora", &["lora"], "wan-14b", "civitai"),
        LibraryModel::infer(model("mystery", "mystery.safetensors", &["lora"]), None),
        recorded("klein-lora", &["lora"], "flux2-klein-9b", "hf"),
    ];
    library.push(installed_file(
        "klein",
        "flux2-klein-9b-q4",
        &["base_diffusion"],
    ));
    library.push(installed_file("vae", "flux2-vae", &["vae"]));
    library.push(installed_file("edit-vae", "flux2-klein-edit-vae", &["vae"]));

    let out = library_packages(&library, FAMILIES, &Catalog::builtin());
    let ids: Vec<&str> = out.groups.iter().map(|g| g.family.id).collect();
    assert_eq!(ids, ["sdxl", "pony", "flux2-klein-9b", "wan-14b"]);

    let sdxl = &out.groups[0];
    assert!(sdxl.complete);
    assert_eq!(sdxl.base.as_ref().unwrap().model.id, "sdxl-base");
    assert_eq!(sdxl.loras.len(), 1);

    let pony = &out.groups[1];
    assert!(!pony.complete, "no Pony checkpoint");
    assert!(pony.base.is_none());
    assert_eq!(pony.loras.len(), 2);
    assert_eq!(pony.loras[1].family_source, FamilySource::Header);
    assert_eq!(
        installed_id(&pony.base_needs[0]),
        Some(("sdxl-base", false))
    );
    assert!(pony.base_needs[1].optional);

    let klein = &out.groups[2];
    assert!(!klein.complete, "the text encoder is missing");
    assert_eq!(klein.base.as_ref().unwrap().model.id, "klein");
    assert_eq!(
        klein.missing_bytes,
        known("qwen3-8b-flux2-encoder").size_bytes
    );

    let wan = &out.groups[3];
    assert!(!wan.complete);
    assert!(matches!(
        wan.base_needs[0].status,
        NeedStatus::NotRunnable { .. }
    ));

    let orphans: Vec<&str> = out.orphans.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(orphans, ["mystery"]);
}
