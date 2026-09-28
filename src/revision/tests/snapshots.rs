//! Snapshots of the working tree, as the agent hook takes them at a turn's
//! events, and the changes read between two of them.
use super::*;

#[test]
fn a_snapshot_holds_untracked_files_but_not_ignored_ones_and_leaves_the_index_alone() {
    let project = Project::new();
    project.write(".gitignore", "*.log\n");
    project.write("a.rs", "fn a() {}\n");
    project.commit_all();
    project.write("a.rs", "fn a() { 1 }\n");
    project.git(&["add", "a.rs"]);
    project.write("b.rs", "fn b() {}\n");
    project.write("debug.log", "trace\n");
    let tree = snapshot_of(&project, &project.0);
    let files = project.git(&["ls-tree", "-r", "--name-only", &tree]);
    assert_eq!(files, ".gitignore\na.rs\nb.rs\n");
    assert_eq!(
        project.git(&["status", "--porcelain"]),
        "M  a.rs\n?? b.rs\n"
    );
    assert_eq!(project.git(&["stash", "list"]), "");
    assert!(!project.0.join(".git/jevgate-test.index").exists());
    let empty = Project::new();
    empty.write("a.rs", "fn a() {}\n");
    empty.git(&["init", "-q"]);
    let tree = snapshot_of(&empty, &empty.0);
    assert_eq!(
        empty.git(&["ls-tree", "--name-only", &tree]),
        "a.rs\n",
        "no commit yet"
    );
}

/// Whether the project's object store holds `id`.
fn stored(project: &Project, id: &str) -> bool {
    git_in(&project.0)
        .args(["cat-file", "-e", id])
        .status()
        .unwrap()
        .success()
}

#[test]
fn a_snapshot_leaves_out_the_directories_a_check_never_reads() {
    let project = Project::new();
    project.write("lib.rs", "fn a() {}\n");
    project.commit_all();
    project.write("node_modules/pkg/index.js", "module.exports = 1;\n");
    project.write("web/node_modules/pkg/index.js", "module.exports = 2;\n");
    project.write("target/debug/out.rs", "fn built() {}\n");
    project.write("src/build.rs", "fn b() {}\n");
    project.write("app.js", "x\n");
    let tree = snapshot_of(&project, &project.0);
    assert_eq!(
        project.git(&["ls-tree", "-r", "--name-only", &tree]),
        "app.js\nlib.rs\nsrc/build.rs\n"
    );
    let dependency = project.git(&["hash-object", "node_modules/pkg/index.js"]);
    assert!(!stored(&project, dependency.trim()), "never copied");
}

#[test]
fn a_file_larger_than_a_snapshot_holds_is_recorded_by_a_stand_in() {
    let project = Project::new();
    project.write("lib.rs", "fn a() {}\n");
    project.write("data.bin", "small\n");
    project.commit_all();
    let large = "x".repeat(snapshot::SNAPSHOT_BYTES as usize + 1);
    project.write("data.bin", &large);
    project.write("dump.sql", &large);
    let baseline = large.replace('x', "b");
    project.write("jevgate-baseline.json", &baseline);
    let before = snapshot_of(&project, &project.0);
    let size = |tree: &str, path: &str| -> u64 {
        let object = format!("{tree}:{path}");
        project
            .git(&["cat-file", "-s", &object])
            .trim()
            .parse()
            .unwrap()
    };
    assert!(
        size(&before, "data.bin") < 100,
        "a tracked file grown past the limit"
    );
    assert!(size(&before, "dump.sql") < 100, "an untracked one");
    assert_eq!(
        size(&before, "jevgate-baseline.json"),
        baseline.len() as u64,
        "the baseline is read whole as the turn began"
    );
    let id = project.git(&["hash-object", "dump.sql"]);
    assert!(!stored(&project, id.trim()), "Git never copied it");
    project.write("dump.sql", &format!("{large}y"));
    let after = snapshot_of(&project, &project.0);
    let changes = Changes::between(&project.0, &before, &after).unwrap();
    assert_eq!(
        changes.paths.keys().collect::<Vec<_>>(),
        [Path::new("dump.sql")],
        "its stand-in changes with it"
    );
}

#[cfg(unix)]
#[test]
fn a_file_git_cannot_read_is_left_out_of_a_snapshot() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    project.write("lib.rs", "fn a() {}\n");
    project.commit_all();
    project.write("new.rs", "fn new() {}\n");
    project.write("dump.sql", "rows\n");
    let dump = project.0.join("dump.sql");
    std::fs::set_permissions(&dump, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable = std::fs::File::open(&dump).is_ok();
    let tree = snapshot_of(&project, &project.0);
    std::fs::set_permissions(&dump, std::fs::Permissions::from_mode(0o644)).unwrap();
    let files = project.git(&["ls-tree", "-r", "--name-only", &tree]);
    if !readable {
        assert_eq!(files, "lib.rs\nnew.rs\n");
    }
}

#[test]
fn a_rewrite_in_the_second_of_the_last_add_is_in_the_snapshot() {
    let project = Project::new();
    let one = "fn a() -> i32 { 1 }\n";
    project.write("a.rs", one);
    project.commit_all();
    let second = |path: &Path| {
        let modified = std::fs::metadata(path).unwrap().modified().unwrap();
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    };
    // An entry as new as the index file is read again, not trusted: a copy
    // of the index stamped a second later must not hide a rewrite of the
    // same size made in the second Git last wrote the index.
    for _ in 0..5 {
        project.write("a.rs", one);
        project.git(&["add", "a.rs"]);
        project.write("a.rs", &one.replace('1', "2"));
        if second(&project.0.join(".git/index")) == second(&project.0.join("a.rs")) {
            break;
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let tree = snapshot_of(&project, &project.0);
    let blob = format!("{tree}:a.rs");
    assert_eq!(
        project.git(&["cat-file", "blob", &blob]),
        one.replace('1', "2")
    );
}

#[test]
fn a_snapshot_past_its_deadline_stops() {
    let project = Project::new();
    project.write("a.rs", "fn a() {}\n");
    project.commit_all();
    let index = index_file(&project.0).unwrap();
    let scratch = project.0.join(".git/jevgate-test.index");
    let error = snapshot(&project.0, &index, &scratch, std::time::Instant::now()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Git ls-files did not finish in the hook's time"
    );
    assert!(!scratch.exists());
}

#[test]
fn changes_between_two_snapshots_follow_the_working_tree() {
    let project = Project::new();
    project.write(".gitignore", "*.log\n");
    project.write("edited.rs", "fn edited() {}\n");
    project.write("gone.rs", "fn gone() {}\n");
    project.write("moved.rs", &"fn moved() { let unchanged = 1; }\n".repeat(5));
    project.commit_all();
    project.write("scratch.rs", "fn scratch() {}\n");
    let before = snapshot_of(&project, &project.0);
    project.write("edited.rs", "fn edited() { 1 }\n");
    std::fs::remove_file(project.0.join("gone.rs")).unwrap();
    std::fs::rename(project.0.join("moved.rs"), project.0.join("renamed.rs")).unwrap();
    project.write("scratch.rs", "fn scratch() { 2 }\n");
    project.write("new.rs", "fn new() {}\n");
    project.write("trace.log", "trace\n");
    let after = snapshot_of(&project, &project.0);
    let changes = Changes::between(&project.0, &before, &after).unwrap();
    let path = |name: &str| PathBuf::from(name);
    assert_eq!(
        changes.paths,
        BTreeMap::from([
            (path("edited.rs"), Some(path("edited.rs"))),
            (path("new.rs"), None),
            (path("renamed.rs"), Some(path("moved.rs"))),
            (path("scratch.rs"), Some(path("scratch.rs"))),
        ]),
        "a file untracked before the turn is edited, not new"
    );
    assert_eq!(changes.deleted, [path("gone.rs")]);
    assert_eq!(changes.revision, before);
    assert!(Changes::between(&project.0, "HEAD", &after).is_err());
}

#[test]
fn lines_between_two_snapshots_are_the_ones_the_turn_changed() {
    let project = Project::new();
    project.write("src/lib.rs", LIB);
    project.commit_all();
    // Edited before the turn began: its lines are not the turn's.
    let started = LIB.replace("one", "uno");
    project.write("src/lib.rs", &started);
    project.write("scratch.rs", LIB);
    let before = snapshot_of(&project, &project.0);
    project.write("src/lib.rs", &started.replace("four", "cuatro"));
    project.write("scratch.rs", &LIB.replace("two", "dos"));
    let after = snapshot_of(&project, &project.0);
    let judged = ["src/lib.rs", "scratch.rs"].map(Path::new);
    let changes = Changes::between(&project.0, &before, &after)
        .unwrap()
        .with_lines(&project.0, judged)
        .unwrap();
    let file = |name: &str| {
        changes
            .file(&project.0, Path::new(name), &Arc::default())
            .unwrap()
    };
    assert_eq!(file("src/lib.rs").lines, lines(&[(8, 8)], &[]));
    assert_eq!(
        file("scratch.rs").lines,
        lines(&[(6, 6)], &[]),
        "a file untracked when the turn began keeps its unchanged lines"
    );
    assert_eq!(
        file("src/lib.rs").before().unwrap().1,
        started,
        "the base version is the turn's start"
    );
}

#[test]
fn snapshots_of_a_root_below_the_git_top_level_are_compared_relative_to_it() {
    let (project, root, before) = below_the_top(&[]);
    project.write("app/new.rs", "fn new() {}\n");
    project.write("other.rs", "fn other() { 1 }\n");
    let after = snapshot_of(&project, &root);
    let changes = Changes::between(&root, &before, &after)
        .unwrap()
        .with_lines(&root, [Path::new("lib.rs")])
        .unwrap();
    assert_eq!(
        changes.paths.keys().collect::<Vec<_>>(),
        [Path::new("lib.rs"), Path::new("new.rs")]
    );
    let file = changes
        .file(&root, Path::new("lib.rs"), &Arc::default())
        .unwrap();
    assert_eq!(file.lines, lines(&[(2, 2)], &[]));
    assert_eq!(file.before().unwrap().1, LIB);
}
