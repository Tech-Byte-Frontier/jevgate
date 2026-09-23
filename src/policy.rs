//! The shared decision thresholds and the comparison that applies them.

pub(crate) const REVIEW_PROBABILITY: f64 = 0.80;
pub(crate) const LOCATION_PROBABILITY: f64 = 0.65;
/// On a benefit Score whose middle level says the code is fine as it is, a
/// consider needs the top level to lead; mass on the middle alone is a note.
pub(crate) const LEADING_PROBABILITY: f64 = 0.50;

/// Aggregating and normalizing binary floats can move an exact decimal boundary
/// by a few machine rounding units. This is only an arithmetic allowance, not a
/// confidence margin; raw probabilities and the configured thresholds stay intact.
pub(crate) fn probability_at_least(value: f64, threshold: f64) -> bool {
    value.is_finite()
        && threshold.is_finite()
        && (value >= threshold || threshold - value <= 8.0 * f64::EPSILON)
}
