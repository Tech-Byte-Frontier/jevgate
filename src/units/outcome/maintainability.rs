//! Outcomes of function simplification's measured Scores. File organization,
//! shared logic and hardcoded values ask one look-here question (`look`).
use super::*;

/// Lines a function may span and still read in one look.
const SHORT_FUNCTION_LINES: usize = 20;

/// The stronger of splitting and (for deeply nested functions only)
/// flattening. Splitting a function of 20 lines or fewer is at most a note:
/// labeled by hand, 12 of 39 such considers were right on the projects used
/// for tuning and 5 of 50 on 23 Bend 2 projects never used for it, most of
/// them helpers that read in one look or dispatches over a token's cases.
pub(in crate::units) fn function_outcome(
    split: Option<&Answer>,
    flatten: Option<&Answer>,
    lines: usize,
) -> Option<Outcome> {
    let split = match benefit(split?) {
        Outcome::Consider(p) if lines <= SHORT_FUNCTION_LINES => Outcome::Note(p),
        outcome => outcome,
    };
    let mut outcomes = vec![split];
    outcomes.extend(flatten.map(benefit));
    Some(strongest(&outcomes))
}
