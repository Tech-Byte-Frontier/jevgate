//! Outcomes of the documentation rules: sections, documents and section pairs.
use super::*;

/// The kind of instruction section each signal names.
const SECTION_KINDS: [(&str, &str); 4] = [
    ("describes", "description"),
    ("commands", "commands"),
    ("generic", "generic"),
    ("history", "record"),
];

/// The share of a kind Choice's probability on one kind.
fn kind_share(answer: Option<&Answer>, kind: &str) -> Option<f64> {
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

/// A pair of sections repeats itself when either covers the other, unless one
/// translates the other; a disagreement counts either way, since a
/// translation that disagrees with its original is out of date. Each is a
/// Score whose middle level is acceptable: covering most of the other, or
/// differing only in detail or examples, raises nothing; repeating at least
/// most of the other without all of it, or leaning toward a contradiction,
/// is a note. Sections about different subjects settle what stays
/// undecided, and so does the pair's relation, asked apart: a repetition
/// or a contradiction it rules out clears that check.
pub(super) fn doc_pair_outcome<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> Option<Outcome> {
    let signals: Vec<Outcome> = pair_signals(get)?.into_iter().map(|(_, o)| o).collect();
    Some(strongest(&signals))
}

/// Each check of a section pair with its settled outcome.
pub(in crate::units) fn pair_signals<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Vec<(&'static str, Outcome)>> {
    let translated = get("translation").is_some_and(|a| matches!(noul(a), Outcome::Review(_)));
    let covers = if translated {
        &[][..]
    } else {
        &["a_covers", "b_covers"][..]
    };
    // Different subjects settle an undecided answer, never a decided one.
    let different = get("subject").is_some_and(|a| noul(a) == Outcome::Clear);
    let ruled_out = |relation: &str| {
        kind_share(get("relation"), relation).is_some_and(|share| at_least(1.0 - share))
    };
    let settle = |outcome: Outcome, relation: &str| match outcome {
        Outcome::Uncertain(_) if different || ruled_out(relation) => Outcome::Clear,
        outcome => outcome,
    };
    let mut signals = covers
        .iter()
        .map(|q| get(q).map(|a| (*q, settle(cleanup(repeated(a)), "repeats"))))
        .collect::<Option<Vec<_>>>()?;
    signals.push((
        "conflict",
        settle(cleanup(disagreement(get("conflict")?)), "contradict"),
    ));
    Some(signals)
}

/// A conflict Score: its top level raises a finding, its two lower levels
/// clear, and an undecided answer that leans toward a contradiction is a
/// note. On vercel/ai most pairs leaning past 0.50 did disagree, such as a
/// README passing `messages` where the reference passes `uiMessages`.
pub(in crate::units) fn disagreement(answer: &Answer) -> Outcome {
    match acceptable_levels(answer) {
        Outcome::Uncertain(top) if probability_at_least(top, LEADING_PROBABILITY) => {
            Outcome::Note(top)
        }
        outcome => outcome,
    }
}

/// A repetition Score: its top level, everything, raises a finding; its two
/// lower levels clear. When the answer rules out "states things the other
/// does not" but not "most", the section repeats at least most of the other:
/// a note, which never claims all of it.
pub(in crate::units) fn repeated(answer: &Answer) -> Outcome {
    let Some([bottom, middle, top]) = levels(answer) else {
        return Outcome::Missing;
    };
    if at_least(top) {
        Outcome::Review(top)
    } else if at_least(bottom + middle) {
        Outcome::Clear
    } else if at_least(middle + top) {
        Outcome::Note(middle + top)
    } else {
        Outcome::Uncertain(top)
    }
}

/// A large document: a split Score where the middle level says it is fine
/// as it is, and a Noul on whether it mainly records past work. Both are at
/// most a consider; an undecided history answer that leans toward past work
/// is a note, since no labeled living document leaned past 0.50.
pub(in crate::units) fn document_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Outcome> {
    let capped = |outcome: Outcome| match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    };
    let split = capped(document_split(get("split")?, get("kind")));
    let history = get("history")?;
    let past = match capped(noul(history)) {
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
    let capped = |outcome: Outcome| match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    };
    let kind = |name: &str| kind_share(get("kind"), name);
    let inferable = match capped(benefit(get("inferable")?)) {
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
            let outcome = match capped(noul(answer)) {
                // A description or command list the files show is what
                // "inferable" asks; when it clearly is not, these settle.
                Outcome::Uncertain(_)
                    if inferable == Outcome::Clear
                        && matches!(question, "describes" | "commands") =>
                {
                    Outcome::Clear
                }
                Outcome::Uncertain(p) => match own {
                    Some(share) if at_least(share) => Outcome::Consider(share),
                    Some(share) if at_least(1.0 - share) => Outcome::Clear,
                    _ => Outcome::Uncertain(p),
                },
                outcome => outcome,
            };
            signals.push((question, outcome));
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
