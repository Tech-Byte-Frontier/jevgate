//! Outcomes of repeated or contradicting section pairs.
use super::{documentation::kind_share, *};

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
