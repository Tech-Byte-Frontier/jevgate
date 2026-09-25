//! Outcomes of injection units: the checks that found a variable placed
//! unhandled, and where its values come from.
use super::{
    security::{checks, choice_mass, settled_checks},
    *,
};

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

/// Kinds where a variable is a concern only when another party controls it:
/// helpers that build a path, URL or redirect target from their parameters
/// are everywhere.
const RESOURCE_CHECKS: [&str; 3] = ["path", "url", "redirect"];

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
    let found = found_injections(get);
    let origin = match (origin_outcome(origin), outside_markup(&found, get)) {
        (Outcome::Review(p), _) => Outcome::Review(p),
        (_, Some(p)) => Outcome::Review(p),
        (outcome, None) => outcome,
    };
    Some(by_origin(origin, &found, get))
}

/// When the markup check found a variable and the PHP markup Choice names
/// what is joined as a request value or a stored record, at the threshold:
/// that value comes from another party, whatever the origin question made
/// of a page's other values. On DVWA an access log joining user names from
/// the database stayed uncertain with the origin split 0.26/0.24/0.50.
fn outside_markup<'a>(found: &[&str], get: &impl Fn(&str) -> Option<&'a Answer>) -> Option<f64> {
    if !found.contains(&"markup") {
        return None;
    }
    choice_mass(get("markup_parts"), &questions::OUTSIDE_MARKUP).filter(|p| at_least(*p))
}

/// The injection checks that found a variable placed unhandled.
fn found_injections<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> Vec<&'static str> {
    settled_checks(catalog::INJECTION, get)
        .into_iter()
        .filter(|(_, o)| matches!(o, Outcome::Review(_)))
        .map(|(id, _)| id)
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
            let leaning = settled_checks(catalog::INJECTION, get)
                .into_iter()
                .filter(|(_, o)| *o != Outcome::Clear)
                .filter_map(|(id, _)| get(id))
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
