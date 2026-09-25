//! The writer's state under `.jevgate/`: the session lock, cached answers and
//! published reports. Reading reports without the lock is in `reports`.
mod reports;

pub use reports::{history, read_latest, writer_active};

use super::schema::{Report, now};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// A cached answer larger than this is ignored and asked again.
const CACHE_ENTRY_BYTES: u64 = 1_048_576;

/// Reports kept in `.jevgate/history/`, by generation; older ones are removed.
const HISTORY: u64 = 64;

pub struct Store {
    pub directory: PathBuf,
    _lock: fs::File,
}

impl Drop for Store {
    fn drop(&mut self) {
        // Releasing only the descriptor can leave a lock held by a forked child
        // until it execs. End the writer lease when its owning Store ends.
        let _ = self._lock.unlock();
    }
}

#[derive(Serialize, Deserialize)]
struct Cache {
    request_hash: String,
    created_at: u64,
    response: Value,
}

/// Create `path` as a directory, or accept an existing real (non-symlink) one.
fn real_directory(path: &Path, message: &'static str) -> Result<()> {
    if path.exists() || path.is_symlink() {
        ensure!(!path.is_symlink() && path.is_dir(), message);
    } else {
        fs::create_dir(path)?;
    }
    Ok(())
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

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let directory = root.join(".jevgate");
        real_directory(&directory, "Jev storage must be a real directory")?;
        real_directory(
            &directory.join("cache"),
            "Jev storage must be a real directory",
        )?;
        let ignore = directory.join(".gitignore");
        if !ignore.exists() {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(ignore)?;
            file.write_all(b"*\n")?;
        }
        let lock = lock_session(&directory.join("session.lock"))?;
        real_directory(
            &directory.join("history"),
            "History must be a real directory",
        )?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }

    /// `ttl` is `None` for answers that never expire (a pinned model version).
    pub fn load(&self, hash: &str, ttl: Option<u64>) -> Option<(Value, u64)> {
        load_entry(&self.directory, hash, ttl)
    }

    pub fn save(&self, hash: &str, response: &Value, created_at: u64) -> Result<()> {
        let entry = Cache {
            request_hash: hash.into(),
            response: response.clone(),
            created_at,
        };
        atomic(
            &self.directory.join("cache").join(format!("{hash}.json")),
            &serde_json::to_vec(&entry)?,
        )
    }

    /// Atomically replace a small state file directly under `.jevgate/`.
    pub fn write(&self, name: &str, bytes: &[u8]) -> Result<()> {
        atomic(&self.directory.join(name), bytes)
    }

    pub fn publish_html(&self, report: &Report) -> Result<()> {
        atomic(
            &self.directory.join("report.html"),
            crate::html_report::render(report)?.as_bytes(),
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
        )?;
        atomic(&self.directory.join("latest.json"), &bytes)?;
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

fn load_entry(directory: &Path, hash: &str, ttl: Option<u64>) -> Option<(Value, u64)> {
    let path = directory.join("cache").join(format!("{hash}.json"));
    if path.is_symlink() {
        return None;
    }
    let entry: Cache =
        serde_json::from_str(&crate::inventory::read_source(&path, CACHE_ENTRY_BYTES).ok()?)
            .ok()?;
    let age = now().checked_sub(entry.created_at)?;
    (entry.request_hash == hash && ttl.is_none_or(|ttl| age < ttl))
        .then_some((entry.response, entry.created_at))
}

/// A cached answer read without opening the store: no lock is taken and no
/// directory is created, so a dry run stays free of saved state.
pub fn peek(root: &Path, hash: &str, ttl: Option<u64>) -> Option<(Value, u64)> {
    let directory = root.join(".jevgate");
    if directory.is_symlink() || !directory.is_dir() {
        return None;
    }
    load_entry(&directory, hash, ttl)
}

fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
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
        file.sync_all()?;
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
