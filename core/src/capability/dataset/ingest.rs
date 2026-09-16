//! Stage 1: walk a dataset root folder tree. Each *immediate* subfolder of
//! the root becomes a tag/label applied to every video/image found under it
//! (recursively — a tag folder may itself have sub-folders, e.g. by shot or
//! by date). Both video files (frame-extraction candidates) and plain image
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
    /// The immediate-child-of-root folder name this file lived under.
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

/// Walk `root`: every immediate subfolder is a tag; every video/image file
/// found anywhere under that subfolder (recursively) is one [`IngestItem`].
/// Files directly inside `root` itself (no tag folder) are skipped — there is
/// no label to give them, and the whole point of the folder-per-style
/// convention is that a tag is never ambiguous.
///
/// Returns items grouped by tag, tag folders in name order and files within
/// a tag in walk order — deterministic, so extraction/filtering/captioning
/// downstream produce a stable, reproducible frame numbering.
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

    let mut items = Vec::new();
    for tag_dir in tag_dirs {
        let tag = tag_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untagged")
            .to_string();
        let mut files: Vec<PathBuf> = WalkDir::new(&tag_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .collect();
        files.sort();
        for path in files {
            if let Some(kind) = classify(&path) {
                items.push(IngestItem {
                    path,
                    tag: tag.clone(),
                    kind,
                });
            }
        }
    }
    Ok(items)
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
    fn tags_come_from_immediate_subfolders_only() {
        let tmp = tempfile::tempdir().unwrap();
        let ghibli = tmp.path().join("Ghibli");
        std::fs::create_dir_all(&ghibli).unwrap();
        touch(&ghibli, "clip.mp4");
        touch(tmp.path(), "loose.mp4"); // no tag folder -- skipped

        let items = walk_dataset_root(tmp.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].tag, "Ghibli");
        assert_eq!(items[0].kind, IngestKind::Video);
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
