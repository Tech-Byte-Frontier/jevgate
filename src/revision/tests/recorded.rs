//! The files a change touched as Git holds them: the index for `--staged`,
//! a commit for `--pre-push`.
use super::*;

/// `lib.rs` committed, then a change staged on its line 2 and another made
/// on disk above it, which shifts its lines; `new.rs` staged new, `gone.rs`
/// staged deleted, and `loose.rs` untracked.
fn staged_in_part() -> Project {
    let project = Project::new();
    project.write("lib.rs", LIB);
    project.write("gone.rs", "fn gone() {}\n");
    project.commit_all();
    project.write("lib.rs", &LIB.replace("one", "uno"));
    project.write("new.rs", "fn fresh() {}\n");
    project.git(&["add", "lib.rs", "new.rs"]);
    project.git(&["rm", "-q", "gone.rs"]);
    project.write(
        "lib.rs",
        &format!(
            "// unstaged\n{}",
            LIB.replace("one", "uno").replace("two", "dos")
        ),
    );
    project.write("loose.rs", "fn loose() {}\n");
    project
}

#[test]
fn the_index_is_read_as_staged_and_only_its_changes() {
    let project = staged_in_part();
    let root = &project.0;
    let base = staged_base(root).unwrap();
    assert_eq!(base, project.git(&["rev-parse", "HEAD"]).trim());
    let recorded = Recorded::load(root, &base, &Now::Index).unwrap();
    let staged = LIB.replace("one", "uno");
    assert_eq!(
        recorded.read(&root.join("lib.rs"), 1024).unwrap().unwrap(),
        staged,
        "the index's text, not the working tree's"
    );
    assert_eq!(
        recorded.size(&root.join("lib.rs")),
        Some(staged.len() as u64)
    );
    assert_eq!(
        recorded.paths().collect::<Vec<_>>(),
        ["lib.rs", "new.rs"].map(Path::new),
        "neither the deleted nor the untracked file"
    );
    assert!(
        recorded.read(&root.join("loose.rs"), 1024).is_none(),
        "a file the change did not touch is read from disk"
    );
    let error = recorded
        .read(&root.join("lib.rs"), 10)
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("exceeds"), "{error}");
    let changes = Changes::between(root, &base, Now::Index)
        .unwrap()
        .with_lines(root, [Path::new("lib.rs")])
        .unwrap();
    assert_eq!(
        changes.paths.keys().collect::<Vec<_>>(),
        ["lib.rs", "new.rs"].map(Path::new)
    );
    assert_eq!(changes.deleted, [PathBuf::from("gone.rs")]);
    assert_eq!(
        changes.lines[Path::new("lib.rs")],
        lines(&[(2, 2)], &[]),
        "the staged line, where the index holds it"
    );
}

#[test]
fn before_the_first_commit_every_staged_file_is_new() {
    let project = Project::new();
    project.git(&["init", "-q"]);
    project.write("lib.rs", LIB);
    project.git(&["add", "lib.rs"]);
    let root = &project.0;
    let base = staged_base(root).unwrap();
    assert!(is_object_id(&base));
    let changes = Changes::between(root, &base, Now::Index).unwrap();
    assert_eq!(changes.paths[Path::new("lib.rs")], None);
    let recorded = Recorded::load(root, &base, &Now::Index).unwrap();
    assert_eq!(
        recorded.read(&root.join("lib.rs"), 1024).unwrap().unwrap(),
        LIB
    );
    let outside = crate::tests::Project::new();
    let error = staged_base(&outside.0).unwrap_err();
    assert!(error.to_string().contains("Git work tree"), "{error}");
}

#[test]
fn a_commit_is_read_as_committed_and_only_text_in_regular_files() {
    let project = Project::new();
    project.write("lib.rs", LIB);
    project.commit_all();
    let base = project.git(&["rev-parse", "HEAD"]).trim().to_string();
    project.write("lib.rs", &LIB.replace("one", "uno"));
    project.write("data.rs", "a\0b");
    #[cfg(unix)]
    std::os::unix::fs::symlink("lib.rs", project.0.join("link.rs")).unwrap();
    project.git(&["add", "-A"]);
    project.git(&["commit", "-qm", "change"]);
    let commit = project.git(&["rev-parse", "HEAD"]).trim().to_string();
    // The work goes on after the commit.
    project.write("lib.rs", "fn later() {}\n");
    let root = &project.0;
    let recorded = Recorded::load(root, &base, &Now::Commit(commit)).unwrap();
    assert_eq!(
        recorded.read(&root.join("lib.rs"), 1024).unwrap().unwrap(),
        LIB.replace("one", "uno")
    );
    let binary = recorded
        .read(&root.join("data.rs"), 1024)
        .unwrap()
        .unwrap_err();
    assert!(binary.to_string().contains("NUL bytes"), "{binary}");
    #[cfg(unix)]
    {
        assert_eq!(recorded.regular(&root.join("link.rs")), Some(false));
        let link = recorded
            .read(&root.join("link.rs"), 1024)
            .unwrap()
            .unwrap_err();
        assert!(link.to_string().contains("symlinks"), "{link}");
    }
    assert!(Recorded::load(root, &base, &Now::WorkingTree).is_err());
}

#[test]
fn a_root_below_the_git_top_level_reads_its_own_paths() {
    let (project, root, _) = below_the_top(&[]);
    project.git(&["add", "-A"]);
    let base = staged_base(&root).unwrap();
    let recorded = Recorded::load(&root, &base, &Now::Index).unwrap();
    assert_eq!(recorded.paths().collect::<Vec<_>>(), [Path::new("lib.rs")]);
    assert_eq!(
        recorded.read(&root.join("lib.rs"), 1024).unwrap().unwrap(),
        LIB.replace("one", "uno")
    );
}

#[test]
fn a_commit_that_concludes_a_merge_is_judged_for_its_resolution_alone() {
    let project = Project::new();
    project.write("lib.rs", LIB);
    project.write("other.rs", "fn other() {}\n");
    project.commit_all();
    project.git(&["branch", "-M", "main"]);
    project.git(&["checkout", "-qb", "side"]);
    project.write("lib.rs", &LIB.replace("two", "side"));
    project.write("side.rs", "fn side() {}\n");
    project.git(&["commit", "-qam", "side"]);
    project.git(&["add", "side.rs"]);
    project.git(&["commit", "-qm", "side file"]);
    project.git(&["checkout", "-q", "main"]);
    project.write("lib.rs", &LIB.replace("two", "main"));
    project.git(&["commit", "-qam", "main"]);
    // The merge stops on lib.rs's line 6; the person resolves it.
    let merged = crate::tests::git::command(&project.0)
        .args(["merge", "-q", "side"])
        .output()
        .unwrap();
    assert!(!merged.status.success(), "a conflict");
    project.write("lib.rs", &LIB.replace("two", "both"));
    project.git(&["add", "lib.rs"]);
    let root = &project.0;
    let base = staged_base(root).unwrap();
    assert_ne!(base, project.git(&["rev-parse", "HEAD"]).trim());
    let changes = Changes::between(root, &base, Now::Index)
        .unwrap()
        .with_lines(root, [Path::new("lib.rs")])
        .unwrap();
    assert_eq!(
        changes.paths.keys().collect::<Vec<_>>(),
        [Path::new("lib.rs")],
        "the side branch's file came with the merge, and is not the commit's"
    );
    assert_eq!(
        changes.lines[Path::new("lib.rs")].changed,
        [(6, 6)],
        "the resolved line, where the conflict's markers were"
    );
}
