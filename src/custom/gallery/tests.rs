use super::*;
use crate::tests::Project;

/// The repository's `gallery/` and docs, read where Cargo builds from.
fn repository(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn entry(name: &str) -> &'static Entry {
    ENTRIES.iter().find(|entry| entry.name == name).unwrap()
}

fn written(project: &Project, name: &str) -> String {
    fs::read_to_string(project.0.join(format!(".jevgate/questions/{name}.toml"))).unwrap()
}

/// `rules add` of `names` in `project`, whose `jevgate.toml` defines no question.
fn added(project: &Project, names: &[&str], force: bool) -> Result<Vec<(Question, bool)>> {
    let names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
    add(&project.0, &[], &names, force)
}

/// Why `rules add` refused.
fn refused(result: Result<Vec<(Question, bool)>>) -> String {
    result.expect_err("refused").to_string()
}

#[test]
fn every_gallery_file_is_compiled_in_and_the_other_way_round() {
    let mut files: Vec<String> = fs::read_dir(repository("gallery"))
        .unwrap()
        .map(|file| file.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|name| name.strip_suffix(".toml").map(str::to_string))
        .collect();
    files.sort();
    let mut names: Vec<&str> = ENTRIES.iter().map(|entry| entry.name).collect();
    names.sort_unstable();
    assert_eq!(files, names);
}

#[test]
fn every_gallery_question_is_a_valid_question_file_that_says_what_it_catches() {
    for entry in ENTRIES {
        let question = entry.question().unwrap();
        assert_eq!(question.rule, format!("custom/{}", entry.name));
        assert_eq!(
            question.source.to_str().unwrap(),
            format!(".jevgate/questions/{}.toml", entry.name),
            "named as a check names the file on every platform"
        );
        let summary = entry.summary();
        assert!(
            summary.len() > 20 && summary.ends_with('.'),
            "{}: the first line says what it catches: {summary:?}",
            entry.name
        );
        let link = format!(
            "# https://tech-byte-frontier.github.io/jevgate/question-gallery.html#{}",
            entry.name
        );
        assert!(
            entry.text.lines().any(|line| line == link),
            "{} links its numbers",
            entry.name
        );
        let keys: toml::Table = toml::from_str(entry.text).unwrap();
        assert!(
            !keys.contains_key("id"),
            "{}: the file name is the id",
            entry.name
        );
        assert!(
            keys.contains_key("threshold") && keys.contains_key("level"),
            "{}: the knobs a team tunes are written out",
            entry.name
        );
        assert!(entry.text.is_ascii(), "{}", entry.name);
    }
}

#[test]
fn the_docs_page_shows_every_gallery_question_from_its_file() {
    let page = fs::read_to_string(repository("site/src/question-gallery.md")).unwrap();
    for entry in ENTRIES {
        let heading = format!("## {}", entry.name);
        assert!(
            page.lines().any(|line| line == heading),
            "{} has a section",
            entry.name
        );
        assert!(
            page.contains(&format!(
                "{{{{#include ../../gallery/{}.toml}}}}",
                entry.name
            )),
            "{}'s section includes the file itself, so the page cannot drift from it",
            entry.name
        );
    }
    let summary = fs::read_to_string(repository("site/src/SUMMARY.md")).unwrap();
    assert!(summary.contains("(question-gallery.md)"));
}

#[test]
fn adding_writes_the_file_as_the_gallery_has_it_and_keeps_it_tracked() {
    let project = Project::new();
    let outcome = added(&project, &["swallowed-errors"], false).unwrap();
    assert_eq!(outcome.len(), 1);
    assert_eq!(outcome[0].0.rule, "custom/swallowed-errors");
    assert!(outcome[0].1, "written");
    assert_eq!(
        written(&project, "swallowed-errors"),
        entry("swallowed-errors").text
    );
    assert_eq!(
        fs::read_to_string(project.0.join(".jevgate/.gitignore")).unwrap(),
        crate::storage::IGNORE,
        "the ignore file that keeps questions/ tracked"
    );
    let loaded = super::super::load(
        &project.0,
        (&project.0.join("jevgate.toml"), &[]),
        Some(&super::super::directory(&project.0)),
    )
    .unwrap();
    assert_eq!(loaded[0].rule, "custom/swallowed-errors");
    assert_eq!(loaded[0].version, outcome[0].0.version);
}

#[test]
fn adding_again_keeps_the_same_file_and_refuses_an_edited_one_without_force() {
    let project = Project::new();
    added(&project, &["n-plus-one"], false).unwrap();
    let again = added(&project, &["n-plus-one"], false).unwrap();
    assert!(!again[0].1, "the same text is left as it is");
    let path = project.0.join(".jevgate/questions/n-plus-one.toml");
    let edited = entry("n-plus-one")
        .text
        .replace("level = \"consider\"", "level = \"review\"");
    fs::write(&path, &edited).unwrap();
    let error = refused(added(&project, &["n-plus-one"], false));
    assert!(
        error.contains("differs from the gallery's; --force replaces it"),
        "{error}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), edited, "kept");
    assert!(added(&project, &["n-plus-one"], true).unwrap()[0].1);
    assert_eq!(written(&project, "n-plus-one"), entry("n-plus-one").text);
}

#[test]
fn every_name_is_checked_before_any_file_is_written() {
    let project = Project::new();
    let config: Config = toml::from_str(
        "[[question]]\nid = \"n-plus-one\"\nquestion = \"Does it query in a loop?\"\nunit = \"function\"\n",
    )
    .unwrap();
    let names = ["resource-leak".to_string(), "n-plus-one".to_string()];
    for force in [false, true] {
        let error = refused(add(&project.0, &config.question, &names, force));
        assert!(
            error.contains("custom/n-plus-one is already defined in jevgate.toml"),
            "{error}"
        );
    }
    let error = refused(added(&project, &["resource-leak", "nope"], false));
    assert!(error.contains("No gallery question nope"), "{error}");
    assert!(
        !project.0.join(".jevgate").exists(),
        "nothing is written when a name is refused"
    );
}

#[test]
fn a_name_given_twice_is_added_once() {
    let project = Project::new();
    let twice = added(&project, &["thin-handlers", "thin-handlers"], false).unwrap();
    assert_eq!(twice.len(), 1);
}

#[cfg(unix)]
#[test]
fn force_replaces_a_link_and_never_writes_through_it() {
    let project = Project::new();
    let outside = Project::new();
    let target = outside.0.join("elsewhere.toml");
    fs::write(&target, "kept").unwrap();
    fs::create_dir_all(project.0.join(".jevgate/questions")).unwrap();
    let link = project.0.join(".jevgate/questions/resource-leak.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let error = refused(added(&project, &["resource-leak"], false));
    assert!(error.contains("differs from the gallery's"), "{error}");
    added(&project, &["resource-leak"], true).unwrap();
    assert!(!link.is_symlink());
    assert_eq!(
        written(&project, "resource-leak"),
        entry("resource-leak").text
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "kept");
}

#[cfg(unix)]
#[test]
fn a_linked_questions_directory_is_refused() {
    let project = Project::new();
    let outside = Project::new();
    fs::create_dir(project.0.join(".jevgate")).unwrap();
    std::os::unix::fs::symlink(&outside.0, project.0.join(".jevgate/questions")).unwrap();
    let error = refused(added(&project, &["todo-without-owner"], false));
    assert!(error.contains("must be a real directory"), "{error}");
    assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);
}
