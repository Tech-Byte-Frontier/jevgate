use super::schema::{Report, now};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

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

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let mut directory = root.to_path_buf();
        for part in [".jevgate", "cache"] {
            directory.push(part);
            if directory.exists() || directory.is_symlink() {
                ensure!(
                    !directory.is_symlink() && directory.is_dir(),
                    "Jev storage must be a real directory"
                );
            } else {
                fs::create_dir(&directory)?;
            }
        }
        directory.pop();
        let ignore = directory.join(".gitignore");
        if !ignore.exists() {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(ignore)?;
            file.write_all(b"*\n")?;
        }
        let lock = directory.join("session.lock");
        ensure!(!lock.is_symlink(), "Session lock must not be a symlink");
        let mut file = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .truncate(false)
            .open(&lock)?;
        file.try_lock()
            .context("Another JevGate session owns latest.json")?;
        file.set_len(0)?;
        writeln!(file, "{}", std::process::id())?;
        let history = directory.join("history");
        if !history.exists() {
            fs::create_dir(&history)?;
        }
        ensure!(
            !history.is_symlink() && history.is_dir(),
            "History must be a real directory"
        );
        let store = Self {
            directory,
            _lock: file,
        };
        Ok(store)
    }

    /// `ttl` is `None` for answers that never expire (a pinned model version).
    pub fn load(&self, hash: &str, ttl: Option<u64>) -> Option<(Value, u64)> {
        let path = self.directory.join("cache").join(format!("{hash}.json"));
        if path.is_symlink() {
            return None;
        }
        let entry: Cache =
            serde_json::from_str(&crate::inventory::read_source(&path, 1_048_576).ok()?).ok()?;
        let age = now().checked_sub(entry.created_at)?;
        (entry.request_hash == hash && ttl.is_none_or(|ttl| age < ttl))
            .then_some((entry.response, entry.created_at))
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
        if report.generation > 64 {
            let old = self
                .directory
                .join("history")
                .join(format!("{}.json", report.generation - 64));
            if old.is_file() {
                fs::remove_file(old)?;
            }
        }
        Ok(())
    }
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

pub fn read_latest(root: &Path) -> Result<Report> {
    let path = root.join(".jevgate/latest.json");
    ensure!(
        !root.join(".jevgate").is_symlink() && !path.is_symlink(),
        "State must not use symlinks"
    );
    let report = read_report(&path)?;
    ensure!(
        report.schema_version == crate::schema::SCHEMA_VERSION && report.root == root,
        "Incompatible report"
    );
    Ok(report)
}

fn read_report(path: &Path) -> Result<Report> {
    ensure!(
        path.canonicalize()? == path && fs::symlink_metadata(path)?.is_file(),
        "Report paths must be regular files without symlinks"
    );
    // Reports aggregate a whole repository and can exceed any one source-file
    // budget. Stream JSON instead of imposing a limit the writer does not have.
    Ok(serde_json::from_reader(BufReader::new(fs::File::open(
        path,
    )?))?)
}

pub fn writer_active(root: &Path) -> bool {
    let lock = root.join(".jevgate/session.lock");
    if lock.is_symlink() {
        return false;
    }
    let Ok(file) = OpenOptions::new().read(true).write(true).open(lock) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(_) => false,
    }
}

pub fn history(root: &Path, since: u64, current: u64) -> Result<Vec<serde_json::Value>> {
    ensure!(current.saturating_sub(since) <= 64, "History expired");
    let mut values = Vec::new();
    for generation in since.saturating_add(1)..=current {
        let path = root.join(format!(".jevgate/history/{generation}.json"));
        let report = read_report(&path)?;
        ensure!(
            report.generation == generation && report.root == root,
            "Incompatible history"
        );
        values.push(serde_json::json!({"generation":generation,"changes":report.changes}));
    }
    Ok(values)
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
}
