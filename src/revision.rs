//! Changed-file selection. Git never executes external diff helpers.
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

pub struct Changes {
    pub revision: String,
    /// Current path -> previous path; None denotes a new or untracked file.
    pub paths: BTreeMap<PathBuf, Option<PathBuf>>,
    pub deleted: Vec<PathBuf>,
}

fn git_path<'a>(fields: &mut impl Iterator<Item = &'a [u8]>, missing: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(std::str::from_utf8(
        fields.next().with_context(|| missing.to_string())?,
    )?))
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .context("Cannot run Git")?;
    ensure!(
        output.status.success(),
        "Git {} failed: {}",
        args.first().unwrap_or(&""),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    ensure!(
        output.stdout.len() <= 16 * 1024 * 1024,
        "Git change metadata exceeds 16 MiB"
    );
    Ok(output.stdout)
}

type ChangedPaths = (BTreeMap<PathBuf, Option<PathBuf>>, Vec<PathBuf>);

/// Changed paths since `revision`, each with its previous path (none when
/// added), and deleted paths. Renames keep their source; conflicts stop the review.
fn tracked_changes(root: &Path, revision: &str) -> Result<ChangedPaths> {
    let bytes = git(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--name-status",
            "-z",
            revision,
            "--",
        ],
    )?;
    let mut fields = bytes.split(|b| *b == 0).filter(|s| !s.is_empty());
    let mut paths = BTreeMap::new();
    let mut deleted = Vec::new();
    while let Some(status) = fields.next() {
        let old = git_path(&mut fields, "Missing Git path")?;
        match status.first() {
            Some(b'R') => {
                let new = git_path(&mut fields, "Missing Git rename target")?;
                paths.insert(new, Some(old));
            }
            Some(b'D') => deleted.push(old),
            Some(b'A') => {
                paths.insert(old, None);
            }
            Some(b'U') => {
                anyhow::bail!("Resolve Git conflict in {} before reviewing", old.display())
            }
            _ => {
                paths.insert(old.clone(), Some(old));
            }
        }
    }
    Ok((paths, deleted))
}

/// The commit to compare with: where `revision` and HEAD diverged, as a pull
/// request diff does, so changes made only on the base branch are not reviewed.
pub fn resolve(root: &Path, revision: &str) -> Result<String> {
    let bytes = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )
    .with_context(|| {
        format!("Cannot find revision {revision}; in CI, fetch it (for example fetch-depth: 0)")
    })?;
    let commit = commit_id(&bytes)?;
    let fork = git(root, &["merge-base", &commit, "HEAD"]).with_context(|| {
        format!(
            "{revision} and HEAD share no history; in a shallow clone, fetch full history (fetch-depth: 0)"
        )
    })?;
    commit_id(&fork)
}

fn commit_id(bytes: &[u8]) -> Result<String> {
    let commit = std::str::from_utf8(bytes)?.trim();
    ensure!(
        [40, 64].contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "Git did not resolve a commit"
    );
    Ok(commit.to_owned())
}

impl Changes {
    pub fn load(root: &Path, base: &str) -> Result<Self> {
        let revision = resolve(root, base)?;
        let (mut paths, deleted) = tracked_changes(root, &revision)?;
        let untracked = git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        for name in untracked.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            paths
                .entry(PathBuf::from(std::str::from_utf8(name)?))
                .or_insert(None);
        }
        Ok(Self {
            revision,
            paths,
            deleted,
        })
    }
}
