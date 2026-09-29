//! What a push sends. Git gives a pre-push hook the refs it pushes on stdin,
//! one a line: `<local ref> <local object> <remote ref> <remote object>`.
//! pre-commit and prek read those lines themselves and pass the first ref
//! with something to push as `PRE_COMMIT_TO_REF`. Each pushed commit is
//! compared with the last commit on its first-parent line that a remote
//! already has, as Qlty's pre-push check compares (qltysh/qlty#2867): a
//! branch pushed before is compared with its last push, a new branch with
//! where it left the remote's history, and a rebased one with where it
//! leaves that history now, not with the commits the rebase replaced.
use super::{git, git_command, is_object_id, object_id};
use anyhow::{Result, bail};
use std::{collections::BTreeMap, path::Path};

/// One ref being pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// The local ref as Git names it (`refs/heads/main`), else `HEAD`.
    pub name: String,
    /// The commit pushed, an object ID or a revision; all zeros when the
    /// push deletes the remote ref.
    pub local: String,
    /// The commit the remote ref holds; none when the push creates it.
    pub remote: Option<String>,
}

impl Update {
    /// HEAD, as a push of the current branch would send it: `check
    /// --pre-push` run by hand, with no pushed refs to read.
    pub fn head() -> Self {
        Self {
            name: "HEAD".into(),
            local: "HEAD".into(),
            remote: None,
        }
    }
}

/// What pushing one ref sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sent {
    /// Commits no remote has, up to `commit`, to compare with `base`: a
    /// commit, or the tree of two merged.
    Commits { base: String, commit: String },
    /// Nothing to judge: every commit is on a remote already, the push
    /// deletes the ref, or the ref names no commit.
    Nothing,
    /// Commits none of which a remote has, down to the first: nothing on a
    /// remote to compare them with, as when a repository is first pushed.
    Unrooted,
}

/// The refs a pre-push hook's `input` names. A line that is not four
/// fields, or whose objects are not IDs, stops the check: Git wrote it.
pub fn updates(input: &str) -> Result<Vec<Update>> {
    input
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, local, _, remote] = fields[..] else {
                bail!("Git passed an unexpected pre-push line: {line:?}");
            };
            if !is_object_id(local) || !is_object_id(remote) {
                bail!("Git passed an unexpected pre-push line: {line:?}");
            }
            Ok(Update {
                name: name.into(),
                local: local.into(),
                remote: (!zero(remote)).then(|| remote.into()),
            })
        })
        .collect()
}

/// The ref pre-commit or prek names for a pre-push hook, through `var`:
/// `PRE_COMMIT_TO_REF` is the commit pushed, and `PRE_COMMIT_FROM_REF` what
/// the remote ref holds, else the commit before the first the remote
/// lacks. Their `pre-commit run --from-ref --to-ref` sets the same.
pub fn from_pre_commit(var: impl Fn(&str) -> Option<String>) -> Option<Update> {
    let local = var("PRE_COMMIT_TO_REF")?;
    Some(Update {
        name: var("PRE_COMMIT_LOCAL_BRANCH").unwrap_or_else(|| "HEAD".into()),
        local,
        remote: var("PRE_COMMIT_FROM_REF"),
    })
}

/// What pushing `update` from the repository at `root` sends: the commits
/// neither a remote-tracking ref nor the remote ref reaches, compared with
/// the commits a remote has that they grow from. That is the last commit on
/// the pushed commit's first-parent line that a remote has, unless the push
/// also merges in commits a remote has: after `git merge origin/main`, a
/// push compared with its branch's last push would carry main's changes as
/// its own, and judge code someone else wrote. The base is then the merge
/// of those commits as Git merges them, so what the push judges is its own
/// commits and how it resolved the merge's conflicts; with more than two, it
/// stays the first-parent one.
pub fn sent(root: &Path, update: &Update) -> Result<Sent> {
    if zero(&update.local) {
        return Ok(Sent::Nothing);
    }
    let Some(pushed) = commit_of(root, &update.local) else {
        return Ok(Sent::Nothing);
    };
    // A remote object this clone never fetched hides nothing it has.
    let remote = update.remote.as_deref().and_then(|r| commit_of(root, r));
    let mut args = vec![
        "rev-list",
        "--boundary",
        "--parents",
        &pushed,
        "--not",
        "--remotes",
    ];
    args.extend(remote.as_deref());
    let listed = String::from_utf8(git(root, &args)?)?;
    // `<commit> <parents>…` for each commit no remote has, and
    // `-<commit> …` for each commit a remote has that one of them grows from.
    let mut parents = BTreeMap::new();
    let mut boundary = Vec::new();
    for line in listed.lines() {
        let mut ids = line.split(' ');
        match ids.next() {
            Some(id) if id.starts_with('-') => boundary.push(id[1..].to_string()),
            Some(id) => {
                parents.insert(id.to_string(), ids.next().map(str::to_string));
            }
            None => {}
        }
    }
    if parents.is_empty() {
        return Ok(Sent::Nothing);
    }
    // The first-parent line, from the pushed commit to the first commit a
    // remote has; none when it reaches the first commit of the history.
    let mut at = pushed.as_str();
    let first_parent = loop {
        match parents.get(at) {
            Some(Some(parent)) => at = parent,
            Some(None) => break None,
            None => break Some(at.to_string()),
        }
    };
    let Some(first_parent) = first_parent else {
        return Ok(Sent::Unrooted);
    };
    let base = match independent(root, &boundary)?.as_slice() {
        [one] => one.clone(),
        [one, other] => merged(root, one, other).unwrap_or(first_parent),
        _ => first_parent,
    };
    Ok(Sent::Commits {
        base,
        commit: pushed,
    })
}

/// Of `commits`, those no other one descends from: merging in an ancestor
/// changes nothing.
fn independent(root: &Path, commits: &[String]) -> Result<Vec<String>> {
    if commits.len() < 2 {
        return Ok(commits.to_vec());
    }
    let mut args = vec!["merge-base", "--independent"];
    args.extend(commits.iter().map(String::as_str));
    let listed = String::from_utf8(git(root, &args)?)?;
    Ok(listed.lines().map(str::to_string).collect())
}

/// The tree of commits `one` and `other` merged, as `git merge` merges
/// them, conflict markers and all: a conflicted file's resolution then
/// differs from it where the person resolved the conflict, and nowhere
/// else. None with a Git older than 2.38, which cannot merge without a
/// work tree, or for histories that share no commit.
pub(super) fn merged(root: &Path, one: &str, other: &str) -> Option<String> {
    let output = git_command(
        root,
        &["merge-tree", "--write-tree", "--no-messages", one, other],
    )
    .output()
    .ok()?;
    // Exit 1: the merge has conflicts, and the tree holds their markers.
    if !matches!(output.status.code(), Some(0 | 1)) {
        return None;
    }
    let tree = String::from_utf8(output.stdout).ok()?;
    object_id(tree.lines().next()?.as_bytes()).ok()
}

/// The commit `revision` names in the repository at `root`, peeling a tag;
/// none when it names no commit this clone has.
fn commit_of(root: &Path, revision: &str) -> Option<String> {
    let named = format!("{revision}^{{commit}}");
    git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &named,
        ],
    )
    .ok()
    .and_then(|id| object_id(&id).ok())
}

/// Whether `object` is Git's all-zero ID: a ref that does not exist.
fn zero(object: &str) -> bool {
    object.bytes().all(|b| b == b'0')
}
