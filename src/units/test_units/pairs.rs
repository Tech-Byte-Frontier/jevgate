//! Redundant test pairs: one request per candidate pair, and the recheck of a
//! pair whose overlap stays undecided, with the body of the function both call.
use super::*;

pub(in crate::units) fn plan_pairs(
    file: &FileContext<'_>,
    (cases, table): (&[TestCase], bool),
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
        let subject = subject_state(&[&pair.subject], subjects.signatures).remove(0);
        let state = pair_state(file, (a, b), subject, ruby);
        let recheck = pair_recheck(file, &id, &state, &pair.subject, subjects, ruby);
        let identical = a.suite == b.suite
            && words(&a.source(file.source).replace(a.name.as_str(), ""))
                == words(&b.source(file.source).replace(b.name.as_str(), ""));
        // Tests that read the same apart from their names check nothing apart.
        let confirm = (!ruby && !identical).then(|| {
            let mut questions = Questions::default();
            questions.ask(
                "distinct".into(),
                questions::test_pair_distinct(),
                &id,
                TEST_REDUNDANCY,
                "distinct",
                Pass::Locate,
            );
            file.request("locate", state.clone(), questions)
        });
        let fits = file.push_fitting(
            file.request("test-pair", state, pair_questions(&id, ruby)),
            requests,
        );
        out.units.push(UnitPlan {
            rule: TEST_REDUNDANCY,
            id,
            name: format!("`{}` and `{}`", a.name, b.name),
            presence: Presence::judged_if(fits),
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
                table,
                unseen_setup: !ruby && a.suite != b.suite,
                identical,
                confirm: confirm
                    .filter(|(request, _)| fits && file.budget.fits(request))
                    .map(Into::into),
            },
            recheck: recheck.filter(|_| fits).map(Into::into),
        });
    }
}

/// The first-pass questions of a test pair; Ruby pairs are also asked
/// whether each test checks something the other does not.
pub(super) fn pair_questions(id: &str, ruby: bool) -> Questions {
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
            id,
            TEST_REDUNDANCY,
            question,
            Pass::First,
        );
    }
    questions
}

/// Both tests of a pair with their subject.
pub(super) fn pair_state(
    file: &FileContext<'_>,
    (a, b): (&TestCase, &TestCase),
    subject: Value,
    ruby: bool,
) -> Value {
    let mut state = json!({
        "test_a": {"name": a.name, "source": a.source(file.source)},
        "test_b": {"name": b.name, "source": b.source(file.source)},
        "subject": subject,
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
    state
}

/// The overlap question again for an undecided pair, with the body of the
/// function both tests call: whether a call throws before the rest of a test
/// runs, or which inputs it tells apart, is in that body. None when the body
/// is unknown or too long.
pub(super) fn pair_recheck(
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
    file.fitting(crate::units::request(
        file.model, "recheck", &paths, state, questions,
    ))
}
