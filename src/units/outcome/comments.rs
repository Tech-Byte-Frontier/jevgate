//! Outcomes of code comments.
use super::*;

/// Each first-pass question of a comment with its outcome: the Scores read
/// as benefits (the middle level says the comment is fine as it is), the
/// Nouls as checks. Documentation that only repeats its declaration is at
/// most a note: a documentation tool or linter may expect a summary line
/// even when it says what the name says, as in the Sphinx docstrings of
/// psf/requests. Wordiness is asked only of long comments and code turned
/// off only of comments that read like code, so either may be missing.
pub(in crate::units) fn comment_signals<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
    documentation: bool,
) -> Option<Vec<(&'static str, Outcome)>> {
    let mut signals = Vec::new();
    for question in crate::units::comments::QUESTIONS {
        let Some(answer) = get(question) else {
            if question == "disabled" || question == "verbose" {
                continue;
            }
            return None;
        };
        let outcome = match question {
            "restates" if documentation => at_most_note(benefit(answer)),
            "restates" | "verbose" => benefit(answer),
            _ => noul(answer),
        };
        signals.push((question, outcome));
    }
    Some(signals)
}

fn at_most_note(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Review(p) | Outcome::Consider(p) => Outcome::Note(p),
        other => other,
    }
}

/// The strongest of a comment's signals, or when they stay undecided, the
/// kind of comment: the kinds a reader could do without reaching the
/// threshold raise a consider (a note for documentation that repeats its
/// declaration), the others reaching it clear it. Comments are cleanups,
/// never defects: at most a consider.
pub(in crate::units) fn comment_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
    documentation: bool,
) -> Option<Outcome> {
    let signals = comment_signals(get, documentation)?;
    let outcome = strongest(&signals.iter().map(|(_, o)| *o).collect::<Vec<_>>());
    let concern = comment_concern_kind(get("kind"));
    Some(cleanup(match (outcome, concern) {
        (Outcome::Uncertain(_), Some(("restates", p))) if documentation && at_least(p) => {
            Outcome::Note(p)
        }
        (Outcome::Uncertain(_), Some((_, p))) if at_least(p) => Outcome::Consider(p),
        (Outcome::Uncertain(_), Some((_, p))) if at_least(1.0 - p) => Outcome::Clear,
        _ => outcome,
    }))
}

/// The likelier kind of comment a reader could do without, and the mass of
/// all such kinds.
pub(in crate::units) fn comment_concern_kind(kind: Option<&Answer>) -> Option<(&str, f64)> {
    let Some(Answer::Choice { probabilities, .. }) = kind else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    if mass <= 0.0 {
        return None;
    }
    let concern = probabilities
        .iter()
        .filter(|(kind, _)| questions::CONCERN_KINDS.contains(&kind.as_str()));
    let likeliest = concern
        .clone()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or("restates", |(kind, _)| kind.as_str());
    Some((likeliest, concern.map(|(_, p)| p / mass).sum()))
}
