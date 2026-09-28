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
    let project = Project::new();
    project.write("pkg/lib.rs", LIB);
    project.write("other/lib.rs", LIB);
    project.commit_all();
    project.write("pkg/lib.rs", &LIB.replace("one", "uno"));
    project.write("other/lib.rs", &LIB.replace("one", "uno"));
    let root = project.0.join("pkg");
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
