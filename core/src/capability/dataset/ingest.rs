//! Stage 1: walk a dataset root folder tree. Each *immediate* subfolder of
//! the root becomes a tag/label applied to every video/image found under it
//! (recursively — a tag folder may itself have sub-folders, e.g. by shot or
//! by date); files lying directly in the root are tagged with the root
//! folder's own name. Both video files (frame-extraction candidates) and plain image
//! files (already-a-frame, the user's own "any kind of data" folders) are
//! collected — see the module doc on [`super`] for why both matter.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::Result;

use super::dataset_err;

/// Extensions ffmpeg is asked to decode. Lowercase, without the dot.
pub const VIDEO_EXTS: [&str; 1] = ["mp4"];
/// Extensions treated as an already-extracted still frame.
pub const IMAGE_EXTS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestKind {
    Video,
    Image,
}

/// One media file found under a tag folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestItem {
    pub path: PathBuf,
    /// The immediate-child-of-root folder name this file lived under, or the
    /// root folder's own name for a file lying directly in the root.
    pub tag: String,
    pub kind: IngestKind,
}

fn ext_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

fn classify(path: &Path) -> Option<IngestKind> {
    let ext = ext_lower(path)?;
    if VIDEO_EXTS.contains(&ext.as_str()) {
        Some(IngestKind::Video)
    } else if IMAGE_EXTS.contains(&ext.as_str()) {
        Some(IngestKind::Image)
    } else {
        None
    }
}

/// Tag used when a folder's name cannot be read as UTF-8 (or a drive root
/// like `D:\` has no name at all).
const FALLBACK_TAG: &str = "untagged";

fn folder_tag(dir: &Path) -> String {
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(FALLBACK_TAG)
        .to_string()
}

/// Every video/image file under `dir` (down to `max_depth`), sorted by path,
/// as items carrying `tag`.
fn media_items(dir: &Path, max_depth: usize, tag: &str) -> Vec<IngestItem> {
    let mut files: Vec<PathBuf> = WalkDir::new(dir)
        .min_depth(1)
        .max_depth(max_depth)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            classify(&path).map(|kind| IngestItem {
                path,
                tag: tag.to_string(),
                kind,
            })
        })
        .collect()
}

/// Walk `root` for video/image files, one [`IngestItem`] each:
///
/// - files directly inside `root` are tagged with the root folder's own name
///   (`"untagged"` if it has none) — so "pick a folder of videos" just works;
/// - every immediate subfolder is a tag, applied to every file found anywhere
///   under it (recursively) — the structured, folder-per-style layout.
///
/// A mixed root yields both. Extensions match case-insensitively.
///
/// Returns root-level files first (sorted by path), then the tag folders in
/// name order with files within a tag sorted by path — deterministic, so
/// extraction/filtering/captioning downstream produce a stable, reproducible
/// frame numbering.
pub fn walk_dataset_root(root: &Path) -> Result<Vec<IngestItem>> {
    if !root.is_dir() {
        return Err(dataset_err(format!(
            "dataset root not found or not a folder: {}",
            root.display()
        )));
    }

    let mut tag_dirs: Vec<PathBuf> = WalkDir::new(root)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();
    tag_dirs.sort();

    let root_items = media_items(root, 1, &folder_tag(root));
    let tagged_items = tag_dirs
        .iter()
        .flat_map(|dir| media_items(dir, usize::MAX, &folder_tag(dir)));
    Ok(root_items.into_iter().chain(tagged_items).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    #[test]
    fn rejects_a_root_that_does_not_exist() {
        let err = walk_dataset_root(Path::new("C:\\definitely\\not\\real")).unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn tags_come_from_immediate_subfolders() {
        let tmp = tempfile::tempdir().unwrap();
        let ghibli = tmp.path().join("Ghibli");
        std::fs::create_dir_all(&ghibli).unwrap();
        touch(&ghibli, "clip.mp4");

        let items = walk_dataset_root(tmp.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].tag, "Ghibli");
        assert_eq!(items[0].kind, IngestKind::Video);
    }

    /// A root that holds only media files (the `D:\Data\Test` shape: one
    /// `.mp4`, no subfolders) is a dataset tagged with the root's own name.
    #[test]
    fn files_directly_in_the_root_are_tagged_with_the_root_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Test");
        std::fs::create_dir_all(&root).unwrap();
        touch(&root, "20250111-_1.mp4");
        touch(&root, "b.MP4");
        touch(&root, "a.JPG");
        touch(&root, "notes.txt"); // not media -- ignored

        let items = walk_dataset_root(&root).unwrap();
        let names: Vec<&str> = items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["20250111-_1.mp4", "a.JPG", "b.MP4"],
            "sorted by path"
        );
        assert!(items.iter().all(|i| i.tag == "Test"), "{items:?}");
        assert_eq!(items[0].kind, IngestKind::Video);
        assert_eq!(items[1].kind, IngestKind::Image);
        assert_eq!(items[2].kind, IngestKind::Video);
    }

    #[test]
    fn a_mixed_root_yields_root_files_first_then_tag_folders_in_name_order() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Mixed");
        for tag in ["B", "A"] {
            std::fs::create_dir_all(root.join(tag)).unwrap();
            touch(&root.join(tag), "clip.mp4");
        }
        touch(&root, "loose.mp4");

        let items = walk_dataset_root(&root).unwrap();
        let tags: Vec<&str> = items.iter().map(|i| i.tag.as_str()).collect();
        assert_eq!(tags, ["Mixed", "A", "B"]);
        assert_eq!(items[0].path, root.join("loose.mp4"));
    }

    #[test]
    fn an_empty_root_yields_no_items() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(walk_dataset_root(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn recurses_into_nested_subfolders_of_a_tag() {
        let tmp = tempfile::tempdir().unwrap();
        let shot1 = tmp.path().join("Anime").join("shot1");
        std::fs::create_dir_all(&shot1).unwrap();
        touch(&shot1, "a.mp4");
        touch(&shot1, "b.png");

        let items = walk_dataset_root(tmp.path()).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.tag == "Anime"));
    }

    #[test]
    fn classifies_videos_and_images_and_ignores_unknown_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let tag = tmp.path().join("Style");
        std::fs::create_dir_all(&tag).unwrap();
        touch(&tag, "a.mp4");
        touch(&tag, "b.jpg");
        touch(&tag, "c.jpeg");
        touch(&tag, "d.webp");
        touch(&tag, "e.png");
        touch(&tag, "notes.txt");

        let items = walk_dataset_root(tmp.path()).unwrap();
        assert_eq!(items.len(), 5, "{items:?}");
        assert_eq!(
            items.iter().filter(|i| i.kind == IngestKind::Video).count(),
            1
        );
        assert_eq!(
            items.iter().filter(|i| i.kind == IngestKind::Image).count(),
            4
        );
    }

    #[test]
    fn is_deterministic_across_repeated_walks() {
        let tmp = tempfile::tempdir().unwrap();
        let tag_a = tmp.path().join("A");
        let tag_b = tmp.path().join("B");
        std::fs::create_dir_all(&tag_a).unwrap();
        std::fs::create_dir_all(&tag_b).unwrap();
        touch(&tag_a, "1.png");
        touch(&tag_a, "2.png");
        touch(&tag_b, "1.png");

        let first = walk_dataset_root(tmp.path()).unwrap();
        let second = walk_dataset_root(tmp.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first[0].tag, "A");
        assert_eq!(first.last().unwrap().tag, "B");
    }

    #[test]
    fn empty_tag_folder_yields_no_items_but_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Empty")).unwrap();
        assert!(walk_dataset_root(tmp.path()).unwrap().is_empty());
    }
}
