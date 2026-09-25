//! Outcomes of the documentation rules about instruction sections, large
//! documents and stale sections.
use super::*;

/// The kind of instruction section each signal names.
const SECTION_KINDS: [(&str, &str); 4] = [
    ("describes", "description"),
    ("commands", "commands"),
    ("generic", "generic"),
    ("history", "record"),
];

/// The share of a kind Choice's probability on one kind.
pub(super) fn kind_share(answer: Option<&Answer>, kind: &str) -> Option<f64> {
    let Some(Answer::Choice { probabilities, .. }) = answer else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    (mass > 0.0).then(|| probabilities.get(kind).copied().unwrap_or(0.0) / mass)
}

/// An undecided staleness check, cleared when the section's missing names
/// are clearly not a current part of the repository.
pub(super) fn stale_outcome(outcome: Outcome, role: Option<&Answer>) -> Outcome {
    match outcome {
        Outcome::Uncertain(_)
            if kind_share(role, "repository").is_some_and(|share| at_least(1.0 - share)) =>
        {
            Outcome::Clear
        }
        outcome => outcome,
    }
}

/// A large document: a split Score where the middle level says it is fine
/// as it is, and a Noul on whether it mainly records past work. Both are at
/// most a consider; an undecided history answer that leans toward past work
/// is a note, since no labeled living document leaned past 0.50.
pub(in crate::units) fn document_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Outcome> {
    let split = cleanup(document_split(get("split")?, get("kind")));
    let history = get("history")?;
    let past = match cleanup(noul(history)) {
        Outcome::Uncertain(p) if probability_at_least(p, LEADING_PROBABILITY) => Outcome::Note(p),
        other => other,
    };
    Some(strongest(&[split, past]))
}

/// The split Score, or when it stays undecided, the kind of document: the
/// kinds that serve one subject reaching the threshold clear it, a
/// collection of unrelated subjects reaching it raises a consider.
pub(in crate::units) fn document_split(split: &Answer, kind: Option<&Answer>) -> Outcome {
    let outcome = benefit(split);
    let (Outcome::Uncertain(_), Some(Answer::Choice { probabilities, .. })) = (outcome, kind)
    else {
        return outcome;
    };
    let mass: f64 = probabilities.values().sum();
    if mass <= 0.0 {
        return outcome;
    }
    let several: f64 = probabilities
        .iter()
        .filter(|(kind, _)| {
            crate::units::questions::SEVERAL_DOCUMENT_KINDS.contains(&kind.as_str())
        })
        .map(|(_, p)| p / mass)
        .sum();
    if at_least(1.0 - several) {
        Outcome::Clear
    } else if at_least(several) {
        Outcome::Consider(several)
    } else {
        outcome
    }
}

/// Each answered question about an instruction section with its outcome.
/// Removing documentation is a cleanup, never a defect, so a signal is at
/// most a consider. A section that loads in every session but applies to one
/// directory is a note, and only on a clear choice of that directory. An
/// undecided "only describes" or "only lists commands" settles when the
/// section is clearly not something the files show, since both findings say
/// agents read it from the files. A signal still undecided settles by the
/// section's kind, asked apart: its own kind at the threshold raises it, and
/// that kind ruled out clears it; instructions clear an undecided
/// "restates the repository".
pub(in crate::units) fn section_signals<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Vec<(&'static str, Outcome)>> {
    let kind = |name: &str| kind_share(get("kind"), name);
    let inferable = match cleanup(benefit(get("inferable")?)) {
        Outcome::Uncertain(_) if kind("instructions").is_some_and(at_least) => Outcome::Clear,
        outcome => outcome,
    };
    let mut signals = vec![("inferable", inferable)];
    for question in ["describes", "commands", "generic", "history", "enforced"] {
        if let Some(answer) = get(question) {
            let own = SECTION_KINDS
                .iter()
                .find(|(q, _)| *q == question)
                .and_then(|(_, k)| kind(k));
            signals.push((question, section_signal(question, answer, inferable, own)));
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

/// One section Noul, settled: a description or command list the files show
/// is what "inferable" asks, so when that clearly is not, these settle; any
/// other that stays undecided settles by `own`, the share of the section's
/// kind that the signal names.
fn section_signal(
    question: &str,
    answer: &Answer,
    inferable: Outcome,
    own: Option<f64>,
) -> Outcome {
    match cleanup(noul(answer)) {
        Outcome::Uncertain(_)
            if inferable == Outcome::Clear && matches!(question, "describes" | "commands") =>
        {
            Outcome::Clear
        }
        Outcome::Uncertain(p) => match own {
            Some(share) if at_least(share) => Outcome::Consider(share),
            Some(share) if at_least(1.0 - share) => Outcome::Clear,
            _ => Outcome::Uncertain(p),
        },
        outcome => outcome,
    }
}
