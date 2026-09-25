//! What the security rules share: their specific trace checks and the settle
//! Choices that clear them.
use super::*;

/// The outcomes of a rule's specific trace checks that were answered. An
/// undecided check is clear when a settle Choice that settles it puts its
/// probability on the options that clear it at the threshold, such as a URL
/// whose host is among the program's own. A Choice asked whenever its
/// checks are not clear clears a found concern too: what a PHP page joins
/// into HTML may be a number, an error message or a body its included file
/// built, which the markup check does not tell from a request value.
pub(in crate::units) fn checks<'a>(
    rule: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Vec<Outcome> {
    settled_checks(rule, get)
        .into_iter()
        .map(|(_, outcome)| outcome)
        .collect()
}

/// Each answered check's id with its outcome, after its settle Choice.
pub(in crate::units) fn settled_checks<'a>(
    rule: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
) -> Vec<(&'static str, Outcome)> {
    crate::units::security::checks(rule)
        .iter()
        .filter_map(|check| {
            let outcome = match noul(get(check.id)?) {
                Outcome::Uncertain(_) if settled(rule, check.id, get, false) => Outcome::Clear,
                Outcome::Review(_) if settled(rule, check.id, get, true) => Outcome::Clear,
                other => other,
            };
            Some((check.id, outcome))
        })
        .collect()
}

/// Whether a settle Choice of `rule` clears the check `id`: an undecided
/// one, or one that `found` a concern when the Choice is asked whenever its
/// checks are not clear.
pub(super) fn settled<'a>(
    rule: &str,
    id: &str,
    get: &impl Fn(&str) -> Option<&'a Answer>,
    found: bool,
) -> bool {
    use crate::units::security::{SETTLES, SettleWhen};
    SETTLES
        .iter()
        .filter(|kind| kind.rule == rule && kind.checks.contains(&id))
        .filter(|kind| !found || kind.when == SettleWhen::NotClear)
        .any(|kind| choice_mass(get(kind.question), kind.clears).is_some_and(at_least))
}

/// The share of a Choice's probability on `options`, when it was answered.
pub(super) fn choice_mass(answer: Option<&Answer>, options: &[&str]) -> Option<f64> {
    let Answer::Choice { probabilities, .. } = answer? else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    (mass > 0.0).then(|| {
        probabilities
            .iter()
            .filter(|(option, _)| options.contains(&option.as_str()))
            .map(|(_, p)| p / mass)
            .sum()
    })
}
