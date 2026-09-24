//! Outcomes of the documentation rules: sections, documents and section pairs.
use super::*;

/// A pair of sections repeats itself when either covers the other, unless one
/// translates the other; a disagreement counts either way, since a
/// translation that disagrees with its original is out of date.
pub(super) fn doc_pair_outcome<'a>(get: &impl Fn(&str) -> Option<&'a Answer>) -> Option<Outcome> {
    let translated = get("translation").is_some_and(|a| matches!(noul(a), Outcome::Review(_)));
    let covers = if translated {
        &[][..]
    } else {
        &["a_covers", "b_covers"][..]
    };
    covers
        .iter()
        .chain(&["conflict"])
        .map(|q| get(q).map(|a| cleanup(noul(a))))
        .collect::<Option<Vec<_>>>()
        .map(|signals| strongest(&signals))
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
pub(in crate::units) fn section_signals<'a>(
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
