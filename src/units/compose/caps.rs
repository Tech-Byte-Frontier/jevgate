//! Measured limits on outcomes: the levels a unit's finding may reach once
//! its answers are composed, each set from findings labeled on the corpus.
use super::*;

/// A unit's outcome under the caps its rule and facts put on it: security
/// code at a test path or resting on what lies outside the function, a
/// short section, and comments too few to act on. The look-here rules take no caps: a coding agent
/// verifies what they flag.
pub(super) fn capped(
    unit: &UnitPlan,
    judgments: &[Judgment],
    few: &BTreeSet<&str>,
    outcome: Outcome,
) -> Outcome {
    if small_section(unit) {
        return at_most_note(outcome);
    }
    let lower = test_path_security(unit)
        || outside_function(unit, judgments)
        || few.contains(unit.id.as_str());
    if lower { lowered(outcome) } else { outcome }
}

/// Weak-setting checks whose review needs its settle Choice to name what
/// the function itself does, and the option that does: whether a token was
/// verified before the function reads it, or whether a callee or model hook
/// hashes the password it saves, lies outside the function.
pub(super) const SHOWN_IN_FUNCTION: [(&str, &str, &str); 2] = [
    ("token", "token_use", "turned_off"),
    ("hash", "password_handling", "fast_hash"),
];

/// An unsafe-settings review named only by checks of `SHOWN_IN_FUNCTION`
/// whose Choice does not name what the function itself does. Labeled by
/// hand, reviews that decoded a token to decide access were right in
/// intentionally vulnerable apps and wrong in three others (a SpacetimeDB
/// module whose host verifies tokens, a SvelteKit hook whose API verifies
/// them, an identity provider's token read over TLS), and reviews for
/// passwords saved as plain text were wrong where a service or an entity's
/// `@BeforeInsert` hook hashed them; turning `verify_signature` off and
/// hashing with MD5 in the function were right. It is one level lower.
pub(super) fn outside_function(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    if unit.rule != catalog::UNSAFE_SETTINGS {
        return false;
    }
    let (outcome, answers) = resolved(unit, judgments);
    if !matches!(outcome, Outcome::Review(_)) {
        return false;
    }
    let get = |q: &str| answers.get(q).copied();
    let named: Vec<&str> = settled_checks(unit.rule, &get)
        .into_iter()
        .filter(|(_, o)| matches!(o, Outcome::Review(_)))
        .map(|(id, _)| id)
        .collect();
    let shown = |check: &str| {
        SHOWN_IN_FUNCTION
            .iter()
            .find(|(id, ..)| *id == check)
            .is_none_or(|(_, question, option)| {
                matches!(
                    choice(get(question)),
                    Some((chosen, p)) if chosen == *option
                        && crate::policy::probability_at_least(p, crate::policy::REVIEW_PROBABILITY)
                )
            })
    };
    !named.is_empty() && !named.iter().any(|check| shown(check))
}

/// Instruction sections of fewer tokens than this cost a session too little
/// to be worth a consider.
pub(super) const SECTION_NOTE_TOKENS: usize = 15;

/// An instruction section of fewer than 15 tokens is a note: labeled by
/// hand, 1 of 10 findings on such sections was right, most of them a title
/// and a "Last updated" line read as a record of past work, against 64 of
/// 68 on larger ones.
pub(super) fn small_section(unit: &UnitPlan) -> bool {
    matches!(unit.detail, Detail::Section { tokens, .. } if tokens < SECTION_NOTE_TOKENS)
}

/// A security unit of a file at a test path, judged as application code
/// because it holds no tests, such as a test app's settings or a model only
/// tests use: like code that runs only in development, it is one level
/// lower. The dummy apps of devise and clearance and a test model hashing
/// with `password.reverse` were three wrong reviews, the only security
/// reviews or considers at test paths across 103 projects.
pub(super) fn test_path_security(unit: &UnitPlan) -> bool {
    matches!(
        unit.detail,
        Detail::Security {
            test_path: true,
            ..
        }
    )
}
