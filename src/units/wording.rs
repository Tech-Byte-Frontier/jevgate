//! The message and recommended action of each kind of finding.
use super::{
    GroupInfo,
    compose::{Answers, Outcome, levels, noul, score},
};
use crate::schema::Strength;

/// A finding's message and the action it recommends.
pub(super) type Wording = (String, &'static str);

/// Splitting, or flattening when only the flatten Score reached this strength.
pub(super) fn function_wording(
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> Wording {
    let reached = |question: &str| {
        answers
            .get(question)
            .map(|a| score(a))
            .is_some_and(|outcome| match strength {
                Strength::Review => matches!(outcome, Outcome::Review(_)),
                Strength::Consider => matches!(outcome, Outcome::Consider(_)),
            })
    };
    let flattening = reached("flatten") && !reached("split");
    match (strength == Strength::Review, flattening) {
        (true, false) => (
            format!(
                "`{name}` mixes separate jobs in long blocks; splitting it would make it easier to understand ({p:.2})."
            ),
            "Extract each separate job into its own named function",
        ),
        (true, true) => (
            format!("`{name}` has nested or repeated branches that hide its main path ({p:.2})."),
            "Flatten the control flow with guard clauses, early returns or a lookup table",
        ),
        (false, false) => match answers.get("split").and_then(|a| levels(a)) {
            // The top level leads without reaching review: say so, with its probability.
            Some([_, _, top]) if top >= 0.5 => (
                format!(
                    "`{name}` likely mixes separate jobs ({top:.2}); splitting it may make it easier to understand."
                ),
                "Consider extracting each separate job into its own named function",
            ),
            _ => (
                format!("`{name}` has a block that could be named as a helper ({p:.2})."),
                "Consider extracting that block into a named function",
            ),
        },
        (false, true) => (
            format!("`{name}` has branching that could return early ({p:.2})."),
            "Consider guard clauses or early returns",
        ),
    }
}

/// The file-wide concern, naming the group chosen as the module when there is one.
pub(super) fn outline_wording(chosen: Option<&GroupInfo>, review: bool, p: f64) -> Wording {
    let detail = chosen.map_or(String::new(), |group| {
        let shown: Vec<_> = group
            .names
            .iter()
            .take(6)
            .map(|n| format!("`{n}`"))
            .collect();
        let more = group.names.len().saturating_sub(shown.len());
        let more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        format!(
            " {} ({}{more}) would be most useful as its own module.",
            group.id,
            shown.join(", ")
        )
    });
    if !review {
        return (
            format!("Some members of this file could live in a separate module ({p:.2}).{detail}"),
            "Consider moving that set of members into its own module",
        );
    }
    (
        format!("This file holds two or more unrelated responsibilities ({p:.2}).{detail}"),
        if chosen.is_some() {
            "Move that group into its own module"
        } else {
            "Split the file along its separate purposes"
        },
    )
}

pub(super) fn pair_wording(
    name: &str,
    differences: &[crate::analysis::clones::Difference],
    within_test: bool,
    in_tests: bool,
    review: bool,
    p: f64,
) -> Wording {
    let renamed = if differences.is_empty() {
        String::new()
    } else {
        let shown: Vec<_> = differences
            .iter()
            .take(6)
            .map(|d| format!("`{}`→`{}`", d.a, d.b))
            .collect();
        format!(" Differences: {}.", shown.join(", "))
    };
    if within_test {
        (
            format!("{name} repeat the same steps inside one test ({p:.2}).{renamed}"),
            "Consider a table of cases or a local helper for the repeated steps",
        )
    } else if review {
        (
            format!("{name} perform the same steps for the same purpose ({p:.2}).{renamed}"),
            if in_tests {
                "Share the steps through a fixture, helper or parameterized test"
            } else {
                "Move the shared steps into one implementation"
            },
        )
    } else {
        (
            format!(
                "{name} repeat related steps; a person should decide whether they belong together ({p:.2}).{renamed}"
            ),
            "Decide whether one implementation should serve both",
        )
    }
}

/// Each test-value signal that reached review, in plain words.
pub(super) fn test_wording(name: &str, review: bool, p: f64, answers: &Answers<'_>) -> Wording {
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
        format!("`{name}` {} ({p:.2}).", reasons.join("; ")),
        if review {
            "Assert on the behavior of the code under test with an independent expected value"
        } else {
            "Assert on observable results, one behavior per test"
        },
    )
}

pub(super) fn test_pair_wording(name: &str, review: bool, p: f64) -> Wording {
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
