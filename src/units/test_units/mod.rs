//! Test quality: one request per test for value checks and one per candidate
//! redundant pair. A test whose value stays undecided is asked again with the
//! bodies of the functions it calls and its file's imports, mocks and setup;
//! an undecided pair, with the body of the function both tests call. `setup`
//! reads what runs before a test from its file's text, and `pairs` plans the
//! redundant pairs.
use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, Questions, TEST_PACK_ITEMS, UnitPlan,
    compact, identity, pack, questions, unique_ids,
};
use crate::{
    analysis::test_map::{self, TestCase},
    catalog::{TEST_REDUNDANCY, TEST_VALUE},
    schema::Pass,
    units::questions::TestEvidence,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
};

mod pairs;
mod setup;
pub(in crate::units) use pairs::*;
pub(in crate::units) use setup::*;

const SUBJECTS: usize = 16;
/// Subjects whose bodies the recheck shows, in the order the test calls them.
const SOURCED_SUBJECTS: usize = 4;
/// A longer body is left out rather than cut; its signature stays.
const SUBJECT_SOURCE_BYTES: usize = 4000;
/// A callable's file and full source, for the recheck.
pub(super) struct SubjectSource {
    pub path: PathBuf,
    pub source: String,
    /// Whether other files can call it: false for a Ruby helper defined in a
    /// file of test cases.
    pub shared: bool,
}

/// What the scope knows about the functions tests call.
pub(super) struct Subjects<'a> {
    pub signatures: &'a BTreeMap<String, String>,
    pub sources: &'a BTreeMap<String, SubjectSource>,
    /// Ruby test helpers by short name, for the recheck's setup.
    pub helpers: &'a BTreeMap<String, Vec<SubjectSource>>,
    /// Source hashes of selected and context files, for freshness checks.
    pub hashes: &'a BTreeMap<PathBuf, String>,
    /// Controller methods by full name, with the route that reaches each,
    /// such as `GET /owners/{ownerId}`.
    pub routes: &'a BTreeMap<String, String>,
}

fn subject_state(names: &[&String], subjects: &BTreeMap<String, String>) -> Vec<Value> {
    let mut seen = Vec::new();
    for name in names {
        if seen.len() < SUBJECTS && !seen.contains(name) {
            seen.push(*name);
        }
    }
    seen.iter()
        .map(|name| json!({"name": name, "signature": subjects.get(*name).cloned().unwrap_or_default()}))
        .collect()
}

pub(super) fn plan_values(
    file: &FileContext<'_>,
    cases: &[TestCase],
    subjects: &Subjects<'_>,
    test_lines: &[Range<usize>],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let ids = unique_ids("test", cases.iter().map(|c| c.name.as_str()));
    let (setup, head) = cases
        .first()
        .map(|first| {
            let region = test_lines
                .iter()
                .find(|r| r.contains(&first.line))
                .map_or(1, |r| r.start);
            let lines: Vec<&str> = file.source.lines().collect();
            let start = region.saturating_sub(1).min(lines.len());
            (
                file_setup(file.source, region, first.line),
                setup_head(&lines, start, first.line),
            )
        })
        .unwrap_or_default();
    let ruby = file.path.extension().is_some_and(|e| e == "rb");
    let mut items = Vec::new();
    for (case, id) in cases.iter().zip(ids) {
        let source = case.source(file.source);
        // A Ruby case gets the setup its groups declare for it, not every
        // hook of the file (an RSpec file's groups often set up differently),
        // and the test helpers it and its hooks call.
        let (own, helper_paths) = if ruby {
            ruby_setup(file, case, head.clone(), subjects.helpers)
        } else {
            (setup.clone(), Vec::new())
        };
        let evidence = value_evidence(file, case, subjects, &own, &helper_paths);
        let recheck = value_recheck(file, &id, &evidence);
        let confirm = (!reaches_past_visibility(source))
            .then(|| value_confirm(file, &id, &evidence))
            .flatten();
        out.units.push(UnitPlan {
            rule: TEST_VALUE,
            id: id.clone(),
            name: case.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(case.line, case.end_line, Some(&case.name))],
            quote: None,
            lines: case.end_line + 1 - case.line,
            identity: identity(&[&case.name, &compact(source)]),
            detail: Detail::Test {
                confirm: confirm.map(Into::into),
            },
            recheck: recheck.map(Into::into),
        });
        items.push((out.units.len() - 1, id, case, test_item(case, source, ruby)));
    }
    for group in pack(items, TEST_PACK_ITEMS, |(_, _, _, item)| item) {
        let (request, asked) = value_request(file, &group, subjects.signatures);
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        } else {
            for (unit, ..) in group {
                out.units[unit].presence = Presence::NeedsContext;
                out.units[unit].recheck = None;
                out.units[unit].detail = Detail::Test { confirm: None };
            }
        }
    }
}

/// One test with the bodies of the functions it calls and its file's setup,
/// and the files they come from: the evidence of its recheck and confirm.
struct Evidence {
    state: Value,
    sources: Vec<(PathBuf, String)>,
    /// Whether it adds a body or setup to what the first pass showed.
    adds: bool,
    ruby: bool,
}

impl Evidence {
    fn request(&self, file: &FileContext<'_>, stage: &str, questions: Questions) -> (Value, Asked) {
        let paths: Vec<(&Path, &str)> = self
            .sources
            .iter()
            .map(|(path, hash)| (path.as_path(), hash.as_str()))
            .collect();
        super::request(file.model, stage, &paths, self.state.clone(), questions)
    }
}

fn value_evidence(
    file: &FileContext<'_>,
    case: &TestCase,
    subjects: &Subjects<'_>,
    setup: &str,
    setup_paths: &[PathBuf],
) -> Evidence {
    let mut sources = vec![(file.path.to_path_buf(), file.source_hash.to_string())];
    for path in setup_paths {
        if let Some(hash) = subjects.hashes.get(path)
            && !sources.iter().any(|(known, _)| known == path)
        {
            sources.push((path.clone(), hash.clone()));
        }
    }
    let listed = sourced_subjects(case, subjects, &mut sources);
    let sourced = listed.iter().any(|s| s.get("source").is_some());
    let ruby = file.path.extension().is_some_and(|e| e == "rb");
    let state = json!({
        "file": file.plain_state(),
        "tests": [test_item(case, case.source(file.source), ruby)],
        "subjects": listed,
        "setup": setup,
    });
    Evidence {
        state,
        sources,
        adds: sourced || !setup.is_empty(),
        ruby,
    }
}

/// The hollow-test questions again for one test, with the bodies of the
/// functions it calls and its file's setup; none when there is nothing to add.
fn value_recheck(file: &FileContext<'_>, id: &str, evidence: &Evidence) -> Option<(Value, Asked)> {
    if !evidence.adds {
        return None;
    }
    let (request, asked) = evidence.request(file, "recheck", recheck_questions(id, evidence.ruby));
    file.budget.fits(&request).then_some((request, asked))
}

/// Calls that reach past a language's visibility: reflection, a cast to
/// `any`, Ruby's `send(:…)` and `instance_variable_get`.
const BYPASSES: [&str; 12] = [
    "ReflectionClass",
    "ReflectionProperty",
    "ReflectionMethod",
    "setAccessible(",
    "getDeclaredField(",
    "getDeclaredMethod(",
    "BindingFlags.NonPublic",
    "Whitebox.",
    "ReflectionTestUtils.",
    "as any)",
    "instance_variable_get",
    ".send(:",
];

/// Whether a test reads or calls members past its language's visibility. It
/// reads internals by the language's own definition, so an internal-details
/// consider on it is not asked what its assertions read: 4 of the 6 labeled
/// tests that did so were right, and the question read two reflected private
/// properties and two `(service as any)` fields as results or state.
fn reaches_past_visibility(source: &str) -> bool {
    BYPASSES.iter().any(|b| source.contains(b))
}

/// What the test's assertions read, with the same evidence as its recheck.
fn value_confirm(file: &FileContext<'_>, id: &str, evidence: &Evidence) -> Option<(Value, Asked)> {
    let mut questions = Questions::default();
    let kind = if evidence.ruby {
        TestEvidence::RecheckGroups
    } else {
        TestEvidence::Recheck
    };
    questions.ask(
        "reads".into(),
        questions::test_reads("tests[0].source", kind),
        id,
        TEST_VALUE,
        "reads",
        Pass::Locate,
    );
    let (request, asked) = evidence.request(file, "locate", questions);
    file.budget.fits(&request).then_some((request, asked))
}

/// The functions a test calls, with the route of a controller method it
/// reaches through a request and the bodies of the first few; each body's
/// file joins `sources`.
fn sourced_subjects(
    case: &TestCase,
    subjects: &Subjects<'_>,
    sources: &mut Vec<(PathBuf, String)>,
) -> Vec<Value> {
    let names: Vec<&String> = case.subjects.iter().collect();
    let mut listed = subject_state(&names, subjects.signatures);
    // A controller method the test reaches through a request, not a call.
    for subject in &mut listed {
        if let Some(route) = subject["name"]
            .as_str()
            .and_then(|name| subjects.routes.get(name))
        {
            subject["route"] = json!(route);
        }
    }
    for subject in listed.iter_mut().take(SOURCED_SUBJECTS) {
        let Some(found) = subject["name"]
            .as_str()
            .and_then(|name| subjects.sources.get(name))
            .filter(|found| found.source.len() <= SUBJECT_SOURCE_BYTES)
        else {
            continue;
        };
        let Some(hash) = subjects.hashes.get(&found.path) else {
            continue;
        };
        subject["source"] = json!(found.source);
        if !sources.iter().any(|(path, _)| *path == found.path) {
            sources.push((found.path.clone(), hash.clone()));
        }
    }
    listed
}

/// The hollow-test questions of a recheck; a Ruby test's name the setup its
/// groups declare for it.
fn recheck_questions(id: &str, ruby: bool) -> Questions {
    let evidence = if ruby {
        TestEvidence::RecheckGroups
    } else {
        TestEvidence::Recheck
    };
    let mut questions = Questions::default();
    let path = "tests[0].source";
    for (question, body) in [
        ("own_logic", questions::test_own_logic(path, evidence)),
        ("mock_only", questions::test_mock_only(path, evidence)),
    ] {
        questions.ask(
            question.into(),
            body,
            id,
            TEST_VALUE,
            question,
            Pass::Recheck,
        );
    }
    questions
}

/// A test as sent: its name and source, and for Ruby the groups it is
/// declared in. An RSpec example reads as a sentence that continues its
/// groups (`describe Registry` … `it "finds a registered object"`), and the
/// outer group often names the class under test.
pub(super) fn test_item(case: &TestCase, source: &str, ruby: bool) -> Value {
    let mut item = json!({"name": case.name, "source": source});
    if ruby && !case.suite.is_empty() {
        item["suite"] = json!(case.suite.join(" > "));
    }
    item
}

/// One test-value request: four questions per test, with the signatures the tests call.
fn value_request(
    file: &FileContext<'_>,
    group: &[(usize, String, &TestCase, Value)],
    subjects: &BTreeMap<String, String>,
) -> (Value, Asked) {
    let mut questions = Questions::default();
    for (index, (_, id, _, _)) in group.iter().enumerate() {
        let path = format!("tests[{index}].source");
        for (question, body) in [
            ("internal", questions::test_internal(&path)),
            (
                "own_logic",
                questions::test_own_logic(&path, TestEvidence::First),
            ),
            (
                "mock_only",
                questions::test_mock_only(&path, TestEvidence::First),
            ),
            ("several", questions::test_several(&path)),
        ] {
            questions.ask(
                format!("t{index}_{question}"),
                body,
                id,
                TEST_VALUE,
                question,
                Pass::First,
            );
        }
    }
    let names: Vec<&String> = group
        .iter()
        .flat_map(|(_, _, case, _)| case.subjects.iter())
        .collect();
    let state = json!({
        "file": file.plain_state(),
        "tests": group.iter().map(|(_, _, _, item)| item.clone()).collect::<Vec<_>>(),
        "subjects": subject_state(&names, subjects),
    });
    file.request("tests", state, questions)
}

/// A test's source as its words: a space inside a string, such as
/// `x:=` against `x := `, can be what two tests differ in.
fn words(source: &str) -> Vec<&str> {
    source.split_whitespace().collect()
}
