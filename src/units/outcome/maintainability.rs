//! Outcomes of the maintainability rules: functions, file organization, shared logic and hardcoded values.
use super::*;

/// Lines a function may span and still read in one look.
const SHORT_FUNCTION_LINES: usize = 20;

/// The stronger of splitting and (for deeply nested functions only)
/// flattening. Splitting a function of 20 lines or fewer is at most a note:
/// labeled by hand, 12 of 39 such considers were right on the projects used
/// for tuning and 5 of 50 on 23 Bend 2 projects never used for it, most of
/// them helpers that read in one look or dispatches over a token's cases.
pub(in crate::units) fn function_outcome(
    split: Option<&Answer>,
    flatten: Option<&Answer>,
    lines: usize,
) -> Option<Outcome> {
    let split = match benefit(split?) {
        Outcome::Consider(p) if lines <= SHORT_FUNCTION_LINES => Outcome::Note(p),
        outcome => outcome,
    };
    let mut outcomes = vec![split];
    outcomes.extend(flatten.map(benefit));
    Some(strongest(&outcomes))
}

/// The judgment id of the benign-kind check that follows an undecided
/// hardcoded-value question.
pub(in crate::units) fn benign_key(question: &str) -> &'static str {
    match question {
        "environment" => "benign_environment",
        "magic" => "benign_magic",
        _ => "benign_special",
    }
}

/// Each hardcoded-value question of a unit with its outcome and whether an
/// undecided answer was turned into a note by its lean. A file's constants
/// are asked only about the environment.
pub(in crate::units) fn value_signals<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
    detail: &Detail,
    settle: bool,
) -> Option<Vec<(&'static str, Outcome, bool)>> {
    let questions: &[&'static str] = if matches!(detail, Detail::Values { .. }) {
        &["environment", "magic", "special"]
    } else {
        &["environment"]
    };
    questions
        .iter()
        .map(|question| {
            let answer = get(question)?;
            let first = if *question == "special" {
                noul(answer)
            } else {
                benefit(answer)
            };
            let outcome = settled(first, answer, get(benign_key(question)), settle);
            let leaned =
                matches!(first, Outcome::Uncertain(_)) && matches!(outcome, Outcome::Note(_));
            Some((*question, outcome, leaned))
        })
        .collect()
}

pub(in crate::units) fn values_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
    detail: &Detail,
) -> Option<Outcome> {
    let signals = value_signals(get, detail, true)?;
    let outcomes: Vec<Outcome> = signals.iter().map(|(_, o, _)| *o).collect();
    Some(strongest(&outcomes))
}

/// The split Score, weighed with the kind of file once it is asked: the
/// kinds that serve one feature reaching the threshold clear an undecided
/// split or a finding, and the kinds that serve several reaching it raise an
/// undecided split to a consider. The kind is asked of a finding only when
/// the recheck raised it from an undecided first answer: of the 18 such
/// findings the kind cleared on the corpus, 4 were right, and one was the
/// proposal to split JevGate's own planner of one unit per rule.
pub(in crate::units) fn organization_outcome(
    split: Option<&Answer>,
    kind: Option<&Answer>,
) -> Option<Outcome> {
    let outcome = benefit(split?);
    let (
        Outcome::Uncertain(_) | Outcome::Consider(_) | Outcome::Review(_),
        Some(Answer::Choice { probabilities, .. }),
    ) = (outcome, kind)
    else {
        return Some(outcome);
    };
    let mass: f64 = probabilities.values().sum();
    if mass <= 0.0 {
        return Some(outcome);
    }
    let several: f64 = probabilities
        .iter()
        .filter(|(kind, _)| questions::SEVERAL_KINDS.contains(&kind.as_str()))
        .map(|(_, p)| p / mass)
        .sum();
    Some(if at_least(1.0 - several) {
        Outcome::Clear
    } else if at_least(several) && matches!(outcome, Outcome::Uncertain(_)) {
        Outcome::Consider(several)
    } else {
        outcome
    })
}

/// The kind of file that decided an undecided split Score toward a split:
/// the likelier of the kinds that serve several features.
pub(in crate::units) fn several_kind<'a>(
    split: Option<&Answer>,
    kind: Option<&'a Answer>,
) -> Option<&'a str> {
    if !matches!(split.map(benefit), Some(Outcome::Uncertain(_))) {
        return None;
    }
    let Some(Answer::Choice { probabilities, .. }) = kind else {
        return None;
    };
    probabilities
        .iter()
        .filter(|(kind, _)| questions::SEVERAL_KINDS.contains(&kind.as_str()))
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(kind, _)| kind.as_str())
}

/// Lines a copy may span and still be short: sharing it saves little.
const SHORT_COPY_LINES: usize = 4;

/// Lines a copy between test cases of different files may span and still
/// only mirror the other file's tests.
const MIRRORED_CASE_LINES: usize = 12;

/// Repetition the behavior requires is not a concern. Copies whose every site
/// is inside test cases are one level lower: spelling out each case is how
/// tests are written, so a table of cases or a fixture is a style choice.
/// Short copies are at most a consider: four lines, such as a pooled
/// builder borrowed and released around one call, repeat in two places as an
/// idiom as often as they hide a missing helper. On 17 projects, 5 of the 9
/// reviews of copies that short were idioms: constructor middleware
/// declarations, hook preambles, a Go validator's field copies. Short copies
/// inside test cases, a login step or an assertion tail, are notes. Copies
/// in test code outside its cases, in fixtures, helpers and setup, are at
/// most a consider: labeled by hand on 25 projects, 13 of 19 such reviews
/// were a level too strong, while 11 of 12 considers were right as they were.
/// Copies of up to twelve lines between test cases in different files are
/// notes too: tests of separate modules or rules repeat the same setup
/// because the code they test is parallel, and a helper shared across test
/// files would couple them. Of 87 such considers labeled by hand on the
/// projects used for tuning, 30 were right; on the held-out projects 2 of 24.
pub(in crate::units) fn shared_outcome(
    required: Option<&Answer>,
    same: Option<&Answer>,
    unit: &UnitPlan,
) -> Option<Outcome> {
    if matches!(noul(required?), Outcome::Review(_)) {
        return Some(Outcome::Clear);
    }
    let same = score(same?);
    let within = |lines: usize| {
        unit.locations
            .iter()
            .all(|l| l.end_line + 1 - l.start_line <= lines)
    };
    let short = within(SHORT_COPY_LINES);
    let mirrored = within(MIRRORED_CASE_LINES)
        && unit
            .locations
            .iter()
            .any(|l| l.path != unit.locations[0].path);
    // Examples spell a flow out on purpose, often once per variant:
    // django-styleguide shows each Google login step as a DRF API and as a
    // plain Django view.
    let examples = unit
        .locations
        .iter()
        .all(|l| crate::analysis::clones::example_code(&l.path));
    Some(match (&unit.detail, same) {
        (_, Outcome::Review(p) | Outcome::Consider(p)) if examples => Outcome::Note(p),
        (Detail::Pair { in_cases: true, .. }, _) if short || mirrored => lowered(lowered(same)),
        // Short copies in test support, such as a run of one-line assertions.
        (Detail::Pair { in_tests: true, .. }, _) if short => lowered(same),
        (Detail::Pair { in_cases: true, .. }, _) => lowered(same),
        (Detail::Pair { in_tests: true, .. }, Outcome::Review(p)) => Outcome::Consider(p),
        (_, Outcome::Review(p)) if short => Outcome::Consider(p),
        _ => same,
    })
}
