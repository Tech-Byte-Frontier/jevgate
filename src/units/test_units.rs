//! Test quality: one request per test for value checks and one per candidate
//! redundant pair. A test whose value stays undecided is asked again with the
//! bodies of the functions it calls and its file's imports, mocks and setup;
//! an undecided pair, with the body of the function both tests call.
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
        let recheck = value_recheck(file, case, &id, subjects, &own, &helper_paths);
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
    setup_paths: &[PathBuf],
) -> Option<(Value, Asked)> {
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
    if !sourced && setup.is_empty() {
        return None;
    }
    let ruby = file.path.extension().is_some_and(|e| e == "rb");
    let state = json!({
        "file": file.plain_state(),
        "tests": [test_item(case, case.source(file.source), ruby)],
        "subjects": listed,
        "setup": setup,
    });
    let paths: Vec<(&Path, &str)> = sources
        .iter()
        .map(|(path, hash)| (path.as_path(), hash.as_str()))
        .collect();
    let questions = recheck_questions(id, ruby);
    let (request, asked) = super::request(file.model, "recheck", &paths, state, questions);
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
fn test_item(case: &TestCase, source: &str, ruby: bool) -> Value {
    let mut item = json!({"name": case.name, "source": source});
    if ruby && !case.suite.is_empty() {
        item["suite"] = json!(case.suite.join(" > "));
    }
    item
}

/// Test helpers shown with one Ruby case, at most.
const HELPERS: usize = 4;

/// A Ruby case's setup: the file's head, the hooks its groups declare, then
/// the test helpers the case and its hooks call, and the helpers those call.
/// Also the other files the helpers come from.
fn ruby_setup(
    file: &FileContext<'_>,
    case: &TestCase,
    head: Option<String>,
    helpers: &BTreeMap<String, Vec<SubjectSource>>,
) -> (String, Vec<PathBuf>) {
    let setup = case_setup(file.source, head, &case.hooks);
    let mut names: Vec<String> = case.calls.iter().chain(&case.hook_calls).cloned().collect();
    let mut shown: Vec<&SubjectSource> = Vec::new();
    let mut next = 0;
    while next < names.len() && shown.len() < HELPERS {
        let name = names[next].clone();
        next += 1;
        let Some(defined) = helpers.get(&name) else {
            continue;
        };
        let found = nearest(file.path, defined);
        let Some(helper) = found.filter(|h| {
            h.source.len() <= HOOK_BYTES
                && !setup.contains(h.source.as_str())
                && !shown.iter().any(|s| s.source == h.source)
        }) else {
            continue;
        };
        shown.push(helper);
        if let Some(tree) = crate::syntax::parse(&helper.path, &helper.source)
            .ok()
            .flatten()
        {
            let mut calls = Vec::new();
            crate::analysis::ruby::called_names(tree.root_node(), &helper.source, &mut calls);
            names.extend(calls);
        }
    }
    let mut parts: Vec<String> = (!setup.is_empty()).then_some(setup).into_iter().collect();
    parts.extend(shown.iter().map(|h| h.source.clone()));
    let paths = shown
        .iter()
        .filter(|h| h.path != file.path)
        .map(|h| h.path.clone())
        .collect();
    (parts.join("\n\n"), paths)
}

/// The one definition of a helper nearest the test: in its own file, else
/// in the support file that shares the most directories with it, at least
/// one, as `test/test_helper.rb` does with `test/routing_test.rb`. None when
/// two definitions are equally near.
pub(super) fn nearest<'a>(test: &Path, defined: &'a [SubjectSource]) -> Option<&'a SubjectSource> {
    let folders = |path: &Path| -> Vec<String> {
        path.parent()
            .into_iter()
            .flat_map(Path::components)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect()
    };
    let own = folders(test);
    let shared = |helper: &SubjectSource| {
        if helper.path == test {
            return usize::MAX;
        }
        own.iter()
            .zip(folders(&helper.path))
            .take_while(|(a, b)| **a == *b)
            .count()
    };
    // A support file in another tree, sharing no directory with the test,
    // serves other tests: `test/test_helper.rb` is not a spec's helper.
    let callable = || {
        defined
            .iter()
            .filter(|h| h.path == test || h.shared && shared(h) > 0)
    };
    let best = callable().map(shared).max()?;
    let mut nearest = callable().filter(|h| shared(h) == best);
    let first = nearest.next();
    nearest.next().is_none().then_some(first).flatten()
}

/// One case's setup: the file's head, then the hooks its groups declare. A
/// hook larger than its limit is left out rather than cut, and so are the
/// hooks when together they are too long.
fn case_setup(source: &str, head: Option<String>, hooks: &[Range<usize>]) -> String {
    let kept = usize::from(head.is_some());
    let mut parts: Vec<String> = head.into_iter().collect();
    parts.extend(
        hooks
            .iter()
            .map(|hook| source[hook.clone()].to_string())
            .filter(|text| text.len() <= HOOK_BYTES),
    );
    let setup = parts.join("\n\n");
    if setup.len() <= SETUP_BYTES + HOOK_BYTES {
        setup
    } else {
        parts.truncate(kept);
        parts.join("")
    }
}

/// The hooks a case's groups declare, as sent beside a pair of tests.
fn hook_text(source: &str, case: &TestCase) -> String {
    case_setup(source, None, &case.hooks)
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

/// The lines from `start` up to the first suite, test, test module or Java
/// setup method, when they are short enough to send. A Java test class's
/// fields, such as its mocks, are part of the head.
fn setup_head(lines: &[&str], start: usize, first_case: usize) -> Option<String> {
    const OPENERS: &[&str] = &[
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
        // Ruby: RSpec groups and examples, and Rails `test "…" do`.
        "describe ",
        "RSpec.describe",
        "context ",
        "it ",
        "test ",
        "module ",
        // Java: setup methods and `@Nested` test classes.
        "@Before",
        "@Nested",
    ];
    let last = first_case.saturating_sub(1).min(lines.len());
    let end = (start..last)
        .find(|&i| {
            let line = lines[i].trim_start();
            // A Python class opens a suite; a braced class holds the fields
            // the tests share.
            OPENERS.iter().any(|opener| line.starts_with(opener))
                && !(line.starts_with("class ") && line.trim_end().ends_with('{'))
        })
        .unwrap_or(last);
    let head = lines[start..end].join("\n");
    (!head.trim().is_empty() && head.len() <= SETUP_BYTES).then(|| head.trim().to_string())
}

/// Every setup hook in `region` short enough to send: `beforeEach`/`beforeAll`
/// calls, Python `setUp`/`setup_method` methods and Java methods annotated
/// `@BeforeEach`, `@BeforeAll`, `@Before` or `@BeforeClass`.
fn setup_hooks(region: &str) -> Vec<String> {
    let mut hooks = Vec::new();
    for (at, _) in region.match_indices("@Before") {
        let name_end = at
            + region[at + 1..]
                .find(|c: char| !c.is_alphanumeric())
                .map_or(region.len() - at, |i| i + 1);
        if matches!(
            &region[at..name_end],
            "@BeforeEach" | "@BeforeAll" | "@Before" | "@BeforeClass"
        ) {
            let text = braced_method(&region[at..]);
            if text.len() <= HOOK_BYTES {
                hooks.push(text.to_string());
            }
        }
    }
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

/// A Java method from its annotation through the brace that closes its body.
fn braced_method(text: &str) -> &str {
    let Some(open) = text.find('{') else {
        return text;
    };
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[..open + i + 1];
                }
            }
            _ => {}
        }
    }
    text
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

pub(super) fn plan_pairs(
    file: &FileContext<'_>,
    cases: &[TestCase],
    subjects: &Subjects<'_>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (pairs, omitted) = test_map::pairs(cases);
    out.rules.insert(TEST_REDUNDANCY, omitted);
    let ruby = file.path.extension().is_some_and(|e| e == "rb");
    for pair in pairs {
        let (a, b) = (&cases[pair.a], &cases[pair.b]);
        let id = format!("test-pair:{}|{}", a.name, b.name);
        let mut questions = Questions::default();
        let distinct = ruby.then(|| ("distinct", questions::test_pair_distinct()));
        for (question, body) in [
            ("overlap", questions::test_pair_overlap(ruby)),
            ("same_input", questions::test_pair_same_input()),
            ("same_outcome", questions::test_pair_same_outcome()),
        ]
        .into_iter()
        .chain(distinct)
        {
            questions.ask(
                question.into(),
                body,
                &id,
                TEST_REDUNDANCY,
                question,
                Pass::First,
            );
        }
        let mut state = json!({
            "test_a": {"name": a.name, "source": a.source(file.source)},
            "test_b": {"name": b.name, "source": b.source(file.source)},
            "subject": subject_state(&[&pair.subject], subjects.signatures).remove(0),
        });
        // Tests in different groups can run on different setup: two RSpec
        // examples that read alike may build different records first.
        if ruby && a.suite != b.suite {
            for (key, case) in [("test_a", a), ("test_b", b)] {
                if !case.suite.is_empty() {
                    state[key]["suite"] = json!(case.suite.join(" > "));
                }
            }
            let (setup_a, setup_b) = (hook_text(file.source, a), hook_text(file.source, b));
            if setup_a != setup_b {
                state["test_a"]["setup"] = json!(setup_a);
                state["test_b"]["setup"] = json!(setup_b);
            }
        }
        let recheck = pair_recheck(file, &id, &state, &pair.subject, subjects, ruby);
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
            recheck: recheck.filter(|_| fits),
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

/// The overlap question again for an undecided pair, with the body of the
/// function both tests call: whether a call throws before the rest of a test
/// runs, or which inputs it tells apart, is in that body. None when the body
/// is unknown or too long.
fn pair_recheck(
    file: &FileContext<'_>,
    id: &str,
    state: &Value,
    subject: &str,
    subjects: &Subjects<'_>,
    ruby: bool,
) -> Option<(Value, Asked)> {
    let found = subjects
        .sources
        .get(subject)
        .filter(|found| found.source.len() <= SUBJECT_SOURCE_BYTES)?;
    let hash = subjects.hashes.get(&found.path)?;
    let mut state = state.clone();
    state["subject"]["source"] = json!(found.source);
    let mut questions = Questions::default();
    // A decisive recheck replaces the first answers, so a Ruby pair is asked
    // again whether each test checks something the other does not.
    let distinct = ruby.then(|| ("distinct", questions::test_pair_distinct()));
    for (question, body) in [("overlap", questions::test_pair_overlap_recheck(ruby))]
        .into_iter()
        .chain(distinct)
    {
        questions.ask(
            question.into(),
            body,
            id,
            TEST_REDUNDANCY,
            question,
            Pass::Recheck,
        );
    }
    let mut paths = vec![(file.path, file.source_hash)];
    if found.path != file.path {
        paths.push((found.path.as_path(), hash.as_str()));
    }
    let (request, asked) = super::request(file.model, "recheck", &paths, state, questions);
    file.budget.fits(&request).then_some((request, asked))
}
