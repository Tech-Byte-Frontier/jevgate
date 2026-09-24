//! Outcomes of the access-control rule: SQL policies and grants, and
//! SpacetimeDB tables, views and reducers.
use super::*;

/// Policies and grants whose intent may be public data are at most a
/// consider, as is an open `search_path`; a SECURITY DEFINER function that
/// skips checking its caller can be a review.
pub(super) fn access_outcome<'a>(
    get: &impl Fn(&str) -> Option<&'a Answer>,
    detail: &Detail,
) -> Option<Outcome> {
    let Detail::Access(access) = detail else {
        return None;
    };
    Some(match access {
        Access::Policy { .. } => strongest(&[
            cleanup(noul(get("others")?)),
            cleanup(noul(get("editable")?)),
        ]),
        Access::Definer => {
            strongest(&[noul(get("unchecked")?), cleanup(noul(get("search_path")?))])
        }
        Access::Grant => cleanup(noul(get("broad")?)),
        Access::Table => cleanup(module_outcome(get("data")?, &[get("exposed")?])),
        Access::View => cleanup(module_outcome(get("rows")?, &[get("returns_others")?])),
        Access::Reducer => module_outcome(
            get("reach")?,
            &[get("argument_rows")?, get("operator_only")?],
        ),
    })
}

/// A SpacetimeDB definition: review when a concern Noul or the Score's top
/// level reaches the threshold, clear when the Score's acceptable levels do
/// and nothing is at review, otherwise uncertain. On a probe of real module
/// code and mutants with a check removed, no real definition reached review
/// and no mutant cleared.
fn module_outcome(score: &Answer, checks: &[&Answer]) -> Outcome {
    let mut signals: Vec<Outcome> = checks.iter().map(|a| noul(a)).collect();
    signals.push(acceptable_levels(score));
    let review = signals
        .iter()
        .filter(|o| matches!(o, Outcome::Review(_)))
        .map(|o| o.concern())
        .reduce(f64::max);
    match (review, acceptable_levels(score)) {
        (Some(p), _) => Outcome::Review(p),
        (None, Outcome::Clear) => Outcome::Clear,
        _ => Outcome::Uncertain(signals.iter().map(|o| o.concern()).fold(0.0, f64::max)),
    }
}
