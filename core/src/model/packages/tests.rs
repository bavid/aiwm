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
        catalog_id: None,
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
fn a_library_group_without_only_the_optional_edit_vae_is_complete() {
    // The group view and `resolve_package` must agree: the edit VAE is
    // offered, never required, so base + encoder + VAE is a complete setup.
    let library: Vec<LibraryModel> = klein_stack_installed()
        .into_iter()
        .filter(|m| m.model.id != "edit-vae")
        .collect();

    let out = library_packages(&library, FAMILIES, &Catalog::builtin());
    let klein = out
        .groups
        .iter()
        .find(|g| g.family.id == "flux2-klein-9b")
        .expect("the klein group");

    assert!(klein.complete, "only the optional edit VAE is missing");
    assert_eq!(klein.missing_bytes, 0);
}

#[test]
fn the_library_groups_by_family_with_unknown_models() {
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

    let unknown: Vec<&str> = out.unknown.iter().map(|m| m.model.id.as_str()).collect();
    assert_eq!(unknown, ["mystery"]);
}

// ---- real-run findings (2026-09-21) ------------------------------------------

/// A library row shaped like the real one, its family inferred on read:
/// legacy `family`, the catalog tag the import recorded, roles.
fn real(id: &str, legacy: Option<&str>, catalog: Option<&str>, roles: &[&str]) -> LibraryModel {
    let mut m = model(id, id, roles);
    m.file_path = format!("E:\\AI\\models\\{id}.safetensors");
    m.family = legacy.map(str::to_string);
    m.source_revision = catalog.map(|c| format!("catalog:{c}"));
    LibraryModel::infer(m, None)
}

fn used_by(p: &Package) -> Vec<&str> {
    p.used_by.iter().map(|f| f.id).collect()
}

fn resolve_library(id: &str, library: &[LibraryModel]) -> Package {
    let m = library.iter().find(|m| m.model.id == id).unwrap();
    resolve(library_item(m, &Catalog::builtin()), library)
}

#[test]
fn a_catalog_text_encoder_resolves_to_the_stacks_that_use_it() {
    let library = [
        real(
            "t5xxl_fp8_e4m3fn",
            Some("sdxl"),
            Some("t5xxl-fp8"),
            &["text_encoder"],
        ),
        real(
            "umt5_xxl_fp8_e4m3fn_scaled",
            Some("sdxl"),
            Some("wan-umt5-xxl-fp8"),
            &["text_encoder"],
        ),
        real(
            "sd_xl_base_1.0",
            Some("sdxl"),
            Some("sdxl-base-1.0"),
            &["base_diffusion"],
        ),
    ];
    let t5 = resolve_library("t5xxl_fp8_e4m3fn", &library);
    assert_eq!(t5.item.kind, ItemKind::Companion);
    assert_eq!(t5.verdict, Verdict::Ready);
    assert!(t5.needs.is_empty(), "a companion needs no base");
    assert!(t5.family.is_none(), "{:?}", t5.family);
    assert_eq!(used_by(&t5), ["flux1", "ltxv"]);

    let umt5 = resolve_library("umt5_xxl_fp8_e4m3fn_scaled", &library);
    assert_eq!(umt5.verdict, Verdict::Ready);
    assert!(umt5.needs.is_empty());
    assert_eq!(used_by(&umt5), ["wan22-5b"]);
}

#[test]
fn a_companion_is_recognised_by_its_hash_and_never_needs_a_base() {
    // Matched by content only — no catalog tag, a wrong legacy string.
    let mut by_hash = real("renamed_encoder", Some("sdxl"), None, &["text_encoder"]);
    by_hash.model.sha256 = Some(known("qwen3-8b-flux2-encoder").sha256.into());
    // Not in the catalog at all: still a companion, with no stack to name.
    let stray = real("some_vae", Some("sdxl"), None, &["vae"]);
    let library = [by_hash, stray];

    let p = resolve_library("renamed_encoder", &library);
    assert_eq!(p.item.kind, ItemKind::Companion);
    assert_eq!(used_by(&p), ["flux2-klein-9b"]);
    let p = resolve_library("some_vae", &library);
    assert_eq!(p.item.kind, ItemKind::Companion);
    assert_eq!(p.verdict, Verdict::Ready);
    assert!(p.needs.is_empty() && p.used_by.is_empty() && p.family.is_none());
}

#[test]
fn a_pony_lora_works_with_the_canonical_sdxl_base_when_installed() {
    let mut animagine = real(
        "animagineXLV31_v31",
        Some("sdxl"),
        None,
        &["base_diffusion"],
    );
    animagine.model.use_count = 40;
    let library = [
        animagine,
        installed_file("sd_xl_base_1.0", "sdxl-base-1.0", &["base_diffusion"]),
    ];
    let p = resolve(item(ItemKind::Lora, Some("pony"), Some("Pony")), &library);
    assert_eq!(installed_id(&p.needs[0]), Some(("sd_xl_base_1.0", false)));
}

#[test]
fn without_the_canonical_base_a_lora_works_with_the_most_used_checkpoint() {
    let mut rare = real(
        "animagineXLV31_v31",
        Some("sdxl"),
        None,
        &["base_diffusion"],
    );
    rare.model.use_count = 2;
    let mut used = real(
        "mopMixtureOfPerverts",
        Some("sdxl"),
        None,
        &["base_diffusion"],
    );
    used.model.use_count = 9;
    let p = resolve(
        item(ItemKind::Lora, Some("pony"), Some("Pony")),
        &[rare, used],
    );
    assert_eq!(
        installed_id(&p.needs[0]),
        Some(("mopMixtureOfPerverts", false))
    );
}

#[test]
fn an_illustrious_lora_is_made_for_an_installed_illustrious_checkpoint() {
    let library = [
        installed_file("sd_xl_base_1.0", "sdxl-base-1.0", &["base_diffusion"]),
        real(
            "hassakuXLIllustrious_v34",
            Some("sdxl"),
            None,
            &["base_diffusion"],
        ),
    ];
    let p = resolve(
        item(ItemKind::Lora, Some("illustrious"), Some("Illustrious")),
        &library,
    );
    assert_eq!(p.needs.len(), 1, "no Illustrious checkpoint to fetch");
    assert_eq!(
        installed_id(&p.needs[0]),
        Some(("hassakuXLIllustrious_v34", true))
    );
}

fn real_library() -> Vec<LibraryModel> {
    let mut unnamed = real(
        "unnamedixlRealisticModel_v7",
        Some("sdxl"),
        None,
        &["base_diffusion"],
    );
    unnamed.model.use_count = 3;
    let krea = |id: &str, size: i64| {
        let mut m = real(id, None, None, &["base_diffusion"]);
        m.model.size_bytes = size;
        m
    };
    vec![
        real(
            "animagineXLV31_v31",
            Some("sdxl"),
            None,
            &["base_diffusion"],
        ),
        unnamed,
        installed_file("sd_xl_base_1.0", "sdxl-base-1.0", &["base_diffusion"]),
        real(
            "hassakuXLIllustrious_v34",
            Some("sdxl"),
            None,
            &["base_diffusion"],
        ),
        krea("realism_engine_krea2_v3.1", 13_000_000_000),
        krea("realism_engine_krea2_v2", 13_100_000_000),
        real("K_spreadinggape", None, None, &["lora"]),
        real(
            "t5xxl_fp8_e4m3fn",
            Some("sdxl"),
            Some("t5xxl-fp8"),
            &["text_encoder"],
        ),
        recorded("myrender-v2", &["lora"], "flux2-klein-4b", "header"),
        real(
            "FLUX.2 [klein] 4B base (training)",
            Some("flux2"),
            None,
            &["training_base_flux2_klein_4b"],
        ),
    ]
}

#[test]
fn every_installed_checkpoint_is_listed_canonical_first() {
    let out = library_packages(&real_library(), FAMILIES, &Catalog::builtin());
    let group = |id: &str| out.groups.iter().find(|g| g.family.id == id).unwrap();
    let names = |g: &LibraryGroup| -> Vec<String> {
        g.checkpoints.iter().map(|c| c.model.id.clone()).collect()
    };
    let sdxl = group("sdxl");
    assert_eq!(
        names(sdxl),
        [
            "sd_xl_base_1.0",
            "unnamedixlRealisticModel_v7",
            "animagineXLV31_v31"
        ]
    );
    assert_eq!(sdxl.base.as_ref().unwrap().model.id, "sd_xl_base_1.0");
    assert_eq!(names(group("illustrious")), ["hassakuXLIllustrious_v34"]);
    assert!(group("illustrious").complete);
}

#[test]
fn models_without_a_base_family_are_listed_as_unknown_with_their_kind() {
    let out = library_packages(&real_library(), FAMILIES, &Catalog::builtin());
    let unknown: Vec<(&str, ItemKind, i64)> = out
        .unknown
        .iter()
        .map(|u| (u.model.id.as_str(), u.kind, u.model.size_bytes))
        .collect();
    assert_eq!(
        unknown,
        [("K_spreadinggape", ItemKind::Lora, 1)],
        "companions, training folders and Krea 2 checkpoints are not \"unknown\""
    );
}

#[test]
fn krea2_checkpoints_group_under_krea2_and_need_its_encoder_and_vae() {
    let out = library_packages(&real_library(), FAMILIES, &Catalog::builtin());
    let krea = out
        .groups
        .iter()
        .find(|g| g.family.id == "krea2")
        .expect("a krea2 group");
    let ids: Vec<&str> = krea
        .checkpoints
        .iter()
        .map(|c| c.model.id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["realism_engine_krea2_v3.1", "realism_engine_krea2_v2"]
    );
    assert!(!krea.complete, "the encoder and VAE are not installed");
    // The base is installed; only the Qwen3-VL encoder and the VAE are due.
    assert_eq!(krea.missing_bytes, 5_242_467_968 + 253_806_246);
}

#[test]
fn a_base_that_is_only_findable_asks_for_a_choice_not_bytes() {
    let out = library_packages(&real_library(), FAMILIES, &Catalog::builtin());
    let klein = out
        .groups
        .iter()
        .find(|g| g.family.id == "flux2-klein-4b")
        .unwrap();
    assert!(!klein.complete);
    assert_eq!(klein.missing_bytes, 0);
    assert!(klein.base_choice_needed);
    assert!(
        klein.checkpoints.is_empty(),
        "the training folder is no base"
    );
    let note = klein.note.as_deref().unwrap_or_default();
    assert!(note.contains("training base is installed"), "{note:?}");

    let sdxl = out.groups.iter().find(|g| g.family.id == "sdxl").unwrap();
    assert!(!sdxl.base_choice_needed && sdxl.note.is_none());
}
