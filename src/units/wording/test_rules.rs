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

/// A Bend 2 law whose comment promises more than, or other than, it states,
/// naming what it adds when the law's recheck chose it.
pub(in crate::units) fn law_wording(
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> Wording {
    if strength == Strength::Note {
        return (
            format!("The comment above law `{name}` says slightly more than the law states."),
            "Check that the comment and the law say the same thing",
        );
    }
    let fixed = particular_inputs(answers.get("fixed").copied()).is_some();
    let adds = match choice(answers.get("relation").copied()) {
        _ if fixed => ": the law checks particular inputs where the comment speaks of any",
        Some(("property", _)) => ": it claims a property the law does not state",
        Some(("inputs", _)) => ": it claims the law for more inputs than the law covers",
        Some(("condition", _)) => {
            ": it claims a consequence the law yields only under a condition it does not state"
        }
        _ => "",
    };
    (
        format!(
            "The comment above law `{name}` promises more than the law states{}{adds}. A definition could break that promise while every proof passes.",
            shown(strength, p)
        ),
        "State the comment's promise in the law, or narrow the comment to what the law states",
    )
}
