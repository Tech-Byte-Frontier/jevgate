//! The shared decision thresholds, the thresholds measured for single
//! questions where the labels disagree with them, and the comparison that
//! applies them.
use crate::{catalog, schema::Strength};

pub(crate) const REVIEW_PROBABILITY: f64 = 0.80;
pub(crate) const LOCATION_PROBABILITY: f64 = 0.65;
/// On a benefit Score whose middle level says the code is fine as it is, a
/// consider needs the top level to lead; mass on the middle alone is a note.
pub(crate) const LEADING_PROBABILITY: f64 = 0.50;

/// A threshold measured for one question: a finding whose level `question`'s
/// answer set at `level` keeps that level only when the answer's probability
/// reaches `threshold`; below it, the finding is one level lower.
pub(crate) struct Calibrated {
    pub rule: &'static str,
    pub question: &'static str,
    pub level: Strength,
    pub threshold: f64,
}

/// A shared-logic consider set by the same-steps Score's middle-or-top mass
/// needs 0.90 there. With a tenth to a fifth of the mass left on "different
/// work that only looks alike", such considers were right 25 times in 54 on
/// the projects the rules were tuned on and 9 in 29 on the projects JevGate
/// was never tuned on, against 32 in 44 and 13 in 23 at 0.90 or more: most
/// of the wrong ones were spans too small to share or copies whose
/// differences were the point. A consider that is a review lowered by a cap
/// keeps its level, since the top level set it: on the tuned projects those
/// were right about as often at any probability (on the unseen ones, 36% of
/// the time below 0.85 and 69% at 0.98 or more, over 105 labels).
const SHARED_CONSIDER: Calibrated = Calibrated {
    rule: catalog::SHARED_LOGIC,
    question: "same",
    level: Strength::Consider,
    threshold: 0.90,
};

/// The thresholds measured per question, fitted on the labeled findings of
/// the projects the rules were tuned on and kept only where, on the 25
/// projects JevGate was never tuned on, they removed at least as many wrong
/// findings as right ones and raised the level's precision. Every rule,
/// level and deciding question with at least 20 labels was checked
/// (`evaluation/reliability.py` in the maintainer's clone); every other
/// question keeps the shared thresholds.
pub(crate) const CALIBRATED: [Calibrated; 1] = [SHARED_CONSIDER];

/// Whether a threshold was measured for one of `rule`'s questions (a
/// catalog key) and kept on the projects never used for tuning.
pub(crate) fn calibrated(rule: &str) -> bool {
    CALIBRATED.iter().any(|entry| entry.rule == rule)
}

/// Aggregating and normalizing binary floats can move an exact decimal boundary
/// by a few machine rounding units. This is only an arithmetic allowance, not a
/// confidence margin; raw probabilities and the configured thresholds stay intact.
const ROUNDING_UNITS: f64 = 8.0;

pub(crate) fn probability_at_least(value: f64, threshold: f64) -> bool {
    value.is_finite()
        && threshold.is_finite()
        && (value >= threshold || threshold - value <= ROUNDING_UNITS * f64::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_measured_threshold_names_a_rule_and_raises_the_shared_one() {
        let keys = catalog::keys();
        for entry in &CALIBRATED {
            assert!(keys.contains(&entry.rule), "{}", entry.rule);
            assert_ne!(entry.level, Strength::Note, "{}", entry.rule);
            assert!(
                entry.threshold > REVIEW_PROBABILITY && entry.threshold < 1.0,
                "{} {}",
                entry.rule,
                entry.question
            );
        }
        let policy = catalog::policy();
        assert_eq!(policy["shared_logic_same_consider_probability"], 0.90);
        assert_eq!(policy["consider_probability"], REVIEW_PROBABILITY);
    }
}
