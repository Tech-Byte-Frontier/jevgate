use super::*;
use crate::tests::Project;

fn lines(changed: &[(usize, usize)], removed_after: &[usize]) -> Lines {
    Lines {
        changed: changed.to_vec(),
        removed_after: removed_after.to_vec(),
    }
}

#[test]
fn a_patch_gives_each_files_changed_lines_and_removals() {
    let patch = concat!(
        "diff --git a/src/lib.rs b/src/lib.rs\n",
        "index 1111111..2222222 100644\n",
        "--- a/src/lib.rs\n",
        "+++ b/src/lib.rs\n",
        "@@ -3 +3 @@ fn a() {\n",
        "-    old();\n",
        "+    new();\n",
        "@@ -10,2 +9,0 @@ fn b() {\n",
        "-    gone();\n",
        "-    gone_too();\n",
        "@@ -20,0 +19,3 @@ fn c() {\n",
        "++++ b/not/a/header.rs\n",
        "+@@ -1 +1 @@\n",
        "+diff --git a/x b/x\n",
        "diff --git a/my file.rs b/my file.rs\n",
        "--- a/my file.rs\t\n",
        "+++ b/my file.rs\t\n",
        "@@ -1 +1,2 @@\n",
        "-x\n",
        "\\ No newline at end of file\n",
        "+x\n",
        "+y\n",
        "\\ No newline at end of file\n",
        "diff --git \"a/caf\\303\\251 \\\"q\\\".rs\" \"b/caf\\303\\251 \\\"q\\\".rs\"\n",
        "--- \"a/caf\\303\\251 \\\"q\\\".rs\"\t\n",
        "+++ \"b/caf\\303\\251 \\\"q\\\".rs\"\t\n",
        "@@ -2 +2 @@\n",
        "-a\n",
        "+b\n",
        "diff --git a/old.rs b/new.rs\n",
        "similarity index 90%\n",
        "rename from old.rs\n",
        "rename to new.rs\n",
        "--- a/old.rs\n",
        "+++ b/new.rs\n",
        "@@ -5 +5 @@\n",
        "-a\n",
        "+b\n",
        "diff --git a/pure.rs b/moved.rs\n",
        "similarity index 100%\n",
        "rename from pure.rs\n",
        "rename to moved.rs\n",
        "diff --git a/logo.png b/logo.png\n",
        "Binary files a/logo.png and b/logo.png differ\n",
    );
    let files = parse_diff(patch.as_bytes()).unwrap();
    let expected: BTreeMap<PathBuf, Lines> = [
        ("src/lib.rs", lines(&[(3, 3), (19, 21)], &[9])),
        ("my file.rs", lines(&[(1, 2)], &[])),
        ("café \"q\".rs", lines(&[(2, 2)], &[])),
        ("new.rs", lines(&[(5, 5)], &[])),
    ]
    .into_iter()
    .map(|(path, lines)| (PathBuf::from(path), lines))
    .collect();
    assert_eq!(files, expected);
    assert!(parse_diff(&b"+++ b/a.rs\n@@ -1,2 +1,2 @@\n-a\n@@ -9 +9 @@\n"[..]).is_err());
}

#[test]
fn a_change_touches_the_spans_holding_its_lines_or_its_removals() {
    let change = lines(&[(10, 12)], &[20]);
    assert!(change.touch(12, 15) && change.touch(1, 10) && change.touch(11, 11));
    assert!(!change.touch(13, 19) && !change.touch(1, 9));
    // Lines removed after line 20 sit inside 18..=25, and at the edge of the
    // spans that end at 20 or start at 21.
    assert!(change.touch(18, 25));
    assert!(!change.touch(15, 20) && !change.touch(21, 30));
}

#[test]
fn long_lines_are_cut_without_losing_the_next() {
    let text = format!("{}\nnext\n", "x".repeat(100));
    let mut reader = text.as_bytes();
    let mut line = Vec::new();
    assert!(read_line(&mut reader, &mut line, 8).unwrap());
    assert_eq!(line, b"xxxxxxxx");
    assert!(read_line(&mut reader, &mut line, 8).unwrap());
    assert_eq!(line, b"next");
    assert!(!read_line(&mut reader, &mut line, 8).unwrap());
}

const LIB: &str = "fn a() {\n    one();\n}\n\nfn b() {\n    two();\n    three();\n    four();\n}\n";

#[test]
fn changes_from_git_hold_each_files_lines_and_its_base_text() {
    let project = Project::new();
    project.write("src/lib.rs", LIB);
    project.write("old.rs", "fn kept() {\n    stays();\n}\n");
    project.write("notes.md", "# Notes\n");
    project.write("legacy/one.rs", "fn one() {}\n");
    project.write(".gitattributes", "marked.rs -diff\n");
    project.write("marked.rs", "fn marked() {\n    one();\n}\n");
    project.write("Cargo.lock", "version = 3\n");
    project.commit_all();
    std::fs::remove_dir_all(project.0.join("legacy")).unwrap();
    project.write("marked.rs", "fn marked() {\n    two();\n}\n");
    project.write("Cargo.lock", "version = 4\n");
    let edited = LIB
        .replace("    one();", "    uno();")
        .replace("    three();\n", "");
    project.write("src/lib.rs", &format!("{edited}fn c() {{ added(); }}\n"));
    project.git(&["mv", "old.rs", "renamed.rs"]);
    project.write("renamed.rs", "fn kept() {\n    stays();\n    more();\n}\n");
    std::fs::remove_file(project.0.join("notes.md")).unwrap();
    project.write("new.rs", "fn fresh() {}\n");
    let judged = ["src/lib.rs", "renamed.rs", "marked.rs", "new.rs"].map(Path::new);
    let changes = Changes::load(&project.0, "HEAD")
        .unwrap()
        .with_lines(&project.0, judged)
        .unwrap();
    assert!(changes.paths.contains_key(Path::new("Cargo.lock")));
    assert_eq!(
        changes.lines.keys().collect::<Vec<_>>(),
        ["marked.rs", "renamed.rs", "src/lib.rs"].map(Path::new),
        "only the judged files are diffed, and a renamed one with its old name"
    );
    assert_eq!(changes.paths[Path::new("new.rs")], None);
    assert_eq!(
        changes.paths[Path::new("renamed.rs")].as_deref(),
        Some(Path::new("old.rs"))
    );
    let removed = changes.removed(&project.0);
    let expected = ["legacy", "legacy/one.rs", "notes.md", "old.rs"];
    assert_eq!(removed, expected.into_iter().map(PathBuf::from).collect());
    let removed = Arc::new(removed);
    let lib = changes
        .file(&project.0, Path::new("src/lib.rs"), &removed)
        .unwrap();
    // `uno` on line 2, `three` removed after line 6 and `c` on line 9.
    assert_eq!(lib.lines, lines(&[(2, 2), (9, 9)], &[6]));
    let names = |_: &Path, text: &str| -> BTreeSet<String> {
        ["a", "b", "c"]
            .into_iter()
            .filter(|name| text.contains(&format!("fn {name}(")))
            .map(String::from)
            .collect()
    };
    // `c` starts on an added line and is new; `a` and `b` start on lines
    // the change left, and `a` pretending to start on line 2 is known.
    assert!(lib.adds([("a", 1), ("b", 5), ("c", 9)].into_iter(), names));
    assert!(!lib.adds([("a", 1), ("b", 5)].into_iter(), names));
    assert!(!lib.adds([("a", 2)].into_iter(), names));
    let renamed = changes
        .file(&project.0, Path::new("renamed.rs"), &removed)
        .unwrap();
    assert_eq!(renamed.lines, lines(&[(3, 3)], &[]));
    assert!(
        changes
            .file(&project.0, Path::new("new.rs"), &removed)
            .is_none()
    );
    let marked = changes
        .file(&project.0, Path::new("marked.rs"), &removed)
        .unwrap();
    assert_eq!(
        marked.lines,
        lines(&[(2, 2)], &[]),
        "`-diff` keeps the lines"
    );
    assert!(!FileChange::unchanged(removed).adds([("a", 1)].into_iter(), names));
}

#[test]
fn a_root_below_the_git_top_level_sees_its_own_paths() {
    let (project, root, _) = below_the_top(&[]);
    project.write("other.rs", "fn other() { 1 }\n");
    let changes = Changes::load(&root, "HEAD")
        .unwrap()
        .with_lines(&root, [Path::new("lib.rs")])
        .unwrap();
    assert_eq!(
        changes.paths.keys().collect::<Vec<_>>(),
        [Path::new("lib.rs")]
    );
    let file = changes
        .file(&root, Path::new("lib.rs"), &Arc::default())
        .unwrap();
    assert_eq!(file.lines, lines(&[(2, 2)], &[]));
    assert_eq!(file.before().unwrap().1, LIB);
}

#[test]
fn paths_are_diffed_in_batches_that_keep_a_renamed_files_names_together() {
    let groups = [vec!["a.rs"], vec!["b.rs", "old/b.rs"], vec!["c.rs"]];
    // Each name counts with the space that follows it: 5, 14 and 5 bytes.
    assert_eq!(
        batches(&groups, 19),
        [vec!["a.rs", "b.rs", "old/b.rs"], vec!["c.rs"]]
    );
    assert_eq!(
        batches(&groups, 10),
        [vec!["a.rs"], vec!["b.rs", "old/b.rs"], vec!["c.rs"]],
        "a group longer than the limit is a batch of its own, never split"
    );
    assert!(
        batches(&[], 10).is_empty(),
        "no path, no diff of everything"
    );
}

/// A snapshot of the working tree at `root`, through a scratch index
/// outside it.
fn snapshot_of(project: &Project, root: &Path) -> String {
    let index = index_file(root).unwrap();
    let scratch = project.0.join(".git/jevgate-test.index");
    snapshot(root, &index, &scratch, far_off()).unwrap()
}

/// A deadline no test reaches.
fn far_off() -> std::time::Instant {
    std::time::Instant::now() + std::time::Duration::from_secs(600)
}

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
    std::process::Command::new("git")
        .args(["cat-file", "-e", id])
        .current_dir(&*project.0)
        .status()
        .unwrap()
        .success()
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

/// A committed project whose `jevgate.toml` would sit in `app/`, with
/// `extra` files there too; its root and a snapshot taken from it, then
/// `app/lib.rs` edited on line 2.
fn below_the_top(extra: &[(&str, &str)]) -> (Project, PathBuf, String) {
    let project = Project::new();
    project.write("app/lib.rs", LIB);
    project.write("other.rs", "fn other() {}\n");
    for (name, text) in extra {
        project.write(&format!("app/{name}"), text);
    }
    project.commit_all();
    let root = project.0.join("app");
    let tree = snapshot_of(&project, &root);
    project.write("app/lib.rs", &LIB.replace("one", "uno"));
    (project, root, tree)
}

#[test]
fn blobs_are_read_from_a_tree_relative_to_the_root() {
    let big = "x".repeat(100);
    let (_project, root, tree) = below_the_top(&[("big.rs", &big), ("data.bin", "a\0b")]);
    let paths = ["lib.rs", "missing.rs", "big.rs", "data.bin", "../other.rs"].map(Path::new);
    // LIB is 70 bytes, big.rs 100.
    let texts = blobs(&root, &tree, &paths, 96).unwrap();
    assert_eq!(
        texts,
        BTreeMap::from([
            (PathBuf::from("lib.rs"), LIB.to_string()),
            (PathBuf::from("../other.rs"), "fn other() {}\n".to_string()),
        ]),
        "the tree's text, not the working tree's; missing, large and binary blobs are left out"
    );
    assert!(blobs(&root, &tree, &[], 96).unwrap().is_empty());
}
