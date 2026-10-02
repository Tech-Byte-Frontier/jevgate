//! The shared decision thresholds and the comparison that applies them.

pub(crate) const REVIEW_PROBABILITY: f64 = 0.80;
pub(crate) const LOCATION_PROBABILITY: f64 = 0.65;
/// On a benefit Score whose middle level says the code is fine as it is, a
/// consider needs the top level to lead; mass on the middle alone is a note.
pub(crate) const LEADING_PROBABILITY: f64 = 0.50;
/// A look-here question flags its unit for a coding agent to verify at this
/// probability. On a 32-file development sample, function, value and copy
/// flags at 0.70 were 26 real, 15 debatable and 4 noise once read; on files
/// labeled for splitting, 6 of 9 against 5 of 54 kept, the recall of 0.60
/// with a third of its flags on kept files.
pub(crate) const LOOK_PROBABILITY: f64 = 0.70;

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
    fn the_report_records_the_shared_thresholds() {
        let policy = crate::catalog::policy();
        assert_eq!(policy["consider_probability"], REVIEW_PROBABILITY);
        assert_eq!(policy["look_probability"], LOOK_PROBABILITY);
    }
}
