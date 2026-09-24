//! Messages of the test value and redundancy rules.
use super::*;

/// Each test-value signal that reached review, in plain words.
pub(in crate::units) fn test_wording(
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> Wording {
    let reasons: Vec<&str> = [
        (
            "own_logic",
            "computes its expected value with the logic it tests",
        ),
        (
            "mock_only",
            "only checks values its mocks were set to return",
        ),
        (
            "internal",
            "asserts internal details instead of observable results",
        ),
        ("several", "checks several unrelated behaviors"),
    ]
    .into_iter()
    .filter(|(q, _)| matches!(answers.get(q).map(|a| noul(a)), Some(Outcome::Review(_))))
    .map(|(_, text)| text)
    .collect();
    (
        format!("`{name}` {}{}.", reasons.join("; "), shown(strength, p)),
        if strength == Strength::Review {
            "Assert on the behavior of the code under test with an independent expected value"
        } else {
            "Assert on observable results, one behavior per test"
        },
    )
}

pub(in crate::units) fn test_pair_wording(name: &str, review: bool, p: f64) -> Wording {
    if review {
        (
            format!(
                "{name} check the same behavior with equivalent inputs; one adds nothing ({p:.2})."
            ),
            "Remove one of the tests",
        )
    } else {
        (
            format!("{name} check the same behavior with different inputs ({p:.2})."),
            "Combine them into one parameterized test",
        )
    }
}
