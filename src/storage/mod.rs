//! The writer's state under `.jevgate/`: the session lock, cached answers and
//! published reports. Reading reports without the lock is in `reports`, and
//! the answer cache's files in `cache`.
mod cache;
mod reports;

pub use cache::{CacheReader, CachedAnswer};
pub use reports::{history, read_latest, writer_active};

use super::schema::Report;
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// Reports kept in `.jevgate/history/`, by generation; older ones are removed.
const HISTORY: u64 = 64;

/// `.jevgate/.gitignore`: the local state stays out of Git, and the custom
/// question files beside it are committed.
pub(crate) const IGNORE: &str = "# JevGate's local state. Custom questions in questions/ are committed.\n*\n!questions/\n!questions/**\n";
/// What `.jevgate/.gitignore` held before custom questions: found as it was,
/// it is rewritten, since it would keep `questions/` out of Git.
const IGNORE_BEFORE: &str = "*\n";

pub struct Store {
    pub directory: PathBuf,
    /// The cache files Git tracks as the store opens, which are never read.
    tracked: std::sync::Arc<std::collections::BTreeSet<PathBuf>>,
    _lock: fs::File,
}

impl Drop for Store {
    fn drop(&mut self) {
        // Releasing only the descriptor can leave a lock held by a forked child
        // until it execs. End the writer lease when its owning Store ends.
        let _ = self._lock.unlock();
    }
}

/// Whether a write waits until its bytes are on disk.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Durability {
    /// Answers paid for, and reports.
    Synced,
    /// A copy of answers another cache file still holds, made again if a
    /// crash loses it.
    Unsynced,
}

/// Create `path` as a directory, or accept an existing real (non-symlink) one.
pub(crate) fn real_directory(path: &Path, message: &'static str) -> Result<()> {
    if path.exists() || path.is_symlink() {
        ensure!(!path.is_symlink() && path.is_dir(), message);
    } else {
        fs::create_dir(path)?;
    }
    Ok(())
}

/// Write `.jevgate/.gitignore`, or rewrite the one JevGate wrote before
/// custom questions; one a person edited is left as it is.
fn ignore_state(directory: &Path) -> Result<()> {
    let path = directory.join(".gitignore");
    if !path.exists() {
        let created = OpenOptions::new().write(true).create_new(true).open(&path);
        match created {
            Ok(mut file) => file.write_all(IGNORE.as_bytes())?,
            // Another JevGate process wrote it first.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    } else if written_before_questions(&path) {
        atomic(&path, IGNORE.as_bytes(), Durability::Synced)?;
    }
    Ok(())
}

/// Whether the `.gitignore` at `path` is the one JevGate wrote before
/// custom questions, which the next check that writes its state rewrites.
pub(crate) fn written_before_questions(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| text == IGNORE_BEFORE)
}

/// Hold the session lock for the life of the store, recording this process.
fn lock_session(path: &Path) -> Result<fs::File> {
    ensure!(!path.is_symlink(), "Session lock must not be a symlink");
    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .context("Another JevGate session owns latest.json")?;
    file.set_len(0)?;
    writeln!(file, "{}", std::process::id())?;
    Ok(file)
}

/// `.jevgate/` under `root`, created with a `.gitignore` that ignores the
/// local state and keeps custom questions tracked (`ignore_state`). Taking
/// no lock, it is where writers other than the session keep state.
pub fn state_directory(root: &Path) -> Result<PathBuf> {
    let directory = root.join(".jevgate");
    real_directory(&directory, "Jev storage must be a real directory")?;
    ignore_state(&directory)?;
    Ok(directory)
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let directory = state_directory(root)?;
        real_directory(
            &directory.join("cache"),
            "Jev storage must be a real directory",
        )?;
        real_directory(
            &cache::answers_directory(&directory),
            "Jev storage must be a real directory",
        )?;
        let lock = lock_session(&directory.join("session.lock"))?;
        real_directory(
            &directory.join("history"),
            "History must be a real directory",
        )?;
        Ok(Self {
            tracked: std::sync::Arc::new(cache::tracked(&directory)),
            directory,
            _lock: lock,
        })
    }

    /// Atomically replace a small state file directly under `.jevgate/`.
    pub fn write(&self, name: &str, bytes: &[u8]) -> Result<()> {
        atomic(&self.directory.join(name), bytes, Durability::Synced)
    }

    pub fn publish_html(&self, report: &Report) -> Result<()> {
        atomic(
            &self.directory.join("report.html"),
            crate::html_report::render(report)?.as_bytes(),
            Durability::Synced,
        )
    }

    pub fn publish(&self, report: &Report) -> Result<()> {
        // Partial repository reports are persisted repeatedly. Whitespace adds
        // substantial I/O but no evidence; CLI JSON formatting stays separate.
        let bytes = serde_json::to_vec(report)?;
        atomic(
            &self
                .directory
                .join("history")
                .join(format!("{}.json", report.generation)),
            &bytes,
            Durability::Synced,
        )?;
        atomic(
            &self.directory.join("latest.json"),
            &bytes,
            Durability::Synced,
        )?;
        if report.generation > HISTORY {
            let old = self
                .directory
                .join("history")
                .join(format!("{}.json", report.generation - HISTORY));
            if old.is_file() {
                fs::remove_file(old)?;
            }
        }
        Ok(())
    }
}

/// Replace `path` with `bytes` through a temporary file, so a reader sees
/// the old file or the new one, never part of either.
pub(crate) fn atomic(path: &Path, bytes: &[u8], durability: Durability) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        if durability == Durability::Synced {
            file.sync_all()?;
        }
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.context("Cannot publish Jev state atomically")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_is_ignored_and_custom_questions_stay_tracked() {
        let project = crate::tests::Project::new();
        let ignore = project.0.join(".jevgate/.gitignore");
        let read = || fs::read_to_string(&ignore).unwrap();
        drop(Store::open(&project.0).unwrap());
        assert_eq!(read(), IGNORE);
        fs::write(&ignore, IGNORE_BEFORE).unwrap();
        drop(Store::open(&project.0).unwrap());
        assert_eq!(read(), IGNORE, "the old generated file is rewritten");
        fs::write(&ignore, "*\n!notes.md\n").unwrap();
        drop(Store::open(&project.0).unwrap());
        assert_eq!(read(), "*\n!notes.md\n", "an edited one is kept");
    }

    #[test]
    fn large_published_reports_remain_readable_in_latest_and_history() {
        let project = crate::tests::Project::new();
        let mut report = crate::evaluate::snapshot(
            &[],
            &Default::default(),
            &crate::tests::args(),
            crate::evaluate::SnapshotContext {
                root: &project.0,
                generation: 1,
                requests: 0,
            },
        );
        report
            .errors
            .push("large repository diagnostic ".repeat(700_000));
        let store = Store::open(&project.0).unwrap();
        store.publish(&report).unwrap();
        assert!(
            fs::metadata(store.directory.join("latest.json"))
                .unwrap()
                .len()
                > 16 * 1024 * 1024
        );
        assert_eq!(read_latest(&project.0).unwrap().errors, report.errors);
        assert_eq!(history(&project.0, 0, 1).unwrap()[0]["generation"], 1);
        #[cfg(unix)]
        {
            fs::remove_file(store.directory.join("latest.json")).unwrap();
            std::os::unix::fs::symlink(
                store.directory.join("history/1.json"),
                store.directory.join("latest.json"),
            )
            .unwrap();
            assert!(read_latest(&project.0).is_err());
            fs::rename(
                store.directory.join("history/1.json"),
                store.directory.join("saved.json"),
            )
            .unwrap();
            std::os::unix::fs::symlink(
                store.directory.join("saved.json"),
                store.directory.join("history/1.json"),
            )
            .unwrap();
            assert!(history(&project.0, 0, 1).is_err());
        }
    }

    #[test]
    fn writer_lease_ends_even_when_a_child_retains_the_open_description() {
        let project = crate::tests::Project::new();
        let first = Store::open(&project.0).unwrap();
        // A fork inherits this same open-file description until exec/exit.
        let inherited = first._lock.try_clone().unwrap();
        assert!(Store::open(&project.0).is_err());
        drop(first);
        let second = Store::open(&project.0).unwrap();
        drop(inherited);
        assert!(
            Store::open(&project.0).is_err(),
            "Old descriptor cannot release a new writer's lease"
        );
        drop(second);
        assert!(Store::open(&project.0).is_ok());
    }

    #[test]
    fn storage_has_single_writer_and_releases_lock() {
        let project = crate::tests::Project::new();
        let store = Store::open(&project.0).unwrap();
        assert!(Store::open(&project.0).is_err());
        drop(store);
        assert!(Store::open(&project.0).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn storage_rejects_symlinked_state_directory() {
        let project = crate::tests::Project::new();
        let outside = crate::tests::Project::new();
        std::os::unix::fs::symlink(&outside.0, project.0.join(".jevgate")).unwrap();
        assert!(Store::open(&project.0).is_err());
        assert!(!outside.0.join("jev").exists());
    }
}
