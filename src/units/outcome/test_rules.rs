//! Outcome of the test value rule.
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
        Outcome::Consider(p)
    } else if let Outcome::Review(p) = several {
        Outcome::Note(p)
    } else if hollow.iter().all(|o| *o == Outcome::Clear) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(hollow.iter().map(|o| o.concern()).fold(0.0, f64::max))
    })
}
