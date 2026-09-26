//! Outcomes of exposed data and weak settings: logged secrets, error details
//! sent to clients, the messages a function creates, and weak settings.
use super::{security::settled, *};

/// Whether the settle Choice sends a function's text anywhere but a remote
/// client, at the threshold.
fn away_from_clients<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> bool {
    settled(catalog::SENSITIVE_DATA, "error_details", get, false)
}

/// Error-detail signals that the "own messages" check clears.
const ERROR_SIGNALS: [&str; 2] = ["error_details", "exception_to_client"];

/// Whether a function's error messages are the program's own text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::units) enum Messages {
    /// Every message is the program's own text.
    Own,
    /// A message carries another error's text.
    Foreign,
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
            Outcome::Clear => Messages::Foreign,
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
        (_, true) => Messages::Foreign,
        _ => Messages::Undecided,
    })
}

/// A judged exposure answer and how far it leans toward its concern.
type Signal = (Outcome, f64);

/// Logged secrets, exposed error details and weak settings: a presence
/// question or a specific check at review raises it, one level lower for
/// code that runs only in development; clear when presence or every check
/// rules it out. Error-detail signals are clear when every error message is
/// the program's own. A signal that leans toward the concern is a note,
/// also when the own-messages check finds an exception's, library's or
/// database's text in an error message, and the rest stay undecided.
pub(in crate::units) fn exposure_outcome<'a>(
    rule: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
    questions: &[&str],
) -> Option<Outcome> {
    let (presence, specific) = exposure_signals(rule, get, questions)?;
    let outcome = exposure_level(rule, &presence, &specific);
    Some(
        if matches!(get("dev_only").map(noul), Some(Outcome::Review(_))) {
            lowered(outcome)
        } else {
            outcome
        },
    )
}

/// The presence answers of `questions` and the rule's specific checks, each
/// judged with its lean; an undecided check its settle Choice clears is
/// clear, and so is one that found a concern a Choice asked whenever the
/// check is not clear rules out. None until every presence question is
/// answered.
fn exposure_signals<'a>(
    rule: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
    questions: &[&str],
) -> Option<(Vec<Signal>, Vec<Signal>)> {
    let own = (rule == catalog::SENSITIVE_DATA)
        .then(|| messages(get))
        .flatten();
    let away = rule == catalog::SENSITIVE_DATA && away_from_clients(get);
    let judge = |question: &str, answer: &Answer| exposure_signal(question, answer, own, away);
    // A Choice asked whenever a presence signal is not clear rules it out
    // too: what a function's logs write clears an audit line that names who
    // signed in, which the presence question found as personal data.
    let presence: Vec<Signal> = questions
        .iter()
        .map(|q| {
            get(q).map(|a| match judge(q, a) {
                (Outcome::Clear, lean) => (Outcome::Clear, lean),
                _ if settled(rule, q, get, true) => (Outcome::Clear, 0.0),
                signal => signal,
            })
        })
        .collect::<Option<_>>()?;
    let specific: Vec<Signal> = crate::units::security::checks(rule)
        .iter()
        .filter_map(|check| {
            let (outcome, lean) = judge(check.id, get(check.id)?);
            Some(match outcome {
                Outcome::Uncertain(_) if settled(rule, check.id, get, false) => {
                    (Outcome::Clear, 0.0)
                }
                // A Choice asked whenever its check is not clear clears a
                // concern it rules out, such as HMAC signing read as a
                // password hash.
                Outcome::Review(_) | Outcome::Consider(_) if settled(rule, check.id, get, true) => {
                    (Outcome::Clear, 0.0)
                }
                _ => (outcome, lean),
            })
        })
        .collect();
    Some((presence, specific))
}

/// The strongest signal that found a concern, clear when presence or every
/// specific check rules it out, otherwise a note when a signal leans toward
/// the concern and undecided when none does.
fn exposure_level(rule: &str, presence: &[Signal], specific: &[Signal]) -> Outcome {
    let ruled_out = presence.iter().all(|(o, _)| *o == Outcome::Clear)
        || (!specific.is_empty() && specific.iter().all(|(o, _)| *o == Outcome::Clear));
    let found: Vec<Outcome> = presence
        .iter()
        .chain(specific)
        .map(|(o, _)| *o)
        .filter(|o| matches!(o, Outcome::Review(_) | Outcome::Consider(_)))
        .collect();
    let leaning = presence
        .iter()
        .chain(specific)
        .filter(|(o, _)| matches!(o, Outcome::Uncertain(_)))
        .map(|(_, lean)| *lean)
        .fold(0.0, f64::max);
    // The weak-settings checks name each kind the broad question looks for;
    // when none leans toward its kind, the broad answer alone names no
    // setting to change, so it is at most a note. On an ASP.NET Core action
    // marked `[AllowAnonymous]` on purpose it was 0.85 while every check
    // stayed at 0.30 or less.
    let unnamed = rule == catalog::UNSAFE_SETTINGS
        && !specific.is_empty()
        && specific.iter().all(|(o, lean)| {
            !probability_at_least(*lean, LEADING_PROBABILITY)
                && !matches!(o, Outcome::Review(_) | Outcome::Consider(_))
        });
    if unnamed && !found.is_empty() {
        Outcome::Note(strongest(&found).concern())
    } else if !found.is_empty() {
        strongest(&found)
    } else if ruled_out {
        Outcome::Clear
    } else if probability_at_least(leaning, LEADING_PROBABILITY) {
        Outcome::Note(leaning)
    } else {
        Outcome::Uncertain(leaning)
    }
}

/// One exposure answer with its lean. The own-messages check (`own`) settles
/// error-detail signals: all messages the program's own clears them. A
/// message carrying another error's text raised a lean toward a client to
/// a consider, and 1 of 28 such findings outside example code was right on
/// labeled projects: a central handler replaced the text with a generic
/// message, the error was one written for users, or no remote client read
/// it. The lean is now a note, like any other, worded with the carried
/// text. A signal still undecided is clear when the settle Choice sends the
/// text `away` from remote clients.
fn exposure_signal(question: &str, answer: &Answer, own: Option<Messages>, away: bool) -> Signal {
    let outcome = noul(answer);
    let lean = lean(answer);
    if !ERROR_SIGNALS.contains(&question) {
        return (outcome, lean);
    }
    match (own, outcome) {
        (Some(Messages::Own), _) => (Outcome::Clear, 0.0),
        // Text that never reaches a remote client is no error-detail leak,
        // whatever error text it carries.
        (_, Outcome::Uncertain(_)) if away => (Outcome::Clear, 0.0),
        _ => (outcome, lean),
    }
}

/// Weak settings of Django code, whose checks name its settings and
/// decorators: a settings module assigns dozens of settings, and the
/// presence question found development settings weak (`DEBUG = True`, any
/// host) as surely as deployed ones, and a signed webhook exempt from CSRF
/// as surely as a form. Only a specific check that names what is weak raises
/// a consider or review; presence alone is at most a note.
pub(in crate::units) fn django_settings_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Outcome> {
    let outcome = exposure_outcome(catalog::UNSAFE_SETTINGS, get, &["weakened"])?;
    let named = crate::units::security::checks(catalog::UNSAFE_SETTINGS)
        .iter()
        .any(|check| {
            get(check.id)
                .map(noul)
                .is_some_and(|o| matches!(o, Outcome::Review(_)))
        });
    Some(match outcome {
        Outcome::Review(p) | Outcome::Consider(p) if !named => Outcome::Note(p),
        other => other,
    })
}
