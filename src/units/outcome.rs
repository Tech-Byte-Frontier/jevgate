//! From typed answers to one unit's outcome. Thresholds are the shared 0.80
//! policy on a Score whose top level is the actionable concern: review when
//! the top level reaches it, consider when the middle-or-top mass does, clear
//! when the top level is ruled out (its complement reaches it), otherwise
//! uncertain. Where the middle level says the code is fine as it is, a
//! consider needs the top level to lead; middle mass alone is an optional note.
use super::{Detail, GroupInfo, UnitPlan};
use crate::{
    catalog,
    policy::{LEADING_PROBABILITY, LOCATION_PROBABILITY, REVIEW_PROBABILITY, probability_at_least},
    schema::Answer,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Review(f64),
    Consider(f64),
    /// An optional improvement to code that reads well as it is.
    Note(f64),
    Clear,
    /// Carries the concern probability that stayed below the thresholds.
    Uncertain(f64),
    /// No answers were recorded, for example after a failed request.
    Missing,
}

impl Outcome {
    pub(super) fn decisive(self) -> bool {
        matches!(
            self,
            Self::Review(_) | Self::Consider(_) | Self::Note(_) | Self::Clear
        )
    }

    pub(super) fn concern(self) -> f64 {
        match self {
            Self::Review(p) | Self::Consider(p) | Self::Note(p) | Self::Uncertain(p) => p,
            Self::Clear | Self::Missing => 0.0,
        }
    }
}

pub(super) fn at_least(value: f64) -> bool {
    probability_at_least(value, REVIEW_PROBABILITY)
}

pub(super) fn levels(answer: &Answer) -> Option<[f64; 3]> {
    let Answer::Score { probabilities, .. } = answer else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    if mass <= 0.0 {
        return None;
    }
    let level = |i: usize| probabilities.get(&i.to_string()).copied().unwrap_or(0.0) / mass;
    Some([level(0), level(1), level(2)])
}

pub fn score(answer: &Answer) -> Outcome {
    let Some([bottom, middle, top]) = levels(answer) else {
        return Outcome::Missing;
    };
    if at_least(top) {
        Outcome::Review(top)
    } else if at_least(middle + top) {
        Outcome::Consider(middle + top)
    } else if at_least(bottom + middle) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(top)
    }
}

/// A Score whose middle level says the code reads well as it is ("Slightly …,
/// but it is fine as it is"): that mass alone raises a note, not a consider.
pub fn benefit(answer: &Answer) -> Outcome {
    match (score(answer), levels(answer)) {
        (Outcome::Consider(p), Some([_, _, top]))
            if !probability_at_least(top, LEADING_PROBABILITY) =>
        {
            Outcome::Note(p)
        }
        (outcome, _) => outcome,
    }
}

/// A benefit note split between its two upper levels: neither "fine as it
/// is" nor the concern leads. It gets the unit's recheck, like an undecided
/// answer, since the note may hide a consider.
pub(super) fn torn(answer: &Answer) -> bool {
    matches!(benefit(answer), Outcome::Note(_))
        && levels(answer).is_some_and(|[_, middle, top]| {
            !probability_at_least(middle, LEADING_PROBABILITY)
                && !probability_at_least(top, LEADING_PROBABILITY)
        })
}

/// Whether a unit's first outcome calls for its recheck: undecided, or a
/// note from a torn function or file-organization answer.
pub(super) fn open(unit: &UnitPlan, answers: &Answers<'_>, outcome: Outcome) -> bool {
    let benefit_questions: &[&str] = match unit.rule {
        catalog::FUNCTION_SIMPLIFICATION => &["split", "flatten"],
        catalog::FILE_ORGANIZATION => &["split"],
        _ => &[],
    };
    match outcome {
        Outcome::Uncertain(_) => true,
        Outcome::Note(_) => benefit_questions
            .iter()
            .any(|q| answers.get(q).is_some_and(|a| torn(a))),
        _ => false,
    }
}

/// One level lower: review becomes consider, consider becomes note.
pub(super) fn lowered(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        Outcome::Consider(p) => Outcome::Note(p),
        other => other,
    }
}

pub fn noul(answer: &Answer) -> Outcome {
    let Answer::Noul { noul } = answer else {
        return Outcome::Missing;
    };
    if at_least(*noul) {
        Outcome::Review(*noul)
    } else if at_least(1.0 - noul) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(*noul)
    }
}

pub(super) fn choice(answer: Option<&Answer>) -> Option<(&str, f64)> {
    let Answer::Choice {
        choice,
        probabilities,
        ..
    } = answer?
    else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    let p = probabilities.get(choice).copied().unwrap_or(0.0) / mass.max(f64::MIN_POSITIVE);
    (choice != "none" && probability_at_least(p, LOCATION_PROBABILITY)).then_some((choice, p))
}

pub(super) type Answers<'a> = BTreeMap<&'a str, &'a Answer>;

pub(super) fn unit_outcome(unit: &UnitPlan, answers: &Answers<'_>) -> Outcome {
    let get = |q: &str| answers.get(q).copied();
    let result = match unit.rule {
        catalog::FUNCTION_SIMPLIFICATION => function_outcome(get("split"), get("flatten")),
        catalog::FILE_ORGANIZATION => {
            organization_outcome(get("split"), get("module"), &unit.detail)
        }
        catalog::SHARED_LOGIC => shared_outcome(get("required"), get("same"), &unit.detail),
        catalog::TEST_VALUE => test_value_outcome(&get),
        catalog::TEST_REDUNDANCY => get("overlap").map(score),
        catalog::HARDCODED_VALUES => values_outcome(&get, &unit.detail),
        catalog::INJECTION => injection_outcome(&get),
        catalog::SENSITIVE_DATA => {
            exposure_outcome(unit.rule, &get, &["logs_secret", "error_details"])
        }
        catalog::UNSAFE_SETTINGS => exposure_outcome(unit.rule, &get, &["weakened"]),
        catalog::LARGE_DOCS => document_outcome(&get),
        catalog::DOC_STALENESS => {
            let question = if matches!(unit.detail, Detail::Plan { .. }) {
                "plan"
            } else {
                "relies"
            };
            get(question).map(|a| cleanup(noul(a)))
        }
        catalog::DOC_DUPLICATION => ["a_covers", "b_covers", "conflict"]
            .iter()
            .map(|q| get(q).map(|a| cleanup(noul(a))))
            .collect::<Option<Vec<_>>>()
            .map(|signals| strongest(&signals)),
        catalog::AGENT_CONTEXT => {
            section_signals(&get).map(|s| strongest(&s.iter().map(|(_, o)| *o).collect::<Vec<_>>()))
        }
        _ => None,
    };
    result.unwrap_or(Outcome::Missing)
}

/// Documentation findings are cleanups, never defects: at most a consider.
pub(super) fn cleanup(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    }
}

/// A large document: a split Score where the middle level says it is fine
/// as it is, and a Noul on whether it mainly records past work. Both are at
/// most a consider; an undecided history answer that leans toward past work
/// is a note, since no labeled living document leaned past 0.50.
pub(super) fn document_outcome<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> Option<Outcome> {
    let capped = |outcome: Outcome| match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    };
    let split = capped(benefit(get("split")?));
    let history = get("history")?;
    let past = match capped(noul(history)) {
        Outcome::Uncertain(p) if probability_at_least(p, LEADING_PROBABILITY) => Outcome::Note(p),
        other => other,
    };
    Some(strongest(&[split, past]))
}

/// Each answered question about an instruction section with its outcome.
/// Removing documentation is a cleanup, never a defect, so a signal is at
/// most a consider. A section that loads in every session but applies to one
/// directory is a note, and only on a clear choice of that directory.
pub(super) fn section_signals<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Vec<(&'static str, Outcome)>> {
    let capped = |outcome: Outcome| match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    };
    let mut signals = vec![("inferable", capped(benefit(get("inferable")?)))];
    for question in ["describes", "commands", "generic", "history", "enforced"] {
        if let Some(answer) = get(question) {
            signals.push((question, capped(noul(answer))));
        }
    }
    if let Some(answer) = get("scope") {
        let chosen = choice(Some(answer)).filter(|(_, p)| at_least(*p));
        signals.push((
            "scope",
            chosen.map_or(Outcome::Clear, |(_, p)| Outcome::Note(p)),
        ));
    }
    Some(signals)
}

/// The strongest of several signals about one unit: review, then consider,
/// then note, each at its highest probability. Clear only when every signal
/// is clear; otherwise uncertain.
pub(super) fn strongest(outcomes: &[Outcome]) -> Outcome {
    let best = |pick: fn(Outcome) -> Option<f64>| {
        outcomes.iter().filter_map(|o| pick(*o)).reduce(f64::max)
    };
    if let Some(p) = best(|o| matches!(o, Outcome::Review(_)).then(|| o.concern())) {
        Outcome::Review(p)
    } else if let Some(p) = best(|o| matches!(o, Outcome::Consider(_)).then(|| o.concern())) {
        Outcome::Consider(p)
    } else if let Some(p) = best(|o| matches!(o, Outcome::Note(_)).then(|| o.concern())) {
        Outcome::Note(p)
    } else if outcomes.iter().all(|o| *o == Outcome::Clear) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(outcomes.iter().map(|o| o.concern()).fold(0.0, f64::max))
    }
}

/// The stronger of splitting and (for deeply nested functions only) flattening.
pub(super) fn function_outcome(
    split: Option<&Answer>,
    flatten: Option<&Answer>,
) -> Option<Outcome> {
    let mut outcomes = vec![benefit(split?)];
    outcomes.extend(flatten.map(benefit));
    Some(strongest(&outcomes))
}

/// The judgment id of the benign-kind check that follows an undecided
/// hardcoded-value question.
pub(super) fn benign_key(question: &str) -> &'static str {
    match question {
        "environment" => "benign_environment",
        "magic" => "benign_magic",
        _ => "benign_special",
    }
}

/// How far an undecided answer leans toward its concern: a Score's
/// middle-or-top mass, a Noul's probability.
pub(super) fn lean(answer: &Answer) -> f64 {
    match answer {
        Answer::Noul { noul } => *noul,
        _ => levels(answer).map_or(0.0, |[_, middle, top]| middle + top),
    }
}

/// An undecided answer after its follow-up: clear when the check finds only
/// acceptable kinds; otherwise a note when it leans toward the concern (a
/// medium-confidence flag that never fails the gate), else still uncertain.
/// Without a follow-up answer (`settle` false) it stays as it was, so the
/// follow-up is asked first.
pub(super) fn settled(
    outcome: Outcome,
    answer: &Answer,
    check: Option<&Answer>,
    settle: bool,
) -> Outcome {
    let Outcome::Uncertain(p) = outcome else {
        return outcome;
    };
    if !settle {
        return outcome;
    }
    if matches!(check.map(noul), Some(Outcome::Review(_))) {
        Outcome::Clear
    } else if probability_at_least(lean(answer), LEADING_PROBABILITY) {
        Outcome::Note(lean(answer))
    } else {
        Outcome::Uncertain(p)
    }
}

/// Each hardcoded-value question of a unit with its outcome and whether an
/// undecided answer was turned into a note by its lean. A file's constants
/// are asked only about the environment.
pub(super) fn value_signals<'a>(
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

pub(super) fn values_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
    detail: &Detail,
) -> Option<Outcome> {
    let signals = value_signals(get, detail, true)?;
    let outcomes: Vec<Outcome> = signals.iter().map(|(_, o, _)| *o).collect();
    Some(strongest(&outcomes))
}

/// Where an injection's values come from: another party (top) is a review;
/// parameters of unknown origin (middle) are a concern a caller settles, so
/// middle-or-top mass is a consider; the program itself (bottom) is clear.
pub(super) fn origin_outcome(answer: &Answer) -> Outcome {
    let Some([bottom, middle, top]) = levels(answer) else {
        return Outcome::Missing;
    };
    if at_least(top) {
        Outcome::Review(top)
    } else if at_least(middle + top) {
        Outcome::Consider(middle + top)
    } else if at_least(bottom) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(top)
    }
}

/// The outcomes of a rule's specific trace checks that were answered.
pub(super) fn checks<'a>(rule: &str, get: &impl Fn(&str) -> Option<&'a Answer>) -> Vec<Outcome> {
    super::security::checks(rule)
        .iter()
        .filter_map(|check| get(check.id).map(noul))
        .collect()
}

/// Kinds where a variable is a concern only when another party controls it:
/// helpers that build a path or URL from their parameters are everywhere.
pub(super) const RESOURCE_CHECKS: [&str; 2] = ["path", "url"];

/// Presence alone never raises an injection: it only decides whether the
/// trace is asked. When every specific check clears the unit, it is clear;
/// otherwise the origin of its values decides. Only a check that finds a
/// variable placed unhandled raises a consider or review, so a finding names
/// its kind; without one, values from another party are a note, and
/// parameters are a note when a check leans toward a concern and undecided
/// otherwise. Parameters spliced into SQL, a
/// shell command, code or markup are a consider (bind, quote or escape them);
/// parameters in a path or URL are a note until a caller shows another party
/// controls them.
pub(super) fn injection_outcome<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> Option<Outcome> {
    let presence = [noul(get("interpreted")?), noul(get("resource")?)];
    if presence.iter().all(|o| *o == Outcome::Clear) {
        return Some(Outcome::Clear);
    }
    let unhandled = checks(catalog::INJECTION, get);
    let (Some(origin), false) = (get("origin"), unhandled.is_empty()) else {
        return Some(Outcome::Uncertain(
            presence.iter().map(|o| o.concern()).fold(0.0, f64::max),
        ));
    };
    if unhandled.iter().all(|o| *o == Outcome::Clear) {
        return Some(Outcome::Clear);
    }
    Some(by_origin(
        origin_outcome(origin),
        &found_injections(get),
        get,
    ))
}

/// The injection checks that found a variable placed unhandled.
fn found_injections<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> Vec<&'static str> {
    super::security::checks(catalog::INJECTION)
        .iter()
        .filter(|check| {
            get(check.id)
                .map(noul)
                .is_some_and(|o| matches!(o, Outcome::Review(_)))
        })
        .map(|check| check.id)
        .collect()
}

/// The origin's outcome given the checks that found something: with none,
/// another party's values are a note and parameters a note only when a
/// check leans toward a concern; parameters only in paths or URLs are lower.
fn by_origin<'a>(
    outcome: Outcome,
    found: &[&str],
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Outcome {
    let resource_only = found.iter().all(|id| RESOURCE_CHECKS.contains(id));
    match outcome {
        Outcome::Review(p) if found.is_empty() => Outcome::Note(p),
        Outcome::Consider(p) if found.is_empty() => {
            let leaning = super::security::checks(catalog::INJECTION)
                .iter()
                .filter_map(|check| get(check.id))
                .map(lean)
                .fold(0.0, f64::max);
            if probability_at_least(leaning, LEADING_PROBABILITY) {
                Outcome::Note(leaning)
            } else {
                Outcome::Uncertain(p)
            }
        }
        Outcome::Consider(_) if resource_only => lowered(outcome),
        _ => outcome,
    }
}

/// Error-detail signals that the "own messages" check clears.
pub(super) const ERROR_SIGNALS: [&str; 2] = ["error_details", "exception_to_client"];

/// Logged secrets, exposed error details and weak settings: a presence
/// question or a specific check at review raises it, one level lower for
/// code that runs only in development; clear when presence or every check
/// rules it out. Error-detail signals are clear when every error message is
/// the program's own. When the own-messages check instead finds an
/// exception's, library's or database's text in an error message, an
/// error-detail signal that leans toward a client is a consider: the check
/// states the detail, and the lean where it goes. Otherwise a signal that
/// leans toward the concern is a note, and the rest stay undecided.
pub(super) fn exposure_outcome<'a>(
    rule: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
    questions: &[&str],
) -> Option<Outcome> {
    let own = (rule == catalog::SENSITIVE_DATA)
        .then(|| get("own_messages").map(noul))
        .flatten();
    let judge = |question: &str, answer: &Answer| exposure_signal(question, answer, own);
    let presence: Vec<(Outcome, f64)> = questions
        .iter()
        .map(|q| get(q).map(|a| judge(q, a)))
        .collect::<Option<_>>()?;
    let specific: Vec<(Outcome, f64)> = super::security::checks(rule)
        .iter()
        .filter_map(|check| get(check.id).map(|a| judge(check.id, a)))
        .collect();
    let ruled_out = presence.iter().all(|(o, _)| *o == Outcome::Clear)
        || (!specific.is_empty() && specific.iter().all(|(o, _)| *o == Outcome::Clear));
    let found: Vec<Outcome> = presence
        .iter()
        .chain(&specific)
        .map(|(o, _)| *o)
        .filter(|o| matches!(o, Outcome::Review(_) | Outcome::Consider(_)))
        .collect();
    let leaning = presence
        .iter()
        .chain(&specific)
        .filter(|(o, _)| matches!(o, Outcome::Uncertain(_)))
        .map(|(_, lean)| *lean)
        .fold(0.0, f64::max);
    let outcome = if !found.is_empty() {
        strongest(&found)
    } else if ruled_out {
        Outcome::Clear
    } else if probability_at_least(leaning, LEADING_PROBABILITY) {
        Outcome::Note(leaning)
    } else {
        Outcome::Uncertain(leaning)
    };
    Some(
        if matches!(get("dev_only").map(noul), Some(Outcome::Review(_))) {
            lowered(outcome)
        } else {
            outcome
        },
    )
}

/// One exposure answer with its lean. The own-messages check (`own`) settles
/// error-detail signals: all messages the program's own clears them; a
/// message carrying another's error text makes a lean toward a client a
/// consider.
fn exposure_signal(question: &str, answer: &Answer, own: Option<Outcome>) -> (Outcome, f64) {
    let outcome = noul(answer);
    let lean = lean(answer);
    if !ERROR_SIGNALS.contains(&question) {
        return (outcome, lean);
    }
    match (own, outcome) {
        (Some(Outcome::Review(_)), _) => (Outcome::Clear, 0.0),
        (Some(Outcome::Clear), Outcome::Uncertain(_))
            if probability_at_least(lean, LEADING_PROBABILITY) =>
        {
            (Outcome::Consider(lean), lean)
        }
        _ => (outcome, lean),
    }
}

/// A split is suggested only when the proposed group has users of its own in
/// other files: a group nothing else imports gains little from its own module.
/// When no member has known users (an entry point, or callers outside the
/// selected files), the evidence is missing and the answer stands.
pub(super) fn organization_outcome(
    split: Option<&Answer>,
    module: Option<&Answer>,
    detail: &Detail,
) -> Option<Outcome> {
    let outcome = benefit(split?);
    let Detail::Outline { groups } = detail else {
        return Some(outcome);
    };
    Some(match outcome {
        Outcome::Review(p) | Outcome::Consider(p) if !split_has_users(groups, module) => {
            Outcome::Note(p)
        }
        other => other,
    })
}

/// Whether the chosen group (or, without a choice, any group) has a user that
/// no other group of the file has; true when no users are known at all.
pub(super) fn split_has_users(groups: &[GroupInfo], module: Option<&Answer>) -> bool {
    if groups.iter().all(|g| g.users.is_empty()) {
        return true;
    }
    let own_users = |group: &GroupInfo| {
        group.users.iter().any(|user| {
            groups
                .iter()
                .filter(|other| other.id != group.id)
                .all(|other| !other.users.contains(user))
        })
    };
    match choice(module).and_then(|(id, _)| groups.iter().find(|g| g.id == id)) {
        Some(chosen) => own_users(chosen),
        None => groups.iter().any(own_users),
    }
}

/// Repetition the behavior requires is not a concern. Copies whose every site
/// is inside test cases are one level lower: spelling out each case is how
/// tests are written, so a table of cases or a fixture is a style choice.
pub(super) fn shared_outcome(
    required: Option<&Answer>,
    same: Option<&Answer>,
    detail: &Detail,
) -> Option<Outcome> {
    if matches!(noul(required?), Outcome::Review(_)) {
        return Some(Outcome::Clear);
    }
    let same = score(same?);
    Some(match detail {
        Detail::Pair { in_cases: true, .. } => lowered(same),
        _ => same,
    })
}

/// Hollow signals decide review and clear; weak signals can raise a consider,
/// and their uncertainty does not block a clear.
pub(super) fn test_value_outcome<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> Option<Outcome> {
    let hollow = [noul(get("own_logic")?), noul(get("mock_only")?)];
    let weak = [noul(get("internal")?), noul(get("several")?)];
    let strongest = |outcomes: &[Outcome]| {
        outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Review(p) => Some(*p),
                _ => None,
            })
            .reduce(f64::max)
    };
    Some(if let Some(p) = strongest(&hollow) {
        Outcome::Review(p)
    } else if let Some(p) = strongest(&weak) {
        Outcome::Consider(p)
    } else if hollow.iter().all(|o| *o == Outcome::Clear) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(hollow.iter().map(|o| o.concern()).fold(0.0, f64::max))
    })
}
