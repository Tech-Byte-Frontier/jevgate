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
pub(in crate::units) const RESOURCE_CHECKS: [&str; 3] = ["path", "url", "redirect"];

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
    Some(match by_origin(origin, &found, get) {
        Outcome::Consider(p) if program_values(get) => Outcome::Note(p),
        Outcome::Review(p) | Outcome::Consider(p) if harmless(get).is_some() => Outcome::Note(p),
        outcome => outcome,
    })
}

/// The kinds of injection whose values a confirm Choice asks about after
/// the finding, with its question and the options that make it a note.
const CONFIRMED: [(&str, &str, &[&str]); 6] = [
    ("path", "paths", &questions::CONFINED_PATHS),
    ("markup", "markup_values", &questions::HARMLESS_MARKUP),
    ("redirect", "redirect_reach", &questions::OWN_SITE),
    ("sql", "query_values", &questions::HARMLESS_QUERY),
    ("shell", "query_values", &questions::HARMLESS_QUERY),
    ("code", "query_values", &questions::HARMLESS_QUERY),
];

/// The kinds whose confirm Choice is `query_values`, sent in a request of
/// its own; the others share the `checked` request.
pub(in crate::units) const QUERIED: [&str; 3] = ["sql", "shell", "code"];

/// The one kind of injection a finding rests on, when every check that
/// found a variable placed unhandled is that kind and a confirm Choice asks
/// about it.
pub(in crate::units) fn confirmable<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<&'static str> {
    let found = found_injections(get);
    let first = *found.first()?;
    CONFIRMED
        .iter()
        .find(|(kind, ..)| *kind == first && found.iter().all(|id| id == kind))
        .map(|(kind, ..)| *kind)
}

/// The kind of a finding whose confirm Choice leans toward values that can
/// do no harm there: paths that stay inside their directory (a route
/// parameter parsed as a UUID or as Rocket's `PathBuf`, a base name, a
/// checked id), markup values already escaped or encoded, or redirect
/// targets that stay on the site. Such a finding is a note. On the corpus,
/// the 5 path findings labeled right answered another party's input at 0.96
/// or more, while vaultwarden's 4 wrong ones on typed Rocket route
/// parameters leaned to confined names at 0.65 to 0.84, as a type's parsing
/// is shown only by its derive list. The 26 markup findings labeled right
/// put at most 0.22 on values that cannot open a tag, and vaultwarden's
/// percent-encoded username 0.56; the 6 redirect findings labeled right put
/// at most 0.44 on staying on the site, and vaultwarden's admin path and
/// shiori's login page 0.68 and 0.58. Asked too of markup considers on the
/// function's parameters, it made notes of 11 labeled wrong (escaped
/// before, typed, or the program's own markup) and 1 labeled right, a JSP
/// header writing a session value, at 0.69; the other 44 labeled right put
/// at most 0.47 there. SQL, command and code findings are asked what their
/// text can hold: fixed clauses a key selects, parsed ids, or a query the
/// sender may run anyway.
pub(in crate::units) fn harmless<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<&'static str> {
    let kind = confirmable(get)?;
    let (_, question, clears) = CONFIRMED.iter().find(|(k, ..)| *k == kind)?;
    choice_mass(get(question), clears)
        .is_some_and(|p| probability_at_least(p, LEADING_PROBABILITY))
        .then_some(kind)
}

/// Whether what a consider's values can hold, asked after it, leans toward
/// the program's own: literals its callers pass, values it creates, or the
/// arguments of a local tool. Such a consider, resting on the function's
/// parameters, is a note.
fn program_values<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> bool {
    choice_mass(get("values"), &questions::PROGRAM_VALUES)
        .is_some_and(|p| probability_at_least(p, LEADING_PROBABILITY))
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

/// Whether the Choice that settles an undecided resource check was asked
/// (where its paths, URLs or redirect targets come from) without leaning
/// toward another party's input, which keeps the check open.
fn settle_asked<'a>(check: &str, get: &impl Fn(&str) -> Option<&'a Answer>) -> bool {
    let question = match check {
        "path" if get("path_parts").is_some() => "path_parts",
        "path" => "path_source",
        "url" => "url_parts",
        "redirect" => "redirect_target",
        _ => return false,
    };
    choice_mass(get(question), &OUTSIDE_SOURCES)
        .is_some_and(|p| !probability_at_least(p, LEADING_PROBABILITY))
}

/// Options of the settle Choices that name another party's input.
const OUTSIDE_SOURCES: [&str; 3] = ["outside", "request", "stored"];

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
            let open: Vec<(&str, Outcome)> = settled_checks(catalog::INJECTION, get)
                .into_iter()
                .filter(|(_, o)| *o != Outcome::Clear)
                .collect();
            let leaning = open
                .iter()
                .filter_map(|(id, _)| get(id))
                .map(lean)
                .fold(0.0, f64::max);
            // Parameters in a path, URL or redirect are a note until a caller
            // shows another party controls them, found or not: undecided,
            // 320 such units on the corpus stayed uncertain while a found one
            // was a note. Only once the Choice that settles each check was
            // asked and did not clear it.
            let resources_open = !open.is_empty()
                && open
                    .iter()
                    .all(|(id, _)| RESOURCE_CHECKS.contains(id) && settle_asked(id, get));
            if probability_at_least(leaning, LEADING_PROBABILITY) || resources_open {
                Outcome::Note(leaning.max(p))
            } else {
                Outcome::Uncertain(p)
            }
        }
        Outcome::Consider(_) if resource_only => lowered(outcome),
        _ => outcome,
    }
}
