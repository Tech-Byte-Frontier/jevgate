//! Outcomes of the maintainability rules: functions, file organization, shared logic and hardcoded values.
use super::*;

/// The stronger of splitting and (for deeply nested functions only) flattening.
pub(in crate::units) fn function_outcome(
    split: Option<&Answer>,
    flatten: Option<&Answer>,
) -> Option<Outcome> {
    let mut outcomes = vec![benefit(split?)];
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

/// The split Score, or when it stays undecided, the kind of file: the kinds
/// that serve one feature ruling a split out clear, the kinds that serve
/// several reaching the threshold a consider.
pub(in crate::units) fn organization_outcome(
    split: Option<&Answer>,
    kind: Option<&Answer>,
) -> Option<Outcome> {
    let outcome = benefit(split?);
    let (Outcome::Uncertain(_), Some(Answer::Choice { probabilities, .. })) = (outcome, kind)
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
    } else if at_least(several) {
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

/// Repetition the behavior requires is not a concern. Copies whose every site
/// is inside test cases are one level lower: spelling out each case is how
/// tests are written, so a table of cases or a fixture is a style choice.
/// Short copies are at most a consider: four lines, such as a pooled
/// builder borrowed and released around one call, repeat in two places as an
/// idiom as often as they hide a missing helper. On 17 projects, 5 of the 9
/// reviews of copies that short were idioms: constructor middleware
/// declarations, hook preambles, a Go validator's field copies. Short copies
/// inside test cases, a login step or an assertion tail, are notes.
pub(in crate::units) fn shared_outcome(
    required: Option<&Answer>,
    same: Option<&Answer>,
    unit: &UnitPlan,
) -> Option<Outcome> {
    if matches!(noul(required?), Outcome::Review(_)) {
        return Some(Outcome::Clear);
    }
    let same = score(same?);
    let short = unit
        .locations
        .iter()
        .all(|l| l.end_line + 1 - l.start_line <= SHORT_COPY_LINES);
    Some(match (&unit.detail, same) {
        (Detail::Pair { in_cases: true, .. }, _) if short => lowered(lowered(same)),
        (Detail::Pair { in_cases: true, .. }, _) => lowered(same),
        (_, Outcome::Review(p)) if short => Outcome::Consider(p),
        _ => same,
    })
}
