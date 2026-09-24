//! Outcomes of the security rules: injection checks and origins, exposed
//! data and messages, and weak settings.
use super::*;

/// Where an injection's values come from: another party (top) is a review;
/// parameters of unknown origin (middle) are a concern a caller settles, so
/// middle-or-top mass is a consider; the program itself (bottom) is clear.
pub(in crate::units) fn origin_outcome(answer: &Answer) -> Outcome {
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

/// The outcomes of a rule's specific trace checks that were answered. An
/// undecided URL check is clear when its settle Choice puts the host among
/// the program's own at the threshold.
pub(in crate::units) fn checks<'a>(
    rule: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Vec<Outcome> {
    crate::units::security::checks(rule)
        .iter()
        .filter_map(|check| {
            let outcome = noul(get(check.id)?);
            let own_host = check.id == "url"
                && choice_mass(get("url_parts"), &questions::OWN_PARTS).is_some_and(at_least);
            Some(match outcome {
                Outcome::Uncertain(_) if own_host => Outcome::Clear,
                other => other,
            })
        })
        .collect()
}

/// The share of a Choice's probability on `options`, when it was answered.
fn choice_mass(answer: Option<&Answer>, options: &[&str]) -> Option<f64> {
    let Answer::Choice { probabilities, .. } = answer? else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    (mass > 0.0).then(|| {
        probabilities
            .iter()
            .filter(|(option, _)| options.contains(&option.as_str()))
            .map(|(_, p)| p / mass)
            .sum()
    })
}

/// Whether the settle Choice sends a function's text anywhere but a remote
/// client, at the threshold.
fn away_from_clients<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> bool {
    choice_mass(get("destination"), &[questions::CLIENT])
        .is_some_and(|client| at_least(1.0 - client))
}

/// Kinds where a variable is a concern only when another party controls it:
/// helpers that build a path or URL from their parameters are everywhere.
const RESOURCE_CHECKS: [&str; 2] = ["path", "url"];

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
pub(in crate::units) fn injection_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Outcome> {
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
    crate::units::security::checks(catalog::INJECTION)
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
            let leaning = crate::units::security::checks(catalog::INJECTION)
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
const ERROR_SIGNALS: [&str; 2] = ["error_details", "exception_to_client"];

/// Whether a function's error messages are the program's own text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::units) enum Messages {
    /// Every message is the program's own text.
    Own,
    /// A message carries another error's text, at this probability.
    Foreign(f64),
    Undecided,
}

/// From the Choice over the errors a function creates, when it creates
/// any: `none` at the threshold is its own text, a message chosen at the
/// threshold carries another error's; else from the own-messages Noul.
pub(in crate::units) fn messages<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Messages> {
    let Some(answer) = get("messages") else {
        return get("own_messages").map(|a| match noul(a) {
            Outcome::Review(_) => Messages::Own,
            Outcome::Clear => Messages::Foreign(1.0 - lean(a)),
            _ => Messages::Undecided,
        });
    };
    let Answer::Choice {
        choice,
        probabilities,
        ..
    } = answer
    else {
        return None;
    };
    let mass = probabilities.values().sum::<f64>().max(f64::MIN_POSITIVE);
    let p = probabilities.get(choice).copied().unwrap_or(0.0) / mass;
    Some(match (choice.as_str(), at_least(p)) {
        ("none", true) => Messages::Own,
        (_, true) => Messages::Foreign(p),
        _ => Messages::Undecided,
    })
}

/// Logged secrets, exposed error details and weak settings: a presence
/// question or a specific check at review raises it, one level lower for
/// code that runs only in development; clear when presence or every check
/// rules it out. Error-detail signals are clear when every error message is
/// the program's own. When the own-messages check instead finds an
/// exception's, library's or database's text in an error message, an
/// error-detail signal that leans toward a client is a consider: the check
/// states the detail, and the lean where it goes. Otherwise a signal that
/// leans toward the concern is a note, and the rest stay undecided.
pub(in crate::units) fn exposure_outcome<'a>(
    rule: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
    questions: &[&str],
) -> Option<Outcome> {
    let own = (rule == catalog::SENSITIVE_DATA)
        .then(|| messages(get))
        .flatten();
    let away = rule == catalog::SENSITIVE_DATA && away_from_clients(get);
    let judge = |question: &str, answer: &Answer| exposure_signal(question, answer, own, away);
    let presence: Vec<(Outcome, f64)> = questions
        .iter()
        .map(|q| get(q).map(|a| judge(q, a)))
        .collect::<Option<_>>()?;
    let specific: Vec<(Outcome, f64)> = crate::units::security::checks(rule)
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
/// consider, at the probability that the message carries it. A signal still
/// undecided is clear when the settle Choice sends the text `away` from
/// remote clients.
fn exposure_signal(
    question: &str,
    answer: &Answer,
    own: Option<Messages>,
    away: bool,
) -> (Outcome, f64) {
    let outcome = noul(answer);
    let lean = lean(answer);
    if !ERROR_SIGNALS.contains(&question) {
        return (outcome, lean);
    }
    match (own, outcome) {
        (Some(Messages::Own), _) => (Outcome::Clear, 0.0),
        (Some(Messages::Foreign(p)), Outcome::Uncertain(_))
            if probability_at_least(lean, LEADING_PROBABILITY) =>
        {
            (Outcome::Consider(p), lean)
        }
        (_, Outcome::Uncertain(_)) if away => (Outcome::Clear, 0.0),
        _ => (outcome, lean),
    }
}
