//! Messages of the maintainability rules: function simplification's measured
//! findings, and the look-here findings of every maintainability rule.
use super::*;

/// Splitting, or flattening when only the flatten Score reached this strength,
/// naming the located block when there is one.
pub(in crate::units) fn function_wording(
    name: &str,
    strength: Strength,
    answers: &Answers<'_>,
    (block, bend): (Option<&Block>, bool),
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
                "`{name}` mixes separate jobs in long blocks; splitting it would make it easier to understand.{located}"
            ),
            if block.is_some() {
                "Extract the located block into a named function"
            } else {
                "Extract each separate job into its own named function"
            },
        ),
        (Strength::Review, true) => (
            format!("`{name}` has nested or repeated branches that hide its main path."),
            if bend {
                "Flatten the matches with nested patterns, a `case _:` fallback or a helper def"
            } else {
                "Flatten the control flow with guard clauses, early returns or a lookup table"
            },
        ),
        (Strength::Consider, false) => (
            format!(
                "`{name}` likely mixes separate jobs; splitting it may make it easier to understand.{located}"
            ),
            if block.is_some() {
                "Consider extracting the located block into a named function"
            } else {
                "Consider extracting each separate job into its own named function"
            },
        ),
        (Strength::Consider, true) => (
            format!("`{name}` has branching that likely hides its main path."),
            if bend {
                "Consider nested patterns, a `case _:` fallback or a helper def"
            } else {
                "Consider guard clauses, early returns or a lookup table"
            },
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

/// A look-here finding: what to look for, since the coding agent that reads
/// the unit decides what, if anything, to change, and dismisses the finding
/// with a reason when nothing should.
pub(in crate::units) fn look_wording(unit: &crate::units::UnitPlan) -> Wording {
    let name = &unit.name;
    match &unit.detail {
        Detail::Outline { tests: true, .. } => (
            "This test file may test several separate subjects.".into(),
            "Move each subject's tests to its own file, or dismiss this finding with a reason",
        ),
        Detail::Outline { .. } => (
            "This file may do several separate kinds of work, such as separate features, layers or integrations.".into(),
            "Move each separate part to its own module, or dismiss this finding with a reason",
        ),
        Detail::Pair => (
            format!("{name} may repeat one piece of logic, so a change to it would have to be made in each place."),
            "Keep the logic in one shared function, or dismiss this finding with a reason if the copies must stay separate",
        ),
        Detail::Constants { .. } => (
            "A constant of this file may fix a value worth a look: one that differs between environments or singles out one record.".into(),
            "Read the value from configuration or data, or dismiss this finding with a reason",
        ),
        Detail::Values { .. } => (
            format!("`{name}` may hold a fixed value worth a look: a special-cased record, a value that differs between environments, or an unexplained number."),
            "Name the value or read it from configuration or data, or dismiss this finding with a reason",
        ),
        _ => (
            format!("`{name}` could likely be made simpler to read or change: it may be long, deeply nested, repetitive or mix separate jobs."),
            "Simplify what makes it hard to follow, or dismiss this finding with a reason if it reads well as it is",
        ),
    }
}
