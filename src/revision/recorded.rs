//! The files a change touched as Git holds them, for a check that judges the
//! index being committed or a commit being pushed rather than the working
//! tree: a file staged with `git add -p` holds lines the working tree has
//! changed since, and the changed lines Git reports are the index's. The
//! files it judges are read from here; files read only as evidence (callers,
//! copies elsewhere, context) still come from disk.
use super::{Batched, DiffEntry, Now, cat_batch, diff_entries};
use anyhow::{Result, bail, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// The largest blob kept: the most any read of a judged file asks for, as
/// `--max-file-bytes` and a local parse stop at 1 MiB.
const KEPT_BYTES: u64 = 1_048_576;

/// The files a change added, modified or renamed, as the index or a commit
/// holds them, by path relative to the root.
#[derive(Debug)]
pub struct Recorded {
    /// The repository root the paths are relative to, as the check names it.
    root: PathBuf,
    files: BTreeMap<PathBuf, Blob>,
}

/// One file as Git holds it.
#[derive(Debug)]
struct Blob {
    /// Recorded as a regular file (mode 100644 or 100755), not a symbolic
    /// link or a submodule, which a check never uploads.
    regular: bool,
    size: u64,
    /// Its bytes, when there are at most [`KEPT_BYTES`].
    bytes: Option<Vec<u8>>,
}

impl Recorded {
    /// The files the change from `base` to `now`, the index or a commit,
    /// added, modified or renamed, read by two Git processes: the diff's
    /// raw entries name each file's mode and object, and one batch reads
    /// the objects.
    pub fn load(root: &Path, base: &str, now: &Now) -> Result<Self> {
        if matches!(now, Now::WorkingTree | Now::Snapshot(_)) {
            bail!("Only the index or a commit is read from Git");
        }
        // A deleted file is not read; the change judges it deleted.
        let entries: Vec<DiffEntry> = diff_entries(root, &now.sides(base))?
            .into_iter()
            .filter(|entry| entry.status != b'D')
            .collect();
        let ids: Vec<String> = entries
            .iter()
            .filter(|e| regular(e))
            .map(|e| e.id.clone())
            .collect();
        let mut contents = cat_batch(root, &ids, KEPT_BYTES)?.into_iter();
        let mut files = BTreeMap::new();
        for entry in &entries {
            let blob = if regular(entry) {
                match contents.next() {
                    Some(Batched::Blob { size, bytes }) => Blob {
                        regular: true,
                        size,
                        bytes,
                    },
                    _ => bail!("Git could not read {} as recorded", entry.path.display()),
                }
            } else {
                Blob {
                    regular: false,
                    size: 0,
                    bytes: None,
                }
            };
            files.insert(entry.path.clone(), blob);
        }
        Ok(Self {
            root: root.to_path_buf(),
            files,
        })
    }

    /// The text of the file at `path`, as a read from disk would give it or
    /// fail; none when the change did not touch it, so it is read from disk.
    pub fn read(&self, path: &Path, limit: u64) -> Option<Result<String>> {
        let blob = self.files.get(path.strip_prefix(&self.root).ok()?)?;
        Some(blob.read(limit))
    }

    /// The size of the file at `path` as Git holds it; none when the change
    /// did not touch it.
    pub fn size(&self, path: &Path) -> Option<u64> {
        let blob = self.files.get(path.strip_prefix(&self.root).ok()?)?;
        Some(blob.size)
    }

    /// Whether the change touched `path`, and Git holds it as a regular
    /// file; none when the change did not touch it.
    pub fn regular(&self, path: &Path) -> Option<bool> {
        let blob = self.files.get(path.strip_prefix(&self.root).ok()?)?;
        Some(blob.regular)
    }

    /// The paths of the files the change touched, relative to the root.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }
}

impl Blob {
    /// The text, failing as `inventory::read_source` fails on disk.
    fn read(&self, limit: u64) -> Result<String> {
        ensure!(
            self.regular,
            "Source symlinks and special files are not uploaded"
        );
        ensure!(
            self.size <= limit,
            "File exceeds --max-file-bytes; no source was truncated or sent"
        );
        match &self.bytes {
            Some(bytes) => crate::inventory::text_of(bytes.clone()),
            None => bail!("File exceeds --max-file-bytes; no source was truncated or sent"),
        }
    }
}

/// Whether Git records the file as a regular one (mode 100644 or 100755),
/// not a symbolic link or a submodule.
fn regular(entry: &DiffEntry) -> bool {
    matches!(entry.mode.as_str(), "100644" | "100755")
}
