//! What a change against a Git revision holds: the changed and deleted
//! files, and the lines of each file the change touched. Git never executes
//! external diff helpers.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
};

pub struct Changes {
    pub revision: String,
    /// Current path -> previous path; None denotes a new or untracked file.
    pub paths: BTreeMap<PathBuf, Option<PathBuf>>,
    pub deleted: Vec<PathBuf>,
    /// The lines changed in each judged file that existed at the revision,
    /// once read by [`Changes::with_lines`].
    pub lines: BTreeMap<PathBuf, Lines>,
}

/// The lines of one file a change touched: those it added or modified, and
/// the places it removed lines from without adding any.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Lines {
    /// Added or modified lines of the current file, as inclusive ranges in order.
    pub changed: Vec<(usize, usize)>,
    /// Lines removed with nothing in their place sit after each of these
    /// current lines (0: before the first line).
    pub removed_after: Vec<usize>,
}

impl Lines {
    /// Whether the change touched lines `start..=end`: it changed one of
    /// them, or removed lines between two of them. A removal just before
    /// the first line or after the last is not counted: without the old
    /// file it belongs as much to the code beside it, and counting it would
    /// judge the neighbours of every deleted function.
    pub fn touch(&self, start: usize, end: usize) -> bool {
        self.changed.iter().any(|&(a, b)| a <= end && start <= b)
            || self
                .removed_after
                .iter()
                .any(|&after| start <= after && after < end)
    }

    fn add_hunk(&mut self, start: usize, count: usize) {
        if count == 0 {
            self.removed_after.push(start);
        } else {
            self.changed.push((start, start + count - 1));
        }
    }
}

/// What a change did to one file, when only what it touched is judged.
#[derive(Clone, Debug)]
pub struct FileChange {
    pub lines: Lines,
    /// The file as the base revision holds it; none for a document the
    /// change left alone, judged only for the paths it removed.
    before: Option<Before>,
    /// Paths the change deleted or renamed away.
    pub removed: Arc<BTreeSet<PathBuf>>,
}

/// A file at the base revision, read from Git the first time a file-level
/// rule asks for it: a watch poll collects files every 250 ms, and only an
/// evaluation needs the old text.
#[derive(Clone, Debug)]
struct Before {
    root: PathBuf,
    revision: String,
    path: PathBuf,
    text: OnceLock<Option<String>>,
}

impl FileChange {
    /// A document the change left alone, judged for the paths it removed.
    pub fn unchanged(removed: Arc<BTreeSet<PathBuf>>) -> Self {
        Self {
            lines: Lines::default(),
            before: None,
            removed,
        }
    }

    /// Whether the change left this document alone: it was selected only
    /// for naming a path the change removed.
    pub fn left_alone(&self) -> bool {
        self.before.is_none()
    }

    /// Whether the change adds one of `members`, each a name and the line it
    /// starts on, that the file at the base revision lacks; `known` gives that
    /// version's names from its path and text. A member the change added
    /// starts on a line it added, so the base version is read and parsed only
    /// when one does: most changes edit bodies. A base version Git cannot
    /// give as text counts as lacking every name, so the rule is asked.
    pub fn adds<'a>(
        &self,
        members: impl Iterator<Item = (&'a str, usize)>,
        known: impl FnOnce(&Path, &str) -> BTreeSet<String>,
    ) -> bool {
        let starts_changed = |line: usize| {
            self.lines
                .changed
                .iter()
                .any(|&(a, b)| a <= line && line <= b)
        };
        let fresh: Vec<&str> = members
            .filter(|&(_, line)| starts_changed(line))
            .map(|(name, _)| name)
            .collect();
        if fresh.is_empty() {
            return false;
        }
        let Some((path, text)) = self.before() else {
            return true;
        };
        let known = known(path, text);
        fresh.iter().any(|name| !known.contains(*name))
    }

    fn before(&self) -> Option<(&Path, &str)> {
        let before = self.before.as_ref()?;
        let text = before
            .text
            .get_or_init(|| show(&before.root, &before.revision, &before.path).ok());
        Some((before.path.as_path(), text.as_deref()?))
    }
}

fn git_path<'a>(fields: &mut impl Iterator<Item = &'a [u8]>, missing: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(std::str::from_utf8(
        fields.next().with_context(|| missing.to_string())?,
    )?))
}

/// Git in `root`, without taking optional locks. GIT_DIFF_OPTS is dropped:
/// Git lets it outrank `-U0`, and its context lines would count as changed.
fn git_command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIFF_OPTS");
    command
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_command(root, args).output().context("Cannot run Git")?;
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
/// added), and deleted paths, relative to the root: a `jevgate.toml` in a
/// subdirectory of a Git repository sees the paths below it, as
/// `ls-files` does. Renames keep their source; conflicts stop the review.
fn tracked_changes(root: &Path, revision: &str) -> Result<ChangedPaths> {
    let bytes = git(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--relative",
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

/// Bytes of paths given to one `git diff`, well under the 32,767
/// characters a Windows command line holds.
const PATHSPEC_BYTES: usize = 16 * 1024;

/// `groups` of paths joined in order into batches of about `limit` bytes,
/// a group never split: a renamed file's two names must be diffed together.
fn batches<'a>(groups: &[Vec<&'a str>], limit: usize) -> Vec<Vec<&'a str>> {
    let mut batches: Vec<Vec<&str>> = Vec::new();
    let mut bytes = 0;
    for group in groups {
        let size: usize = group.iter().map(|path| path.len() + 1).sum();
        if batches.is_empty() || bytes + size > limit {
            batches.push(Vec::new());
            bytes = 0;
        }
        bytes += size;
        batches.last_mut().unwrap().extend(group);
    }
    batches
}

/// The lines each of `paths` changed since `revision`, when it was modified
/// or renamed, from one `git diff -U0` read as it streams: only hunk
/// headers and paths are kept, since a patch can run to many megabytes.
/// Added and deleted files are left out; an added one changed throughout.
/// `--text` gives the lines of a file `.gitattributes` marks `-diff` or
/// `binary`, which Git would otherwise report only as changed.
fn changed_lines(root: &Path, revision: &str, paths: &[&str]) -> Result<BTreeMap<PathBuf, Lines>> {
    let mut args = vec![
        "diff",
        "-U0",
        "--text",
        "--inter-hunk-context=0",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--find-renames",
        "--relative",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--diff-filter=MRT",
        revision,
        "--",
    ];
    args.extend_from_slice(paths);
    let mut child = git_command(root, &args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot run Git")?;
    // Read apart from the patch, so a long warning cannot block Git.
    let errors = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = stderr.read_to_end(&mut text);
            text
        })
    });
    let stdout = child.stdout.take().context("Git gave no output")?;
    let parsed = parse_diff(BufReader::new(stdout));
    if parsed.is_err() {
        // Git may still be writing; it must not wait on a full pipe.
        let _ = child.kill();
    }
    let status = child.wait().context("Cannot run Git")?;
    let errors = errors
        .and_then(|thread| thread.join().ok())
        .unwrap_or_default();
    ensure!(
        status.success() || parsed.is_err(),
        "Git diff failed: {}",
        String::from_utf8_lossy(&errors).trim()
    );
    parsed
}

/// Bytes of one patch line kept: enough for a header naming any path.
const KEPT_LINE_BYTES: usize = 64 * 1024;

/// The changed lines per new path of a `-U0` patch. Content lines are
/// counted off each hunk's header, so a line starting `+++ ` inside a hunk
/// is never read as a header.
fn parse_diff(mut reader: impl BufRead) -> Result<BTreeMap<PathBuf, Lines>> {
    let mut files = BTreeMap::<PathBuf, Lines>::new();
    let mut current: Option<PathBuf> = None;
    // Old and new lines still to come in the current hunk.
    let (mut old, mut new) = (0usize, 0usize);
    let mut line = Vec::new();
    while read_line(&mut reader, &mut line, KEPT_LINE_BYTES)? {
        if old + new > 0 {
            match line.first() {
                Some(b'-') => old = old.saturating_sub(1),
                Some(b'+') => new = new.saturating_sub(1),
                Some(b' ') => {
                    old = old.saturating_sub(1);
                    new = new.saturating_sub(1);
                }
                Some(b'\\') => {}
                _ => bail!("Git diff ended a hunk early"),
            }
            continue;
        }
        if line.starts_with(b"diff ") {
            current = None;
        } else if let Some(name) = line.strip_prefix(b"+++ ") {
            current = new_path(name)?;
            if let Some(path) = &current {
                files.entry(path.clone()).or_default();
            }
        } else if line.starts_with(b"@@ ") {
            let hunk = hunk_header(&line).context("Git diff has an unreadable hunk header")?;
            (old, new) = (hunk.old_count, hunk.count);
            if let Some(path) = &current {
                files
                    .entry(path.clone())
                    .or_default()
                    .add_hunk(hunk.start, hunk.count);
            }
        }
    }
    Ok(files)
}

/// Read one line into `line`, keeping at most `keep` bytes of it and
/// skipping the rest; false at the end of the input.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>, keep: usize) -> Result<bool> {
    line.clear();
    let mut read = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(read);
        }
        read = true;
        let end = buffer.iter().position(|&b| b == b'\n');
        let chunk = &buffer[..end.unwrap_or(buffer.len())];
        let room = keep.saturating_sub(line.len());
        line.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = end.map_or(buffer.len(), |end| end + 1);
        reader.consume(used);
        if end.is_some() {
            return Ok(true);
        }
    }
}

/// One hunk's counts: old lines replaced, and where its new lines start and
/// how many there are.
struct Hunk {
    old_count: usize,
    start: usize,
    count: usize,
}

/// `@@ -a[,b] +c[,d] @@ …`, a missing count meaning one line.
fn hunk_header(line: &[u8]) -> Option<Hunk> {
    let text = std::str::from_utf8(line.get(3..)?).ok()?;
    let mut ranges = text.split(' ');
    let range = |text: &str| -> Option<(usize, usize)> {
        let (start, count) = text.split_once(',').unwrap_or((text, "1"));
        Some((start.parse().ok()?, count.parse().ok()?))
    };
    let (_, old_count) = range(ranges.next()?.strip_prefix('-')?)?;
    let (start, count) = range(ranges.next()?.strip_prefix('+')?)?;
    Some(Hunk {
        old_count,
        start,
        count,
    })
}

/// The path of a `+++` line, none for `/dev/null`. Git quotes a name with
/// unusual bytes C-style and puts a tab after a name holding a space.
fn new_path(name: &[u8]) -> Result<Option<PathBuf>> {
    let name = name.strip_suffix(b"\t").unwrap_or(name);
    if name == b"/dev/null" {
        return Ok(None);
    }
    let name = match name.strip_prefix(b"\"") {
        Some(quoted) => unquote(quoted.strip_suffix(b"\"").unwrap_or(quoted)),
        None => name.to_vec(),
    };
    let name = name
        .strip_prefix(b"b/")
        .context("Git diff path lacks its prefix")?;
    Ok(Some(PathBuf::from(
        String::from_utf8(name.to_vec()).context("Git path is not UTF-8")?,
    )))
}

/// A C-quoted name without its quotes: `\t`, `\n`, `\"`, `\\` and octal
/// bytes such as `\303\251`.
fn unquote(quoted: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(quoted.len());
    let mut bytes = quoted.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let Some(escaped) = bytes.next() else { break };
        out.push(match escaped {
            b'a' => b'\x07',
            b'b' => b'\x08',
            b't' => b'\t',
            b'n' => b'\n',
            b'v' => b'\x0b',
            b'f' => b'\x0c',
            b'r' => b'\r',
            b'0'..=b'7' => {
                let mut value = u32::from(escaped - b'0');
                for _ in 0..2 {
                    if let Some(digit) = bytes.next_if(|b| (b'0'..=b'7').contains(b)) {
                        value = value * 8 + u32::from(digit - b'0');
                    }
                }
                value as u8
            }
            other => other,
        });
    }
    out
}

/// Bytes of a file's base version read to compare its members; a larger one
/// is unknown, like other files too large to parse locally.
const BEFORE_BYTES: usize = 1_048_576;

/// `path`, as Git writes it, at `revision`, as UTF-8 text.
fn show(root: &Path, revision: &str, path: &Path) -> Result<String> {
    let path = path.to_str().context("Path is not UTF-8")?;
    let bytes = git(root, &["cat-file", "blob", &format!("{revision}:./{path}")])?;
    ensure!(bytes.len() <= BEFORE_BYTES, "Base version is too large");
    String::from_utf8(bytes).context("Base version is not UTF-8")
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
            lines: BTreeMap::new(),
        })
    }

    /// The same changes with the lines each of the `judged` files changed,
    /// when it existed at the revision. Only those files are diffed, each
    /// with its name before a rename: a changed binary or a regenerated
    /// lockfile the check never reads costs nothing, which matters to
    /// `--watch`, whose every poll collects the files anew. A 150 MB binary
    /// diffed as text took 0.59 s and 460 MB of memory in Git.
    pub fn with_lines<'a>(
        mut self,
        root: &Path,
        judged: impl IntoIterator<Item = &'a Path>,
    ) -> Result<Self> {
        let mut groups: Vec<Vec<&str>> = Vec::new();
        for path in judged {
            let Some(Some(previous)) = self.paths.get(path) else {
                continue;
            };
            let mut names: Vec<&str> = [path, previous.as_path()]
                .iter()
                .filter_map(|name| name.to_str())
                .collect();
            names.dedup();
            groups.push(names);
        }
        for batch in batches(&groups, PATHSPEC_BYTES) {
            self.lines
                .extend(changed_lines(root, &self.revision, &batch)?);
        }
        Ok(self)
    }

    /// Paths the change deleted or renamed away, with the directories it
    /// emptied, which no longer exist under `root`: a document names
    /// `src/legacy/` as readily as a file in it.
    pub fn removed(&self, root: &Path) -> BTreeSet<PathBuf> {
        let renamed = self
            .paths
            .iter()
            .filter_map(|(path, previous)| previous.as_ref().filter(|p| *p != path));
        let mut removed: BTreeSet<PathBuf> = self.deleted.iter().chain(renamed).cloned().collect();
        let folders: BTreeSet<&Path> = removed
            .iter()
            .flat_map(|path| path.ancestors().skip(1))
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect();
        let emptied: Vec<PathBuf> = folders
            .into_iter()
            .filter(|dir| !root.join(dir).exists())
            .map(Path::to_path_buf)
            .collect();
        removed.extend(emptied);
        removed
    }

    /// What the change did to `path`, judged by the lines it touched; none
    /// for a file it added, judged whole, or one it left alone. `root` is
    /// where Git reads the file's base version.
    pub fn file(
        &self,
        root: &Path,
        path: &Path,
        removed: &Arc<BTreeSet<PathBuf>>,
    ) -> Option<FileChange> {
        let previous = self.paths.get(path)?.as_ref()?;
        Some(FileChange {
            lines: self.lines.get(path).cloned().unwrap_or_default(),
            before: Some(Before {
                root: root.to_path_buf(),
                revision: self.revision.clone(),
                path: previous.clone(),
                text: OnceLock::new(),
            }),
            removed: removed.clone(),
        })
    }
}

#[cfg(test)]
mod tests;
