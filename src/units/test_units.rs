//! Test quality: one request per test for value checks and one per candidate
//! redundant pair. A test whose value stays undecided is asked again with the
//! bodies of the functions it calls and its file's imports, mocks and setup.
use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, Questions, TEST_PACK_ITEMS, UnitPlan,
    compact, identity, pack, questions, unique_ids,
};
use crate::{
    analysis::test_map::{self, TestCase},
    catalog::{TEST_REDUNDANCY, TEST_VALUE},
    schema::Pass,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
};

const SUBJECTS: usize = 16;
/// Subjects whose bodies the recheck shows, in the order the test calls them.
const SOURCED_SUBJECTS: usize = 4;
/// A longer body is left out rather than cut; its signature stays.
const SUBJECT_SOURCE_BYTES: usize = 4000;
/// Setup text before the first test, and each setup hook, above these sizes
/// is left out rather than cut.
const SETUP_BYTES: usize = 4000;
const HOOK_BYTES: usize = 1500;

/// A callable's file and full source, for the recheck.
pub(super) struct SubjectSource {
    pub path: PathBuf,
    pub source: String,
}

/// What the scope knows about the functions tests call.
pub(super) struct Subjects<'a> {
    pub signatures: &'a BTreeMap<String, String>,
    pub sources: &'a BTreeMap<String, SubjectSource>,
    /// Source hashes of selected and context files, for freshness checks.
    pub hashes: &'a BTreeMap<PathBuf, String>,
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
    let setup = cases
        .first()
        .map(|first| {
            let region = test_lines
                .iter()
                .find(|r| r.contains(&first.line))
                .map_or(1, |r| r.start);
            file_setup(file.source, region, first.line)
        })
        .unwrap_or_default();
    let mut items = Vec::new();
    for (case, id) in cases.iter().zip(ids) {
        let source = case.source(file.source);
        let recheck = value_recheck(file, case, &id, subjects, &setup);
        out.units.push(UnitPlan {
            rule: TEST_VALUE,
            id: id.clone(),
            name: case.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(case.line, case.end_line, Some(&case.name))],
            quote: None,
            lines: case.end_line + 1 - case.line,
            identity: identity(&[&case.name, &compact(source)]),
            detail: Detail::Test,
            recheck,
        });
        items.push((
            out.units.len() - 1,
            id,
            case,
            json!({"name": case.name, "source": source}),
        ));
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
            }
        }
    }
}

/// The hollow-test questions again for one test, with the bodies of the
/// functions it calls and its file's setup; none when there is nothing to add.
fn value_recheck(
    file: &FileContext<'_>,
    case: &TestCase,
    id: &str,
    subjects: &Subjects<'_>,
    setup: &str,
) -> Option<(Value, Asked)> {
    let mut sources = vec![(file.path.to_path_buf(), file.source_hash.to_string())];
    let names: Vec<&String> = case.subjects.iter().collect();
    let mut listed = subject_state(&names, subjects.signatures);
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
    let sourced = listed.iter().any(|s| s.get("source").is_some());
    if !sourced && setup.is_empty() {
        return None;
    }
    let mut questions = Questions::default();
    let path = "tests[0].source";
    for (question, body) in [
        ("own_logic", questions::test_own_logic(path, true)),
        ("mock_only", questions::test_mock_only(path, true)),
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
    let state = json!({
        "file": file.file_state(),
        "tests": [{"name": case.name, "source": case.source(file.source)}],
        "subjects": listed,
        "setup": setup,
    });
    let paths: Vec<(&Path, &str)> = sources
        .iter()
        .map(|(path, hash)| (path.as_path(), hash.as_str()))
        .collect();
    let (request, asked) = super::request(file.model, "recheck", &paths, state, questions);
    file.budget.fits(&request).then_some((request, asked))
}

/// A test file's shared setup: the text of its test region before the first
/// test or suite (imports, mocks, fixtures), then each setup hook. A part
/// larger than its limit is left out rather than cut.
pub(super) fn file_setup(source: &str, region_start: usize, first_case: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let start = region_start.saturating_sub(1).min(lines.len());
    let mut parts: Vec<String> = setup_head(&lines, start, first_case).into_iter().collect();
    parts.extend(setup_hooks(&lines[start..].join("\n")));
    let setup = parts.join("\n\n");
    if setup.len() <= SETUP_BYTES + HOOK_BYTES {
        setup
    } else {
        parts.truncate(1);
        parts.join("")
    }
}

/// The lines from `start` up to the first suite, test or test module, when
/// they are short enough to send.
fn setup_head(lines: &[&str], start: usize, first_case: usize) -> Option<String> {
    const OPENERS: [&str; 12] = [
        "describe(",
        "describe.",
        "suite(",
        "context(",
        "test(",
        "test.",
        "it(",
        "it.",
        "def test",
        "class ",
        "mod tests",
        "#[test]",
    ];
    let last = first_case.saturating_sub(1).min(lines.len());
    let end = (start..last)
        .find(|&i| {
            let line = lines[i].trim_start();
            OPENERS.iter().any(|opener| line.starts_with(opener))
        })
        .unwrap_or(last);
    let head = lines[start..end].join("\n");
    (!head.trim().is_empty() && head.len() <= SETUP_BYTES).then(|| head.trim().to_string())
}

/// Every setup hook in `region` short enough to send: `beforeEach`/`beforeAll`
/// calls and Python `setUp`/`setup_method` methods.
fn setup_hooks(region: &str) -> Vec<String> {
    let mut hooks = Vec::new();
    for hook in [
        "beforeEach(",
        "beforeAll(",
        "def setUp(",
        "def setup_method(",
    ] {
        let mut from = 0;
        while let Some(found) = region[from..].find(hook) {
            let at = from + found;
            let text = if hook.starts_with("def ") {
                let line_start = region[..at].rfind('\n').map_or(0, |i| i + 1);
                indented_block(&region[at..], at - line_start)
            } else {
                balanced_call(&region[at..])
            };
            if text.len() <= HOOK_BYTES {
                hooks.push(text.to_string());
            }
            from = at + hook.len();
        }
    }
    hooks
}

/// A call from its name through the parenthesis that closes it.
fn balanced_call(text: &str) -> &str {
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return &text[..i + 1];
                }
            }
            _ => {}
        }
    }
    text
}

/// A Python definition line, indented `base` columns, and the lines indented below it.
fn indented_block(text: &str, base: usize) -> &str {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return text;
    };
    let mut end = first.len();
    let indent = |line: &str| line.len() - line.trim_start().len();
    for line in lines {
        if !line.trim().is_empty() && indent(line) <= base {
            break;
        }
        end += line.len();
    }
    text[..end].trim_end()
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
            ("own_logic", questions::test_own_logic(&path, false)),
            ("mock_only", questions::test_mock_only(&path, false)),
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
        "file": file.file_state(),
        "tests": group.iter().map(|(_, _, _, item)| item.clone()).collect::<Vec<_>>(),
        "subjects": subject_state(&names, subjects),
    });
    file.request("tests", state, questions)
}

pub(super) fn plan_pairs(
    file: &FileContext<'_>,
    cases: &[TestCase],
    subjects: &BTreeMap<String, String>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (pairs, omitted) = test_map::pairs(cases);
    out.rules.insert(TEST_REDUNDANCY, omitted);
    for pair in pairs {
        let (a, b) = (&cases[pair.a], &cases[pair.b]);
        let id = format!("test-pair:{}|{}", a.name, b.name);
        let mut questions = Questions::default();
        for (question, body) in [
            ("overlap", questions::test_pair_overlap()),
            ("same_input", questions::test_pair_same_input()),
            ("same_outcome", questions::test_pair_same_outcome()),
        ] {
            questions.ask(
                question.into(),
                body,
                &id,
                TEST_REDUNDANCY,
                question,
                Pass::First,
            );
        }
        let state = json!({
            "test_a": {"name": a.name, "source": a.source(file.source)},
            "test_b": {"name": b.name, "source": b.source(file.source)},
            "subject": subject_state(&[&pair.subject], subjects).remove(0),
        });
        let (request, asked) = file.request("test-pair", state, questions);
        let fits = file.budget.fits(&request);
        out.units.push(UnitPlan {
            rule: TEST_REDUNDANCY,
            id,
            name: format!("`{}` and `{}`", a.name, b.name),
            presence: if fits {
                Presence::Judged
            } else {
                Presence::NeedsContext
            },
            locations: vec![
                file.location(a.line, a.end_line, Some(&a.name)),
                file.location(b.line, b.end_line, Some(&b.name)),
            ],
            quote: None,
            lines: a.end_line + 1 - a.line + b.end_line + 1 - b.line,
            identity: identity(&[
                &a.name,
                &b.name,
                &compact(a.source(file.source)),
                &compact(b.source(file.source)),
            ]),
            detail: Detail::TestPair {
                names: [a.name.clone(), b.name.clone()],
                subject: pair.subject.clone(),
            },
            recheck: None,
        });
        if fits {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        }
    }
}
