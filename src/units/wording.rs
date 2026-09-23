//! The message and recommended action of each kind of finding.
use super::{
    Block, GroupInfo,
    compose::{Answers, Outcome, benefit, levels, noul},
};
use crate::schema::{Answer, Strength};

/// A finding's message and the action it recommends.
pub(super) type Wording = (String, &'static str);

/// Splitting, or flattening when only the flatten Score reached this strength,
/// naming the located block when there is one.
pub(super) fn function_wording(
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
    block: Option<&Block>,
) -> Wording {
    let reached = |question: &str| {
        answers
            .get(question)
            .map(|a| benefit(a))
            .is_some_and(|outcome| match strength {
                Strength::Review => matches!(outcome, Outcome::Review(_)),
                Strength::Consider => matches!(outcome, Outcome::Consider(_)),
                Strength::Note => matches!(outcome, Outcome::Note(_)),
            })
    };
    let flattening = reached("flatten") && !reached("split");
    let located = block.map_or(String::new(), |block| {
        let l = &block.location;
        format!(
            " Lines {}–{} would be most useful as their own function.",
            l.start_line, l.end_line
        )
    });
    match (strength, flattening) {
        (Strength::Review, false) => (
            format!(
                "`{name}` mixes separate jobs in long blocks; splitting it would make it easier to understand ({p:.2}).{located}"
            ),
            if block.is_some() {
                "Extract the located block into a named function"
            } else {
                "Extract each separate job into its own named function"
            },
        ),
        (Strength::Review, true) => (
            format!("`{name}` has nested or repeated branches that hide its main path ({p:.2})."),
            "Flatten the control flow with guard clauses, early returns or a lookup table",
        ),
        (Strength::Consider, false) => {
            let top = answers
                .get("split")
                .and_then(|a| levels(a))
                .map_or(p, |[_, _, top]| top);
            (
                format!(
                    "`{name}` likely mixes separate jobs ({top:.2}); splitting it may make it easier to understand.{located}"
                ),
                if block.is_some() {
                    "Consider extracting the located block into a named function"
                } else {
                    "Consider extracting each separate job into its own named function"
                },
            )
        }
        (Strength::Consider, true) => (
            format!("`{name}` has branching that likely hides its main path ({p:.2})."),
            "Consider guard clauses, early returns or a lookup table",
        ),
        (Strength::Note, false) => (
            format!("`{name}` reads well as it is; one block could be named as a helper ({p:.2})."),
            "Optional: extract that block if it grows",
        ),
        (Strength::Note, true) => (
            format!("`{name}` is easy to follow; one condition could return early ({p:.2})."),
            "Optional: a guard clause or early return",
        ),
    }
}

/// The file-wide concern, naming the group chosen as the module when there is
/// one. A note says why: the file is coherent as it is, or the proposed group
/// has no users of its own elsewhere.
pub(super) fn outline_wording(
    chosen: Option<&GroupInfo>,
    strength: Strength,
    own_users: bool,
    p: f64,
) -> Wording {
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
    match strength {
        Strength::Note if !own_users => (
            format!(
                "Some members of this file could live in a separate module ({p:.2}), but no other file uses them apart from the rest, so a module would gain little.{detail}"
            ),
            "Optional: keep the file whole until another file needs that group alone",
        ),
        Strength::Note => (
            format!(
                "This file is coherent as it is; a small set of members could live elsewhere ({p:.2}).{detail}"
            ),
            "Optional: move that set of members if it grows",
        ),
        Strength::Consider => (
            format!("Some members of this file could live in a separate module ({p:.2}).{detail}"),
            "Consider moving that set of members into its own module",
        ),
        Strength::Review => (
            format!("This file holds two or more unrelated responsibilities ({p:.2}).{detail}"),
            if chosen.is_some() {
                "Move that group into its own module"
            } else {
                "Split the file along its separate purposes"
            },
        ),
    }
}

/// `sites` is (within one test, owner in test code, every site in a test case).
pub(super) fn pair_wording(
    name: &str,
    differences: &[crate::analysis::clones::Difference],
    sites: (bool, bool, bool),
    strength: Strength,
    p: f64,
) -> Wording {
    let (within_test, in_tests, in_cases) = sites;
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
    let place = if within_test {
        "inside one test"
    } else {
        "across test cases"
    };
    match strength {
        Strength::Note => (
            format!(
                "{name} repeat steps {place}; writing each case out is common in tests ({p:.2}).{renamed}"
            ),
            "Optional: a fixture, helper or table of cases if the steps grow",
        ),
        Strength::Consider if in_cases => (
            format!("{name} repeat the same steps {place} ({p:.2}).{renamed}"),
            if within_test {
                "Consider a table of cases or a local helper for the repeated steps"
            } else {
                "Consider a fixture or helper for the repeated steps"
            },
        ),
        Strength::Review => (
            format!("{name} perform the same steps for the same purpose ({p:.2}).{renamed}"),
            if in_tests {
                "Share the steps through a fixture, helper or parameterized test"
            } else {
                "Move the shared steps into one implementation"
            },
        ),
        Strength::Consider => (
            format!(
                "{name} repeat related steps; a person should decide whether they belong together ({p:.2}).{renamed}"
            ),
            "Decide whether one implementation should serve both",
        ),
    }
}

/// A question in plain words, for listing what a unit left undecided.
pub(super) fn question_label(question: &str) -> &str {
    match question {
        "split" => "splitting",
        "flatten" => "flattening",
        "same" => "same steps",
        "environment" => "environment value",
        "magic" => "unnamed value",
        "special" => "special case",
        "own_logic" => "recomputed expected value",
        "mock_only" => "checks only its mocks",
        "overlap" => "overlapping tests",
        other => other,
    }
}

/// One hardcoded-value question, how it composes, and its words.
struct ValueSignal {
    question: &'static str,
    outcome: fn(&Answer) -> Outcome,
    finding: &'static str,
    note: &'static str,
    action: &'static str,
}

const VALUE_SIGNALS: [ValueSignal; 3] = [
    ValueSignal {
        question: "environment",
        outcome: benefit,
        finding: "fixes a value that differs between deployments",
        note: "has a local default that configuration could own",
        action: "Read the value from configuration or the environment",
    },
    ValueSignal {
        question: "magic",
        outcome: benefit,
        finding: "uses a value whose meaning a reader must guess",
        note: "has a value that could be named, though its context explains it",
        action: "Give the value a descriptive constant name",
    },
    ValueSignal {
        question: "special",
        outcome: noul,
        finding: "special-cases one specific identity",
        note: "special-cases one specific identity",
        action: "Move the special case into data or configuration",
    },
];

/// Each hardcoded-value signal that reached this strength, in plain words; the
/// first one's remedy is the action.
pub(super) fn values_wording(
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> Wording {
    let reached: Vec<&ValueSignal> = VALUE_SIGNALS
        .iter()
        .filter(|signal| {
            answers.get(signal.question).is_some_and(|a| {
                matches!(
                    (strength, (signal.outcome)(a)),
                    (Strength::Review, Outcome::Review(_))
                        | (Strength::Consider, Outcome::Consider(_))
                        | (Strength::Note, Outcome::Note(_))
                )
            })
        })
        .collect();
    let reasons: Vec<&str> = reached
        .iter()
        .map(|signal| match strength {
            Strength::Note => signal.note,
            _ => signal.finding,
        })
        .collect();
    let subject = if name == "module constants" {
        "Module constants".to_string()
    } else {
        format!("`{name}`")
    };
    let likely = if strength == Strength::Consider {
        " likely"
    } else {
        ""
    };
    (
        format!("{subject}{likely} {} ({p:.2}).", reasons.join("; ")),
        match (strength, reached.first()) {
            (Strength::Note, _) => "Optional: name or configure the value if it changes",
            (_, Some(signal)) => signal.action,
            (_, None) => "Review the values",
        },
    )
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
