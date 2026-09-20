//! Groups `media_retention` and `media_orphans`: a flat file in the outputs
//! folder, deleted only when the scan — re-run right before, with the
//! retention rule, the job rows and the in-place frames read fresh — still
//! offers its name. A client-sent path is never used: the id is a file name
//! and the file must sit directly in the outputs folder.

use std::collections::HashSet;

use super::{blocking, canonical_dir, entry, not_offered, EntryResult, ScanContext, Tally};
use crate::Result;

pub(super) async fn apply(
    ctx: &ScanContext,
    group: &str,
    offered: HashSet<String>,
    ids: &[String],
    dry_run: bool,
) -> Result<Vec<EntryResult>> {
    let outputs = ctx.paths.outputs_dir();
    let group = group.to_string();
    let ids = ids.to_vec();
    blocking(move || {
        let root = canonical_dir(&outputs);
        ids.iter()
            .map(|id| {
                if !offered.contains(id) {
                    return not_offered(&group, id);
                }
                let Some(root) = root.as_deref() else {
                    return not_offered(&group, id);
                };
                let mut tally = Tally::default();
                tally.remove_flat_file(root, &outputs.join(id), dry_run);
                entry(&group, id, id, tally, 0)
            })
            .collect()
    })
    .await
}
