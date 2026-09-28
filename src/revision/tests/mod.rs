//! Changes against a Git revision: the lines and removals a patch holds,
//! the files and lines `Changes` reads from Git, and blobs read from a
//! tree. Snapshots of the working tree, and the changes read between two
//! of them, are in `snapshots`.
use super::*;
use crate::tests::Project;
mod snapshots;

fn lines(changed: &[(usize, usize)], removed: &[Removal]) -> Lines {
    Lines {
        changed: changed.to_vec(),
        removed: removed.to_vec(),
    }
}

/// Lines removed after `after`: the first indented `opens` when it holds
/// text, the last holding text when `closes`.
fn removal(after: usize, opens: Option<usize>, closes: bool) -> Removal {
    Removal {
        after,
        opens,
        closes,
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
        (
            "src/lib.rs",
            lines(&[(3, 3), (19, 21)], &[removal(9, Some(4), true)]),
        ),
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
fn paths_for_git_variables_drop_the_windows_verbatim_prefix() {
    let plain = |path: &str| for_git(Path::new(path)).to_string_lossy().into_owned();
    assert_eq!(
        plain(r"\\?\C:\Temp\repo\.jevgate\turns\a.index"),
        r"C:\Temp\repo\.jevgate\turns\a.index"
    );
    assert_eq!(
        plain(r"\\?\UNC\server\share\repo\a.index"),
        r"\\server\share\repo\a.index"
    );
    assert_eq!(
        plain("/tmp/repo/.jevgate/turns/a.index"),
        "/tmp/repo/.jevgate/turns/a.index"
    );
    assert_eq!(plain(r"C:\Temp\a.index"), r"C:\Temp\a.index");
}

#[test]
fn a_change_touches_the_spans_holding_its_lines_or_its_removals() {
    let change = lines(&[(10, 12)], &[removal(20, None, false)]);
    assert!(change.touch(12, 15) && change.touch(1, 10) && change.touch(11, 11));
    assert!(!change.touch(13, 19) && !change.touch(1, 9));
    // Lines removed after line 20 sit inside 18..=25, and at the edge of the
    // spans that end at 20 or start at 21.
    assert!(change.touch(18, 25));
    assert!(!change.touch(15, 20) && !change.touch(21, 30));
}

#[test]
fn a_removal_at_an_edge_belongs_to_the_lines_it_was_written_against() {
    // A removed decorator above `orders`, a removed function with the blank
    // lines after it above `b`, and the last statement of `total`'s body.
    let patch = concat!(
        "+++ b/views.py\n",
        "@@ -5 +4,0 @@ def home(request):\n",
        "-@login_required\n",
        "@@ -12,4 +10,0 @@ def orders(request):\n",
        "-def gone():\n",
        "-    return 2\n",
        "-\n",
        "-\n",
        "@@ -18 +14,0 @@ def total():\n",
        "-    audit(total)\n",
    );
    let lines = &parse_diff(patch.as_bytes()).unwrap()[Path::new("views.py")];
    assert_eq!(
        lines.removed,
        [
            removal(4, Some(0), true),
            removal(10, Some(0), false),
            removal(14, Some(4), true)
        ]
    );
    // `orders` now starts on line 5, `b` on 11, `total` spans 13..=14.
    assert!(
        lines.edge((5, 8), 0),
        "the decorator was written against it"
    );
    assert!(!lines.touch(5, 8));
    assert!(
        !lines.edge((11, 13), 0),
        "a function removed above it is not"
    );
    assert!(lines.edge((13, 14), 0), "its body lost its last statement");
    assert!(
        !lines.edge((13, 14), 4),
        "a line no deeper than its own first line was not in its body"
    );
    assert!(!lines.edge((1, 3), 0) && !lines.edge((6, 8), 0));
    assert!(lines.edited() && !Lines::default().edited());
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
    assert_eq!(
        lib.lines,
        lines(&[(2, 2), (9, 9)], &[removal(6, Some(4), true)])
    );
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
