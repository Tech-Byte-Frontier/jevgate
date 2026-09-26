//! Outcomes of the test value and redundancy rules.
use super::*;

/// Hollow signals decide review and clear; internal details can raise a
/// consider, and their uncertainty does not block a clear. "Several unrelated
/// behaviors" is only a note: on labeled tests it rated tables of inputs and
/// browser journeys as high as tests that really mix behaviors.
pub(in crate::units) fn test_value_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Option<Outcome> {
    let hollow = [noul(get("own_logic")?), noul(get("mock_only")?)];
    let weak = [noul(get("internal")?)];
    let several = noul(get("several")?);
    let strongest = |outcomes: &[Outcome]| {
        outcomes
            .iter()
            .filter_map(|o| match o {
                Outcome::Review(p) => Some(*p),
                _ => None,
            })
            .reduce(f64::max)
    };
    Some(if let Some(p) = strongest(&hollow) {
        Outcome::Review(p)
    } else if let Some(p) = strongest(&weak) {
        internal_outcome(p, get("reads"))
    } else if let Outcome::Review(p) = several {
        Outcome::Note(p)
    } else if hollow.iter().all(|o| *o == Outcome::Clear) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(hollow.iter().map(|o| o.concern()).fold(0.0, f64::max))
    })
}

/// An internal-details consider, weighed with what the test's assertions
/// read once that is asked: results, state or effects a caller observes at
/// the threshold clear it, stored input or the program's own calls leading
/// keep it, and otherwise it is a note.
fn internal_outcome(p: f64, reads: Option<&Answer>) -> Outcome {
    let Some(Answer::Choice { probabilities, .. }) = reads else {
        return Outcome::Consider(p);
    };
    let mass: f64 = probabilities.values().sum();
    if mass <= 0.0 {
        return Outcome::Consider(p);
    }
    let observed: f64 = probabilities
        .iter()
        .filter(|(kind, _)| questions::OBSERVED_READS.contains(&kind.as_str()))
        .map(|(_, q)| q / mass)
        .sum();
    if at_least(observed) {
        Outcome::Clear
    } else if probability_at_least(1.0 - observed, LEADING_PROBABILITY) {
        Outcome::Consider(p)
    } else {
        Outcome::Note(p)
    }
}

/// Two tests that check the same behavior with equivalent inputs make a
/// review: one of them adds nothing. A review also needs both tests to
/// exercise the same input case and expect the same outcome, each at the
/// shared threshold; otherwise the pair is at most a consider. On just, a
/// test formatting `x:=` from stdin read as redundant with one checking
/// already-formatted input (same input 0.69), and on zero2prod a valid
/// 256-grapheme name with a rejected 257-grapheme one (same outcome 0.05).
/// Tests `identical` apart from their names need neither: their names can
/// say different cases, as lobsters' "when story is deleted" and "when story
/// is available" did, which deleted nothing. When asked (for Ruby) whether
/// each checks something the other does not, a review also needs that ruled
/// out.
pub(in crate::units) fn redundancy_outcome<'a>(
    overlap: &Answer,
    get: &impl Fn(&str) -> Option<&'a Answer>,
    identical: bool,
) -> Outcome {
    let outcome = score(overlap);
    let separable = get("distinct")
        .is_some_and(|answer| matches!(answer, Answer::Noul { noul } if !at_least(1.0 - noul)));
    let differs = !identical
        && ["same_input", "same_outcome"]
            .iter()
            .any(|q| matches!(get(q), Some(Answer::Noul { noul }) if !at_least(*noul)));
    // Different inputs with outcomes that clearly differ, such as a range
    // that parses and one that does not, are separate behaviors to keep.
    let outcomes_differ = !identical
        && matches!(get("same_outcome"), Some(Answer::Noul { noul }) if at_least(1.0 - noul));
    match (outcome, levels(overlap)) {
        (Outcome::Review(_), Some([_, middle, top])) if separable || differs => {
            Outcome::Consider(middle + top)
        }
        (Outcome::Consider(p), _) if outcomes_differ => Outcome::Note(p),
        _ => outcome,
    }
}
