//! The pure half of the dataset-wide dedup: decode and score every frame,
//! then group near-duplicates. No database, no async — the caller runs it on
//! `spawn_blocking` and writes the marks.

use std::path::PathBuf;

use super::super::filter;

pub(super) struct DedupPlan {
    pub(super) duplicates: Vec<String>,
    pub(super) groups: u64,
    pub(super) unreadable: u64,
}

/// Most decoding threads the dedup uses — decoding full-size stills is the
/// whole cost (~9 ms per 1280x720 PNG in a release build), and it spreads
/// perfectly across cores.
const MAX_DEDUP_THREADS: usize = 8;

/// Hash + sharpness of every item, in item order (`None` = unreadable),
/// decoded on up to [`MAX_DEDUP_THREADS`] scoped threads.
fn score_all(items: &[(String, PathBuf)]) -> Vec<Option<(Vec<u8>, f64)>> {
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .clamp(1, MAX_DEDUP_THREADS);
    let chunk = items.len().div_ceil(threads).max(1);
    let score = |(_, path): &(String, PathBuf)| {
        filter::phash_and_sharpness(path)
            .ok()
            .map(|(hash, sharpness)| (hash.as_bytes().to_vec(), sharpness))
    };
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| s.spawn(move || part.iter().map(score).collect::<Vec<_>>()))
            .collect();
        // A panicking decoder thread counts its whole chunk as unreadable
        // rather than taking the request down.
        handles
            .into_iter()
            .zip(items.chunks(chunk))
            .flat_map(|(h, part)| h.join().unwrap_or_else(|_| vec![None; part.len()]))
            .collect()
    })
}

/// Greedy and deterministic: frames are visited sharpest first (ties: the
/// earlier frame); each joins the first keeper within `threshold` or becomes
/// a keeper itself, so every group's keeper is its sharpest member and two
/// different pictures can never be chained together through small steps.
pub(super) fn plan_dedup(items: &[(String, PathBuf)], threshold: u32) -> DedupPlan {
    let mut unreadable = 0u64;
    let mut scored: Vec<(usize, &str, Vec<u8>, f64)> = Vec::with_capacity(items.len());
    for (i, ((id, _), score)) in items.iter().zip(score_all(items)).enumerate() {
        match score {
            Some((hash, sharpness)) => scored.push((i, id, hash, sharpness)),
            None => unreadable += 1,
        }
    }
    // Sharpest first; equal sharpness keeps the earlier frame.
    scored.sort_by(|a, b| b.3.total_cmp(&a.3).then(a.0.cmp(&b.0)));

    let mut keepers: Vec<(Vec<u8>, u64)> = Vec::new();
    let mut duplicates = Vec::new();
    for (_, id, hash, _) in scored {
        match keepers
            .iter_mut()
            .find(|(k, _)| hamming(k, &hash) <= threshold)
        {
            Some((_, members)) => {
                *members += 1;
                duplicates.push(id.to_string());
            }
            None => keepers.push((hash, 0)),
        }
    }
    DedupPlan {
        duplicates,
        groups: keepers.iter().filter(|(_, m)| *m > 0).count() as u64,
        unreadable,
    }
}

/// Bit distance of two hashes from the same hasher (equal length); a length
/// mismatch counts as "very different".
fn hamming(a: &[u8], b: &[u8]) -> u32 {
    if a.len() != b.len() {
        return u32::MAX;
    }
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}
