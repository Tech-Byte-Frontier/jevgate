//! A repeat's fingerprint, which names its copies, and a baseline written by
//! JevGate 0.35, which still accepts findings through the fingerprints they
//! had then until the next baseline write rewrites it.
use super::{
    changed::{copies_found, load_copy},
    duplicates::LOAD,
    *,
};
use crate::{baseline, options::Disposition};
use std::path::Path;

/// Shared-logic and function-simplification arguments, for the whole project.
fn whole() -> CheckArgs {
    let mut options = args();
    options.rules = vec![
        catalog::SHARED_LOGIC.into(),
        catalog::FUNCTION_SIMPLIFICATION.into(),
    ];
    options
}

/// `LOAD` in `a.rs` and its copy loading a team in `b.rs`.
fn copied() -> Project {
    let project = Project::new();
    project.write("a.rs", LOAD);
    project.write("b.rs", &copy("load_team", "\"title\""));
    project
}

fn copy(what: &str, field: &str) -> String {
    load_copy(what, field, String::new(), (false, false))
}

/// `jevgate baseline mark REASON TARGET`.
fn mark(project: &Project, reason: Disposition, target: String) -> usize {
    baseline::mark(
        &project.0,
        &baseline::Mark {
            reason,
            note: None,
            targets: &[target],
            rules: &[],
        },
    )
    .unwrap()
}

fn target(path: &Path, finding: &crate::schema::Finding) -> String {
    format!("{}:{}", path.display(), finding.line)
}

#[test]
fn a_repeat_has_one_fingerprint_whichever_copy_a_check_selects() {
    let project = copied();
    let all = run(&project, &whole(), &mut scripted(2));
    let found = copies_found(&all);
    assert_eq!(found.len(), 1, "{found:?}");
    let (owner, repeat) = found[0];
    assert_eq!(owner, Path::new("a.rs"));
    assert_eq!(
        repeat.identity.members,
        ["a.rs::load_user", "b.rs::load_team"]
    );
    // A check of `b.rs` reading `a.rs` as context owns the repeat in `b.rs`.
    let mut from_b = whole();
    from_b.paths = vec!["b.rs".into()];
    from_b.context = vec!["a.rs".into()];
    let report = run(&project, &from_b, &mut scripted(2));
    let found = copies_found(&report);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, Path::new("b.rs"));
    assert_eq!(found[0].1.fingerprint, repeat.fingerprint);
    assert_ne!(
        found[0].1.identity.v1, repeat.identity.v1,
        "JevGate 0.35 identified it by the copy the check selected"
    );
}

/// A repeat stays accepted while every copy is one an accepted repeat
/// names, and is asked about again when a new copy joins it.
#[test]
fn a_repeat_a_new_copy_joins_is_asked_about_again_and_one_a_copy_leaves_is_not() {
    let project = copied();
    accept_all(&project, &run(&project, &whole(), &mut scripted(2)));
    project.write("c.rs", &copy("load_org", "\"label\""));
    let report = run(&project, &whole(), &mut scripted(2));
    let found = copies_found(&report);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].1.identity.members.len(), 3);
    assert!(!found[0].1.baselined, "a copy no entry names joined");
    crate::storage::Store::open(&project.0)
        .unwrap()
        .publish(&report)
        .unwrap();
    assert_eq!(
        mark(
            &project,
            Disposition::Intended,
            target(found[0].0, found[0].1)
        ),
        1
    );
    std::fs::remove_file(project.0.join("b.rs")).unwrap();
    let report = run(&project, &whole(), &mut scripted(2));
    let found = copies_found(&report);
    assert_eq!(
        found[0].1.identity.members,
        ["a.rs::load_user", "c.rs::load_org"]
    );
    assert!(found[0].1.baselined, "both copies are accepted ones");
}

/// The repeat of [`copied`] and a long function in `c.rs`, committed, with
/// a baseline as JevGate 0.35 wrote it: the repeat accepted under the
/// fingerprint it had then, for later, with a note. Returns the check's
/// report.
fn upgraded() -> (Project, Report) {
    let project = copied();
    project.write("c.rs", &long_function("f"));
    let report = run(&project, &whole(), &mut scripted(2));
    let (_, repeat) = copies_found(&report)[0];
    let v1 = repeat.identity.v1.clone().unwrap();
    assert_ne!(v1, repeat.fingerprint);
    let old = json!({
        "version": 1,
        "created_at": 1,
        "findings": [{
            "fingerprint": v1,
            "rule": repeat.rule,
            "path": "a.rs",
            "line": repeat.line,
            "message": "Old wording.",
            "reason": "later",
            "note": "#12",
        }],
    });
    project.write(
        baseline::BASELINE_FILE,
        &serde_json::to_string_pretty(&old).unwrap(),
    );
    project.commit_all();
    let report = run(&project, &whole(), &mut scripted(2));
    crate::storage::Store::open(&project.0)
        .unwrap()
        .publish(&report)
        .unwrap();
    (project, report)
}

/// The fingerprint, reason and note of each entry of the baseline.
fn entries(project: &Project) -> Vec<(String, Option<Disposition>, Option<String>)> {
    baseline::list(&project.0, &[], &[])
        .unwrap()
        .into_iter()
        .map(|l| (l.fingerprint, l.reason, l.note))
        .collect()
}

#[test]
fn an_old_baseline_accepts_through_earlier_fingerprints_until_a_write_rewrites_it() {
    let (project, report) = upgraded();
    let (_, repeat) = copies_found(&report)[0];
    assert!(repeat.baselined, "accepted through its v1 fingerprint");
    let function = report
        .files
        .iter()
        .find(|f| f.path == Path::new("c.rs"))
        .map(|f| &f.findings[0])
        .unwrap();
    assert!(!function.baselined);
    // Marking another finding rewrites the old entry, keeping its reason
    // and note; the turn's dismissals are the mark alone.
    assert_eq!(
        mark(
            &project,
            Disposition::Wrong,
            target(Path::new("c.rs"), function)
        ),
        1
    );
    assert_eq!(
        entries(&project),
        [
            (
                repeat.fingerprint.clone(),
                Some(Disposition::Later),
                Some("#12".into())
            ),
            (function.fingerprint.clone(), Some(Disposition::Wrong), None),
        ]
    );
    let dismissals = baseline::Dismissals::since(&project.0, "HEAD").unwrap();
    assert_eq!(dismissals.of(repeat), None, "accepted as the turn began");
    assert_eq!(
        dismissals.of(function).map(|d| d.reason),
        Some(Disposition::Wrong)
    );
    assert_eq!(
        crate::gate::exit_code(&run(&project, &whole(), &mut scripted(2))),
        0
    );
}

#[test]
fn writing_the_baseline_over_an_old_one_moves_each_entry_to_its_current_fingerprint() {
    for merge in [false, true] {
        let (project, report) = upgraded();
        let (_, repeat) = copies_found(&report)[0];
        baseline::write(&project.0, merge, None).unwrap();
        let rewritten = entries(&project);
        assert_eq!(rewritten.len(), 2, "merge {merge}: no v1 entry is kept");
        assert!(
            rewritten.contains(&(
                repeat.fingerprint.clone(),
                Some(Disposition::Later),
                Some("#12".into())
            )),
            "merge {merge}: {rewritten:?}"
        );
        let text = std::fs::read_to_string(project.0.join(baseline::BASELINE_FILE)).unwrap();
        assert!(
            text.contains("\"members\""),
            "a repeat's entry names its copies"
        );
    }
}
