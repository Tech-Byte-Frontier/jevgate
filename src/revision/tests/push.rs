//! What a push sends: the refs a pre-push hook reads, and the commits each
//! pushed ref is compared with.
use super::super::push::{Sent, Update, from_pre_commit, sent, updates};
use crate::tests::Project;
use std::collections::BTreeMap;

const ZERO: &str = "0000000000000000000000000000000000000000";

/// A clone whose `main` a bare remote has, a first commit holding `a.rs`.
struct Clone {
    project: Project,
    _remote: Project,
}

impl Clone {
    fn new() -> Self {
        let remote = Project::new();
        remote.git(&["init", "-q", "--bare"]);
        let project = Project::new();
        project.git(&["init", "-q", "-b", "main"]);
        let clone = Self {
            project,
            _remote: remote,
        };
        clone.commit("a");
        // Git for Windows reads no remote at a path in the verbatim form.
        let url = super::super::for_git(&clone._remote.0);
        clone.git(&["remote", "add", "origin", url.to_str().unwrap()]);
        clone.git(&["push", "-q", "origin", "main"]);
        clone
    }

    fn git(&self, args: &[&str]) -> String {
        self.project.git(args)
    }

    /// Commit `name.rs` on the current branch.
    fn commit(&self, name: &str) {
        self.project
            .write(&format!("{name}.rs"), &format!("fn {name}() {{}}\n"));
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", name]);
    }

    /// `feature` pushed with a commit of `f1.rs` holding `feature`, and then
    /// a commit on `main` of `a.rs` holding `main` and of `m1.rs`, pushed
    /// too; `feature` checked out.
    fn diverged(&self) {
        self.git(&["checkout", "-qb", "feature"]);
        self.commit("f1");
        self.git(&["push", "-q", "origin", "feature"]);
        self.git(&["checkout", "-q", "main"]);
        self.project.write("a.rs", "fn a() { main() }\n");
        self.commit("m1");
        self.git(&["push", "-q", "origin", "main"]);
        self.git(&["checkout", "-q", "feature"]);
    }

    fn id(&self, revision: &str) -> String {
        self.git(&["rev-parse", revision]).trim().to_string()
    }

    /// What pushing `local` sends, over a remote ref at `remote`.
    fn sent(&self, local: &str, remote: Option<&str>) -> Sent {
        let update = Update {
            name: "refs/heads/feature".into(),
            local: self.id(local),
            remote: remote.map(|r| self.id(r)),
        };
        sent(&self.project.0, &update).unwrap()
    }

    /// The files that change from the base of what pushing `local` sends to
    /// the commit pushed.
    fn judged(&self, local: &str, remote: Option<&str>) -> Vec<String> {
        let Sent::Commits { base, commit } = self.sent(local, remote) else {
            panic!("nothing sent");
        };
        let names = self.git(&["diff", "--name-only", &base, &commit]);
        names.lines().map(str::to_string).collect()
    }
}

#[test]
fn the_refs_git_passes_a_pre_push_hook_are_read_and_nothing_else() {
    let (local, remote) = ("1".repeat(40), "2".repeat(40));
    let input = format!(
        "refs/heads/a {local} refs/heads/a {remote}\n\nrefs/heads/b {local} refs/heads/b {ZERO}\n(delete) {ZERO} refs/heads/c {remote}\n"
    );
    let read = updates(&input).unwrap();
    assert_eq!(
        read,
        [
            Update {
                name: "refs/heads/a".into(),
                local: local.clone(),
                remote: Some(remote.clone()),
            },
            Update {
                name: "refs/heads/b".into(),
                local: local.clone(),
                remote: None,
            },
            Update {
                name: "(delete)".into(),
                local: ZERO.into(),
                remote: Some(remote.clone()),
            },
        ]
    );
    assert!(updates("").unwrap().is_empty(), "nothing to push");
    for line in ["refs/heads/a 1111", &format!("a {local} b not-an-id")] {
        assert!(updates(line).is_err(), "{line}");
    }
}

#[test]
fn pre_commit_names_the_pushed_commit_and_what_the_remote_has() {
    let variables = BTreeMap::from([
        ("PRE_COMMIT_TO_REF", "HEAD"),
        ("PRE_COMMIT_FROM_REF", "origin/main"),
        ("PRE_COMMIT_LOCAL_BRANCH", "refs/heads/feature"),
    ]);
    let var = |name: &str| variables.get(name).map(|v| v.to_string());
    assert_eq!(
        from_pre_commit(var),
        Some(Update {
            name: "refs/heads/feature".into(),
            local: "HEAD".into(),
            remote: Some("origin/main".into()),
        })
    );
    assert_eq!(from_pre_commit(|_| None), None, "not run by pre-commit");
}

#[test]
fn a_new_branch_is_compared_with_where_it_leaves_the_remotes_history() {
    let clone = Clone::new();
    clone.git(&["checkout", "-qb", "feature"]);
    clone.commit("f1");
    clone.commit("f2");
    assert_eq!(clone.judged("feature", None), ["f1.rs", "f2.rs"]);
    // Pushed under another name, the same commits are compared alike.
    let Sent::Commits { base, .. } = clone.sent("feature", None) else {
        panic!("nothing sent");
    };
    assert_eq!(base, clone.id("main"));
}

#[test]
fn a_branch_pushed_before_is_compared_with_its_last_push() {
    let clone = Clone::new();
    clone.git(&["checkout", "-qb", "feature"]);
    clone.commit("f1");
    clone.git(&["push", "-q", "origin", "feature"]);
    clone.commit("f2");
    assert_eq!(clone.judged("feature", Some("origin/feature")), ["f2.rs"]);
    assert_eq!(
        clone.sent("feature~1", Some("origin/feature")),
        Sent::Nothing,
        "every commit is on the remote already"
    );
}

#[test]
fn a_rebased_branch_is_compared_with_where_it_now_leaves_the_remotes_history() {
    let clone = Clone::new();
    clone.diverged();
    clone.git(&["rebase", "-q", "main"]);
    // The remote's feature holds the commit the rebase replaced.
    assert_eq!(
        clone.judged("feature", Some("origin/feature")),
        ["f1.rs"],
        "main's commit is not the push's"
    );
}

#[test]
fn a_merge_of_what_a_remote_has_is_not_judged_as_the_pushs_own() {
    let clone = Clone::new();
    clone.diverged();
    clone.git(&["merge", "-q", "--no-edit", "main"]);
    clone.commit("f2");
    assert_eq!(
        clone.judged("feature", Some("origin/feature")),
        ["f2.rs"],
        "compared with main merged into the last push, as Git merges them"
    );
    // A local branch merged in is the push's own work.
    clone.git(&["checkout", "-qb", "topic", "main"]);
    clone.commit("t1");
    clone.git(&["checkout", "-q", "feature"]);
    clone.git(&["merge", "-q", "--no-edit", "topic"]);
    assert_eq!(
        clone.judged("feature", Some("origin/feature")),
        ["f2.rs", "t1.rs"]
    );
}

#[test]
fn a_deletion_a_tag_and_a_history_no_remote_has() {
    let clone = Clone::new();
    let deletion = Update {
        name: "(delete)".into(),
        local: ZERO.into(),
        remote: Some(clone.id("main")),
    };
    assert_eq!(sent(&clone.project.0, &deletion).unwrap(), Sent::Nothing);
    clone.git(&["checkout", "-qb", "feature"]);
    clone.commit("f1");
    clone.git(&["tag", "-a", "-m", "v1", "v1"]);
    let tag = Update {
        name: "refs/tags/v1".into(),
        local: clone.id("v1"),
        remote: None,
    };
    assert_eq!(
        sent(&clone.project.0, &tag).unwrap(),
        Sent::Commits {
            base: clone.id("main"),
            commit: clone.id("feature"),
        },
        "an annotated tag is peeled to its commit"
    );
    let unknown = Update {
        name: "refs/heads/feature".into(),
        local: clone.id("feature"),
        remote: Some("3".repeat(40)),
    };
    assert!(
        matches!(
            sent(&clone.project.0, &unknown).unwrap(),
            Sent::Commits { .. }
        ),
        "a remote object this clone never fetched hides nothing"
    );
    let alone = Project::new();
    alone.write("a.rs", "fn a() {}\n");
    alone.commit_all();
    let first = Update {
        name: "refs/heads/main".into(),
        local: "HEAD".into(),
        remote: None,
    };
    assert_eq!(sent(&alone.0, &first).unwrap(), Sent::Unrooted);
}

#[test]
fn a_pushed_merge_is_judged_for_how_it_resolved_its_conflict() {
    let clone = Clone::new();
    clone.diverged();
    clone.project.write("a.rs", "fn a() { feature() }\n");
    clone.git(&["commit", "-qam", "feature a"]);
    clone.git(&["push", "-q", "origin", "feature"]);
    let merged = crate::tests::git::command(&clone.project.0)
        .args(["merge", "-q", "main"])
        .output()
        .unwrap();
    assert!(!merged.status.success(), "a conflict");
    clone.project.write("a.rs", "fn a() { both() }\n");
    clone.git(&["add", "a.rs"]);
    clone.git(&["commit", "-qm", "merge main"]);
    assert_eq!(
        clone.judged("feature", Some("origin/feature")),
        ["a.rs"],
        "the resolution, and not main's m1.rs"
    );
}
