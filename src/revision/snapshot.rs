//! Snapshots of the working tree for the agent hook's turns: a Git tree of
//! the tracked and untracked files under the root, less ignored ones,
//! written through a copy of the repository's index, so its own index and
//! stash list are never touched. Every Git process a snapshot runs is stopped
//! at the event's deadline.
use super::{is_object_id, object_id};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// The largest file a snapshot records as it is: a check reads a document up
/// to 1 MiB, and source up to `--max-file-bytes`, which is at most 1 MiB. A
/// larger changed or untracked file is recorded as a stand-in naming its
/// size and modification time, so a turn still sees it change while Git
/// never copies it into the object store: a 400 MB untracked file took 21 s
/// and 400 MB of objects per snapshot, and a 60 MB database the application
/// rewrote between two events added 31 MB of objects each time.
pub(crate) const SNAPSHOT_BYTES: u64 = 1_048_576;

/// Files a snapshot records whole whatever their size: the agent hook's gate
/// reads them as the turn began.
const WHOLE: [&str; 2] = [crate::init::CONFIG_FILE, crate::baseline::BASELINE_FILE];

/// The working tree under `root` as a Git tree, written through a copy of
/// `index` at `scratch` and finished by `deadline`. The copy's stat cache
/// hashes only the files that changed (42-173 ms on corpus clones of 7,310
/// and 8,707 files).
pub fn snapshot(root: &Path, index: &Path, scratch: &Path, deadline: Instant) -> Result<String> {
    let _ = fs::remove_file(scratch);
    // Left by a process killed while Git held it, it would stop the next
    // process given the same ID.
    let _ = fs::remove_file(scratch.with_extension("index.lock"));
    copy_index(index, scratch)?;
    let tree = Snapshot {
        root,
        scratch,
        deadline,
    }
    .take();
    let _ = fs::remove_file(scratch);
    tree
}

/// Copy `index` to `scratch` with its modification time. Git reads a file
/// again when its entry is as new as the index file (racily clean); a copy
/// stamped now would trust such an entry, so a same-size rewrite in the
/// second of the last `git add` would be missed. `fs::copy` keeps the time
/// on macOS only.
fn copy_index(index: &Path, scratch: &Path) -> Result<()> {
    match fs::copy(index, scratch) {
        Ok(_) => {}
        // A repository without commits may have no index yet.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("Cannot copy the Git index"),
    }
    let modified = fs::metadata(index)?.modified()?;
    fs::File::options()
        .write(true)
        .open(scratch)
        .and_then(|copy| copy.set_modified(modified))
        .context("Cannot copy the Git index")
}

/// One snapshot being taken.
struct Snapshot<'a> {
    root: &'a Path,
    scratch: &'a Path,
    deadline: Instant,
}

/// A changed or untracked file too large to record as it is.
struct Large {
    /// Relative to the root, as Git lists it.
    path: String,
    bytes: u64,
    modified: SystemTime,
}

impl Large {
    /// The stand-in recorded for it: its size and time change when it does.
    fn stand_in(&self) -> String {
        let modified = self.modified.duration_since(UNIX_EPOCH).unwrap_or_default();
        format!(
            "JevGate snapshot stand-in: {} bytes, modified {}.{:09}\n",
            self.bytes,
            modified.as_secs(),
            modified.subsec_nanos()
        )
    }
}

impl Snapshot<'_> {
    fn take(&self) -> Result<String> {
        let listed = self.git(
            &[
                "ls-files",
                "-z",
                "--modified",
                "--others",
                "--exclude-standard",
            ],
            None,
        )?;
        let large = self.large(&listed);
        self.add(&large)?;
        if !large.is_empty() {
            self.stand_ins(&large)?;
        }
        object_id(&self.git(&["write-tree"], None)?)
    }

    /// The listed files larger than [`SNAPSHOT_BYTES`], less [`WHOLE`] ones.
    fn large(&self, listed: &[u8]) -> Vec<Large> {
        let names: BTreeSet<&str> = listed
            .split(|b| *b == 0)
            .filter_map(|name| std::str::from_utf8(name).ok())
            .filter(|name| !name.is_empty() && !WHOLE.contains(name))
            .collect();
        names
            .into_iter()
            .filter_map(|name| {
                let metadata = fs::symlink_metadata(self.root.join(name)).ok()?;
                (metadata.is_file() && metadata.len() > SNAPSHOT_BYTES).then(|| Large {
                    path: name.to_string(),
                    bytes: metadata.len(),
                    modified: metadata.modified().unwrap_or(UNIX_EPOCH),
                })
            })
            .collect()
    }

    /// Record every change under the root but the `large` files. A file Git
    /// cannot read, such as a database volume another user owns, is passed
    /// over (`--ignore-errors`, exit 1) instead of stopping every snapshot.
    fn add(&self, large: &[Large]) -> Result<()> {
        let mut pathspecs = b".\0".to_vec();
        for file in large {
            pathspecs.extend(format!(":(exclude,literal){}\0", file.path).bytes());
        }
        self.run(
            &[
                "add",
                "--all",
                "--ignore-errors",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            Some(pathspecs),
            &[0, 1],
        )
        .map(drop)
    }

    /// Record each of `large` as its stand-in: the stand-ins are written
    /// beside the scratch index and hashed by one Git process, since a
    /// repository may hold many large untracked files.
    fn stand_ins(&self, large: &[Large]) -> Result<()> {
        let texts: Vec<PathBuf> = (0..large.len())
            .map(|n| self.scratch.with_extension(format!("stand-in-{n}")))
            .collect();
        let written = texts
            .iter()
            .zip(large)
            .try_for_each(|(text, file)| fs::write(text, file.stand_in()));
        // Named from the root, with `/`: a Windows root in its `\\?\` form is
        // not a path Git reads.
        let listed: String = texts
            .iter()
            .map(|text| {
                let name = text.strip_prefix(self.root).unwrap_or(text);
                format!("{}\n", name.to_string_lossy().replace('\\', "/"))
            })
            .collect();
        let ids = written.map_err(anyhow::Error::from).and_then(|()| {
            self.git(
                &["hash-object", "-w", "--no-filters", "--stdin-paths"],
                Some(listed.into_bytes()),
            )
        });
        for text in &texts {
            let _ = fs::remove_file(text);
        }
        let ids = String::from_utf8(ids?)?;
        // The index takes paths from the Git top level, which the root may sit below.
        let prefix = String::from_utf8(self.git(&["rev-parse", "--show-prefix"], None)?)?;
        let prefix = prefix.trim_end_matches('\n');
        let mut entries = Vec::new();
        for (file, id) in large.iter().zip(ids.lines()) {
            ensure!(is_object_id(id), "Git did not print an object ID");
            entries.extend(format!("100644 {id}\t{prefix}{}\0", file.path).bytes());
        }
        self.git(&["update-index", "-z", "--index-info"], Some(entries))
            .map(drop)
    }

    fn git(&self, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>> {
        self.run(args, input, &[0])
    }

    /// What Git, given `args` and `input` on stdin, printed about the scratch
    /// index, when it exits with one of the `accepted` codes; it is stopped
    /// at the deadline. Pathspec magic is read, so `add` can leave files out.
    fn run(&self, args: &[&str], input: Option<Vec<u8>>, accepted: &[i32]) -> Result<Vec<u8>> {
        let name = args.first().copied().unwrap_or_default();
        ensure!(
            Instant::now() < self.deadline,
            "Git {name} did not finish in the hook's time"
        );
        let mut child = Command::new("git")
            .arg("-C")
            .arg(self.root)
            .args(args)
            .env("GIT_INDEX_FILE", self.scratch)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env_remove("GIT_LITERAL_PATHSPECS")
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Cannot run Git")?;
        if let (Some(mut stdin), Some(bytes)) = (child.stdin.take(), input) {
            // Written apart, so Git never waits on a full output pipe.
            std::thread::spawn(move || stdin.write_all(&bytes));
        }
        let Some(output) = crate::child::output_until(child, self.deadline)? else {
            bail!("Git {name} did not finish in the hook's time");
        };
        ensure!(
            output
                .status
                .code()
                .is_some_and(|code| accepted.contains(&code)),
            "Git {name} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(output.stdout)
    }
}
