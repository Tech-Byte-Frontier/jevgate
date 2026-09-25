//! From typed answers to one unit's outcome. Thresholds are the shared 0.80
//! policy on a Score whose top level is the actionable concern: review when
//! the top level reaches it, consider when the middle-or-top mass does, clear
//! when the top level is ruled out (its complement reaches it), otherwise
//! uncertain. Where the middle level says the code is fine as it is, a
//! consider needs the top level to lead; middle mass alone is an optional note.
use super::{Access, Detail, UnitPlan, questions};
use crate::{
    catalog,
    policy::{LEADING_PROBABILITY, LOCATION_PROBABILITY, REVIEW_PROBABILITY, probability_at_least},
    schema::Answer,
};
use std::collections::BTreeMap;

mod access;
mod documentation;
mod maintainability;
mod security;
mod test_rules;

use access::access_outcome;
use documentation::doc_pair_outcome;
pub(super) use documentation::{document_outcome, section_signals};
pub(super) use maintainability::{
    benign_key, function_outcome, organization_outcome, several_kind, shared_outcome,
    value_signals, values_outcome,
};
pub(super) use security::{
    Messages, checks, django_settings_outcome, exposure_outcome, injection_outcome, messages,
    origin_outcome, settled_checks,
};
pub(super) use test_rules::{redundancy_outcome, test_value_outcome};

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
        // Who calls a group is evidence in the outline, not a gate: a module
        // with one caller still helps a reader find a feature of a large file.
        // A test file's layout is advice, one level lower.
        catalog::FILE_ORGANIZATION => {
            organization_outcome(get("split"), get("kind")).map(|outcome| {
                if matches!(unit.detail, Detail::Outline { tests: true, .. }) {
                    lowered(outcome)
                } else {
                    outcome
                }
            })
        }
        catalog::SHARED_LOGIC => shared_outcome(get("required"), get("same"), &unit.detail),
        catalog::TEST_VALUE => test_value_outcome(&get),
        catalog::TEST_REDUNDANCY => {
            get("overlap").map(|overlap| redundancy_outcome(overlap, get("distinct")))
        }
        catalog::HARDCODED_VALUES => values_outcome(&get, &unit.detail),
        catalog::INJECTION => injection_outcome(&get),
        catalog::SENSITIVE_DATA if matches!(unit.detail, Detail::Handler { .. }) => {
            get("handler_leaks").map(noul)
        }
        catalog::SENSITIVE_DATA => {
            exposure_outcome(unit.rule, &get, &["logs_secret", "error_details"])
        }
        catalog::UNSAFE_SETTINGS if unit.name == crate::units::security::SETTINGS_MODULE => {
            django_settings_outcome(&get).map(|outcome| {
                if matches!(get("dev_only").map(noul), Some(Outcome::Review(_))) {
                    lowered(outcome)
                } else {
                    outcome
                }
            })
        }
        catalog::UNSAFE_SETTINGS
            if matches!(unit.detail, Detail::Security { django: true, .. }) =>
        {
            django_settings_outcome(&get)
        }
        catalog::UNSAFE_SETTINGS => exposure_outcome(unit.rule, &get, &["weakened"]),
        catalog::ACCESS_CONTROL => access_outcome(&get, &unit.detail),
        catalog::WORKFLOWS => {
            let asked: Vec<Outcome> = ["outside", "untrusted"]
                .iter()
                .filter_map(|q| get(q).map(noul))
                .collect();
            (!asked.is_empty()).then(|| strongest(&asked))
        }
        catalog::LARGE_DOCS => document_outcome(&get),
        catalog::DOC_STALENESS => {
            let question = if matches!(unit.detail, Detail::Plan { .. }) {
                "plan"
            } else {
                "relies"
            };
            get(question).map(|a| cleanup(noul(a)))
        }
        catalog::DOC_DUPLICATION => doc_pair_outcome(&get),
        catalog::AGENT_CONTEXT => {
            section_signals(&get).map(|s| strongest(&s.iter().map(|(_, o)| *o).collect::<Vec<_>>()))
        }
        _ => None,
    };
    result.unwrap_or(Outcome::Missing)
}

/// A Score whose two lower levels are acceptable: review at its top level,
/// clear when the two lower levels reach the threshold, otherwise uncertain.
pub(super) fn acceptable_levels(answer: &Answer) -> Outcome {
    let Some([bottom, middle, top]) = levels(answer) else {
        return Outcome::Missing;
    };
    if at_least(top) {
        Outcome::Review(top)
    } else if at_least(bottom + middle) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(top)
    }
}

/// Documentation findings are cleanups, never defects: at most a consider.
pub(super) fn cleanup(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    }
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
