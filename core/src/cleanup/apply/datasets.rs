//! Groups `discarded_frames`, `unclaimed_dataset_folders` and
//! `missing_frame_rows`. Discarded frames go through
//! `housekeeping::cleanup` (the dataset guard decides file by file);
//! unclaimed folders through the guard's unclaimed rule, re-evaluated now;
//! missing rows only when their file is still not found now. The busy check
//! ran before anything was touched (see [`super::refuse_if_busy`]).

use std::collections::HashMap;
use std::path::PathBuf;

use crate::capability::dataset::housekeeping;
use crate::db::Database;
use crate::Result;

use super::super::scan::datasets::is_missing;
use super::super::scan::file_name;
use super::{
    blocking, display, entry, not_offered, refused, remove_tree, EntryResult, ScanContext, Tally,
};

const DISCARDED: &str = "discarded_frames";
const UNCLAIMED: &str = "unclaimed_dataset_folders";
const MISSING: &str = "missing_frame_rows";

/// Per dataset: the guard's plan (the exact files) and, in a real run,
/// `housekeeping::cleanup`.
pub(super) async fn discarded(
    db: &Database,
    ctx: &ScanContext,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let roots = ctx.data_roots();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(dataset) = db.datasets().get(id).await? else {
            out.push(not_offered(DISCARDED, id));
            continue;
        };
        let Some(plan) = housekeeping::discarded_plan(db, &roots, id).await? else {
            out.push(not_offered(DISCARDED, id));
            continue;
        };
        let paths: Vec<String> = plan.files.iter().map(|(p, _)| display(p)).collect();
        let mut result = EntryResult {
            group: DISCARDED.into(),
            id: id.clone(),
            label: dataset.name.clone(),
            paths,
            ..EntryResult::default()
        };
        if dry_run {
            result.files = plan.files.len() as u64;
            result.bytes = plan.files.iter().map(|(_, b)| b).sum();
            result.rows = plan.frames;
        } else {
            match housekeeping::cleanup(db, &roots, id, false).await? {
                Some(s) => {
                    result.files = s.deleted_files;
                    result.bytes = s.bytes;
                    result.rows = s.frames;
                    result.skipped = s.skipped_files;
                }
                None => result = not_offered(DISCARDED, id),
            }
        }
        out.push(result);
    }
    Ok(out)
}

/// The folders directly under the datasets root that no dataset claims —
/// the guard's rule, evaluated now — deleted as a whole.
pub(super) async fn unclaimed(
    db: &Database,
    ctx: &ScanContext,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let found = housekeeping::unclaimed_work_folders(db, &ctx.data_roots()).await?;
    let by_name: HashMap<String, PathBuf> = found
        .folders
        .iter()
        .map(|p| (file_name(p), p.clone()))
        .collect();
    let ctx = ctx.clone();
    let ids = ids.to_vec();
    blocking(move || {
        ids.iter()
            .map(|id| {
                let Some(folder) = by_name.get(id) else {
                    return not_offered(UNCLAIMED, id);
                };
                if let Some(why) = ctx.whole_folder_refusal(folder, None) {
                    return refused(UNCLAIMED, id, id, folder, &why);
                }
                let mut tally = Tally::default();
                remove_tree(&mut tally, folder, dry_run);
                entry(UNCLAIMED, id, id, tally, 0)
            })
            .collect()
    })
    .await
}

/// Per dataset: the rows whose file is still not found right now, removed
/// through the frame repo. Zero bytes freed.
pub(super) async fn missing_rows(
    db: &Database,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(dataset) = db.datasets().get(id).await? else {
            out.push(not_offered(MISSING, id));
            continue;
        };
        let frames = db.dataset_frames().list_for_dataset(id).await?;
        let gone: Vec<String> = blocking(move || {
            frames
                .into_iter()
                .filter(|f| is_missing(&f.frame_path))
                .map(|f| f.id)
                .collect()
        })
        .await?;
        let rows = if dry_run {
            gone.len() as u64
        } else {
            db.dataset_frames().delete_many(id, &gone).await?
        };
        out.push(EntryResult {
            group: MISSING.into(),
            id: id.clone(),
            label: dataset.name,
            rows,
            ..EntryResult::default()
        });
    }
    Ok(out)
}
