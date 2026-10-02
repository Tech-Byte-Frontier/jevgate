//! The answers a unit's outcome rests on: its first pass with the rechecks,
//! traces, settles and locates its rule reads beside or in place of them.
use super::*;

pub(super) fn answers<'a>(judgments: &'a [Judgment], unit: &str, pass: Pass) -> Answers<'a> {
    judgments
        .iter()
        .filter(|j| j.unit == unit && j.pass == pass)
        .map(|j| (j.question.as_str(), &j.answer))
        .collect()
}

pub(super) fn security(rule: &str) -> bool {
    catalog::SECURITY.contains(&rule)
}

/// A security unit's first-pass and trace answers, with each recheck answer
/// (the origin or a check, seen with callers) in place of the traced one,
/// unless the traced answer is decisive and the recheck is not: an undecided
/// traced answer is replaced even by an undecided recheck, whose lean saw
/// more evidence.
pub(super) fn security_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> Answers<'a> {
    let mut merged = answers(judgments, &unit.id, Pass::First);
    merged.extend(answers(judgments, &unit.id, Pass::Trace));
    let judged = |question: &str, answer: &Answer| match question {
        "origin" => origin_outcome(answer),
        _ => noul(answer),
    };
    replace_rechecked(
        &mut merged,
        answers(judgments, &unit.id, Pass::Recheck),
        judged,
    );
    // The settle answers sit beside the checks they settle, under their own
    // names, and so does what an injection consider's values can hold.
    merged.extend(answers(judgments, &unit.id, Pass::Settle));
    merged.extend(answers(judgments, &unit.id, Pass::Locate));
    merged
}

/// A unit's outcome and the answers it rests on: its first-pass answers with
/// the follow-ups its rule reads beside or in place of them.
pub(super) fn resolved<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> (Outcome, Answers<'a>) {
    let merged = if security(unit.rule) {
        security_answers(unit, judgments)
    } else if let Some(pass) = beside(unit.rule) {
        beside_answers(unit, judgments, pass)
    } else if unit.rule == catalog::COMMENTS {
        comment_answers(unit, judgments)
    } else if unit.rule == catalog::TEST_VALUE {
        let merged = test_value_answers(unit, judgments);
        let outcome = leaning_test(unit, judgments, unit_outcome(unit, &merged));
        return (outcome, merged);
    } else if unit.rule == catalog::TEST_REDUNDANCY {
        // Whether each test checks something the other does not, asked of a
        // pair that reached a review, sits beside its answers.
        let (_, mut merged) = rechecked(unit, judgments);
        merged.extend(answers(judgments, &unit.id, Pass::Locate));
        merged
    } else {
        return rechecked(unit, judgments);
    };
    (unit_outcome(unit, &merged), merged)
}

/// A test whose hollow checks stay undecided once its recheck is asked (or
/// when it has none) leans: below 0.50 it is clear. Labeled from the code,
/// 4 of 43 such tests below 0.50 checked only their mocks or recomputed
/// their expected value (5 counting a test whose one real check is weak),
/// against 10 of 35 at 0.50 or more; 636 of the 792 undecided tests on the
/// corpus lean below.
pub(super) fn leaning_test(unit: &UnitPlan, judgments: &[Judgment], outcome: Outcome) -> Outcome {
    let rechecked =
        unit.recheck.is_none() || !answers(judgments, &unit.id, Pass::Recheck).is_empty();
    match outcome {
        Outcome::Uncertain(p)
            if rechecked
                && !crate::policy::probability_at_least(p, crate::policy::LEADING_PROBABILITY) =>
        {
            Outcome::Clear
        }
        other => other,
    }
}

/// The pass of the follow-ups whose questions sit beside the first answers
/// under their own ids: document section and pair checks, the kind of a
/// large document, and the rechecks of instruction sections and workflows.
pub(super) fn beside(rule: &str) -> Option<Pass> {
    if [
        catalog::DOC_STALENESS,
        catalog::DOC_DUPLICATION,
        catalog::LARGE_DOCS,
    ]
    .contains(&rule)
    {
        Some(Pass::Trace)
    } else if [catalog::AGENT_CONTEXT, catalog::WORKFLOWS].contains(&rule) {
        Some(Pass::Recheck)
    } else {
        None
    }
}

pub(super) fn beside_answers<'a>(
    unit: &UnitPlan,
    judgments: &'a [Judgment],
    pass: Pass,
) -> Answers<'a> {
    let mut merged = answers(judgments, &unit.id, Pass::First);
    merged.extend(answers(judgments, &unit.id, pass));
    // How a pair's sections relate, or what a section treats its missing
    // names as, asked when its checks stay undecided.
    merged.extend(answers(judgments, &unit.id, Pass::Settle));
    merged
}

/// A comment's recheck replaces its first answers when the first stayed open
/// and the recheck decides, or neither decides; the kind of comment, asked
/// when it stays undecided, sits beside them.
pub(super) fn comment_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> Answers<'a> {
    let first = answers(judgments, &unit.id, Pass::First);
    let before = unit_outcome(unit, &first);
    let recheck = answers(judgments, &unit.id, Pass::Recheck);
    let mut merged = if !recheck.is_empty()
        && open(unit, &first, before)
        && (unit_outcome(unit, &recheck).decisive() || !before.decisive())
    {
        recheck
    } else {
        first
    };
    merged.extend(answers(judgments, &unit.id, Pass::Settle));
    merged
}

/// A test recheck asks the hollow-test questions again with the code under
/// test and the setup; each answer replaces the first one unless only the
/// first is decisive.
pub(super) fn test_value_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> Answers<'a> {
    let mut merged = answers(judgments, &unit.id, Pass::First);
    let recheck = answers(judgments, &unit.id, Pass::Recheck);
    replace_rechecked(&mut merged, recheck, |_, answer| noul(answer));
    // What its assertions read, asked after an internal-details consider.
    merged.extend(answers(judgments, &unit.id, Pass::Locate));
    merged
}

/// Puts each `recheck` answer in place of the one `merged` holds, unless
/// only the held one is decisive as `judged` reads them.
fn replace_rechecked<'a>(
    merged: &mut Answers<'a>,
    recheck: Answers<'a>,
    judged: impl Fn(&str, &Answer) -> Outcome,
) {
    for (question, answer) in recheck {
        let held = merged.get(question).map(|a| judged(question, a));
        if judged(question, answer).decisive() || !held.is_some_and(Outcome::decisive) {
            merged.insert(question, answer);
        }
    }
}

/// The first-pass outcome, or the recheck's when the first called for one
/// and the recheck decides.
pub(super) fn rechecked<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> (Outcome, Answers<'a>) {
    let first = answers(judgments, &unit.id, Pass::First);
    let outcome = unit_outcome(unit, &first);
    let recheck = answers(judgments, &unit.id, Pass::Recheck);
    if open(unit, &first, outcome) && !recheck.is_empty() {
        let second = unit_outcome(unit, &recheck);
        if second.decisive() {
            return (second, recheck);
        }
    }
    (outcome, first)
}
