//! Reading published reports without taking the writer's lock: the latest
//! report, its history for watchers, and whether a writer is running.
use super::HISTORY;
use crate::schema::Report;
use anyhow::{Result, ensure};
use std::{
    fs::{self, OpenOptions},
    io::BufReader,
    path::Path,
};

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
    ensure!(current.saturating_sub(since) <= HISTORY, "History expired");
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
