//! Messages of the maintainability rules: functions, file organization, shared logic and hardcoded values.
use super::*;

/// Splitting, or flattening when only the flatten Score reached this strength,
/// naming the located block when there is one.
pub(in crate::units) fn function_wording(
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
        (Strength::Consider, false) => (
            format!(
                "`{name}` likely mixes separate jobs; splitting it may make it easier to understand ({p:.2}).{located}"
            ),
            if block.is_some() {
                "Consider extracting the located block into a named function"
            } else {
                "Consider extracting each separate job into its own named function"
            },
        ),
        (Strength::Consider, true) => (
            format!("`{name}` has branching that likely hides its main path ({p:.2})."),
            "Consider guard clauses, early returns or a lookup table",
        ),
        (Strength::Note, false) => (
            format!("`{name}` reads well as it is; one block could be named as a helper."),
            "Optional: extract that block if it grows",
        ),
        (Strength::Note, true) => (
            format!("`{name}` is easy to follow; one condition could return early."),
            "Optional: a guard clause or early return",
        ),
    }
}

/// The file-wide concern, naming the group chosen as the module (or, for a
/// test file, as its own test file) when there is one, or the two groups
/// the choice leans toward, and the kind of file
/// when the kind decided it. A test file's finding is at most a consider.
pub(in crate::units) fn outline_wording(
    chosen: &[&GroupInfo],
    tests: bool,
    several: Option<&str>,
    strength: Strength,
    p: f64,
) -> Wording {
    let (kind, parts) = if tests {
        ("test file", "tests")
    } else {
        ("module", "members")
    };
    let shown: Vec<String> = chosen
        .iter()
        .map(|group| {
            let names: Vec<_> = group
                .names
                .iter()
                .take(6)
                .map(|n| format!("`{n}`"))
                .collect();
            let more = group.names.len().saturating_sub(names.len());
            let more = if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            };
            format!("{} ({}{more})", group.id, names.join(", "))
        })
        .collect();
    let detail = if shown.is_empty() {
        String::new()
    } else {
        format!(
            " {} would be most useful as its own {kind}.",
            shown.join(" or ")
        )
    };
    match strength {
        // A test file's note may be a lowered consider, so it does not say the file reads well.
        Strength::Note if tests => (
            format!("Some tests of this file could move to a separate test file ({p:.2}).{detail}"),
            "Optional: move those tests when you next change them",
        ),
        Strength::Note => (
            format!(
                "This file is easy to navigate as it is; a small set of {parts} could live elsewhere.{detail}"
            ),
            "Optional: move that set of members if it grows",
        ),
        Strength::Consider => (
            match several {
                Some("per_feature") => format!(
                    "This file writes out the same kind of code for several features ({p:.2}); each feature's part would be easier to find in its own {kind}.{detail}"
                ),
                Some(_) => format!("This file holds several unrelated features ({p:.2}).{detail}"),
                None => format!(
                    "Some {parts} of this file could move to a separate {kind} ({p:.2}).{detail}"
                ),
            },
            if tests {
                "Consider moving those tests into their own test file"
            } else {
                "Consider moving that set of members into its own module"
            },
        ),
        Strength::Review => (
            format!(
                "This file holds several features that would be easier to find apart ({p:.2}).{detail}"
            ),
            if !chosen.is_empty() {
                "Move that group into its own module"
            } else {
                "Split the file into one module per feature"
            },
        ),
    }
}

/// `sites` is (within one test, owner in test code, every site in a test case).
pub(in crate::units) fn pair_wording(
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
                "{name} repeat steps {place}; writing each case out is common in tests.{renamed}"
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

/// One hardcoded-value question and its words.
struct ValueSignal {
    question: &'static str,
    finding: &'static str,
    note: &'static str,
    action: &'static str,
}

const VALUE_SIGNALS: [ValueSignal; 3] = [
    ValueSignal {
        question: "environment",
        finding: "fixes a value that differs between deployments",
        note: "has a local default that configuration could own",
        action: "Read the value from configuration or the environment",
    },
    ValueSignal {
        question: "magic",
        finding: "uses a value whose meaning a reader must guess",
        note: "has a value that could be named, though its context explains it",
        action: "Give the value a descriptive constant name",
    },
    ValueSignal {
        question: "special",
        finding: "special-cases one specific identity",
        note: "special-cases one specific identity",
        action: "Move the special case into data or configuration",
    },
];

/// Each hardcoded-value signal that reached this strength, in plain words; the
/// first one's remedy is the action. A note from an undecided answer that
/// leans toward the concern says the answer was split. `unnamed` marks a
/// finding one level lower because its value was not named; its signals
/// reached the level above.
pub(in crate::units) fn values_wording(
    name: &str,
    detail: &Detail,
    (strength, lowered): (Strength, Option<Strength>),
    p: f64,
    answers: &Answers<'_>,
) -> Wording {
    let get = |q: &str| answers.get(q).copied();
    let signals = value_signals(&get, detail, true).unwrap_or_default();
    // A lowered finding, such as one whose value was not named, is lower
    // than the strength its signals reached.
    let reached_at = lowered.unwrap_or(strength);
    let reached: Vec<(&ValueSignal, bool)> = signals
        .iter()
        .filter(|(_, outcome, _)| {
            matches!(
                (reached_at, outcome),
                (Strength::Review, Outcome::Review(_))
                    | (Strength::Consider, Outcome::Consider(_))
                    | (Strength::Note, Outcome::Note(_))
            )
        })
        .filter_map(|(question, _, leaned)| {
            VALUE_SIGNALS
                .iter()
                .find(|s| s.question == *question)
                .map(|s| (s, *leaned))
        })
        .collect();
    let reasons: Vec<String> = reached
        .iter()
        .map(|(signal, leaned)| match (reached_at, leaned) {
            (Strength::Note, true) => {
                format!("may {}; the answer was split", base_form(signal.finding))
            }
            (Strength::Note, false) => signal.note.to_string(),
            _ => signal.finding.to_string(),
        })
        .collect();
    let subject = if name == "module constants" {
        "One of this file's constants".to_string()
    } else {
        format!("`{name}`")
    };
    let likely = if reached_at == Strength::Consider {
        " likely"
    } else {
        ""
    };
    (
        format!(
            "{subject}{likely} {}{}.",
            reasons.join("; "),
            shown(strength, p)
        ),
        match (strength, reached.first()) {
            (Strength::Note, _) => "Optional: name or configure the value if it changes",
            (_, Some((signal, _))) => signal.action,
            (_, None) => "Review the values",
        },
    )
}
