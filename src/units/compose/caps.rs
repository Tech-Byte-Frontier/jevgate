//! Measured limits on outcomes: the levels a unit's finding may reach once
//! its answers are composed, each set from findings labeled on the corpus.
use super::*;

/// A unit's outcome under the caps its rule and facts put on it: a level set
/// below its question's measured threshold, an unnamed or single-use value,
/// a value that only needs a name, security code at a test path or resting
/// on what lies outside the function, a short outline or section, an outline
/// naming no group, and comments too few to act on.
pub(super) fn capped(
    unit: &UnitPlan,
    judgments: &[Judgment],
    few: &BTreeSet<&str>,
    outcome: Outcome,
) -> Outcome {
    let outcome = calibrated(unit, judgments, outcome);
    if unnamed_value(unit, judgments) {
        return lowered(lowered(outcome));
    }
    if single_use_value(unit, judgments)
        || readable_value(unit, judgments)
        || same_everywhere(unit, judgments)
        || short_outline(unit)
        || sectioned_outline(unit)
        || small_section(unit)
    {
        return at_most_note(outcome);
    }
    if named_value_only(unit, judgments) || bend_outline(unit) {
        return at_most_consider(outcome);
    }
    let lower = test_path_security(unit)
        || outside_function(unit, judgments)
        || unnamed_outline(unit, judgments)
        || few.contains(unit.id.as_str());
    if lower { lowered(outcome) } else { outcome }
}

/// One level lower when a question with a measured threshold
/// (`policy::CALIBRATED`) set the unit's level below it: the unit's outcome
/// is exactly what that question's answer gives under the shared thresholds,
/// so its probability is the finding's. Caps come after the follow-ups are
/// chosen, so a measured threshold never changes what is asked.
pub(super) fn calibrated(unit: &UnitPlan, judgments: &[Judgment], outcome: Outcome) -> Outcome {
    let Some((level, p)) = strength_of(outcome) else {
        return outcome;
    };
    let below: Vec<&str> = crate::policy::CALIBRATED
        .iter()
        .filter(|entry| {
            entry.rule == unit.rule
                && entry.level == level
                && !crate::policy::probability_at_least(p, entry.threshold)
        })
        .map(|entry| entry.question)
        .collect();
    if below.is_empty() {
        return outcome;
    }
    let (_, answers) = resolved(unit, judgments);
    let own = |answer: &Answer| match answer {
        Answer::Noul { .. } => noul(answer),
        _ => score(answer),
    };
    let set_by = |question: &&str| {
        answers
            .get(question)
            .is_some_and(|answer| own(answer) == outcome)
    };
    if below.iter().any(set_by) {
        lowered(outcome)
    } else {
        outcome
    }
}

/// Such a consider whose value, asked what it is, clearly reads for itself
/// where it is used: the field or argument it fills or a comment beside it
/// says what it is, or it is an idiom or a hand-tuned number, together at
/// the threshold of a clear located part. Its finding is a note; a value
/// with copies that must change together, or that nothing explains, stays
/// a consider. Leaning toward those kinds was not enough: at 0.50 they took
/// 30 of 46 right considers with 47 of 64 wrong ones.
pub(super) fn readable_value(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    named_value_only(unit, judgments)
        && choice_mass(
            answers(judgments, &unit.id, Pass::Locate)
                .get("value_kind")
                .copied(),
            &crate::units::questions::READABLE_VALUES,
        )
        .is_some_and(|p| {
            crate::policy::probability_at_least(p, crate::policy::LOCATION_PROBABILITY)
        })
}

/// A finding that rests on the environment whose value or constant, asked
/// where it would differ, needs no configuration at the review threshold:
/// the same in every copy of the program on purpose, a fallback used only
/// when configuration gives none, or code no deployment runs. Its finding
/// is a note. Labeled by hand, that took 17 of 36 wrong reviews and
/// considers and 2 of 17 right ones (a frontend's API host, edited in code
/// three times, and a template author's domain as a fallback); leaning at
/// 0.50 would have taken 25 wrong and 6 right.
pub(super) fn same_everywhere(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    environment_only(unit, judgments)
        && choice_mass(
            answers(judgments, &unit.id, Pass::Locate)
                .get("environment_kind")
                .copied(),
            &crate::units::questions::SAME_EVERYWHERE,
        )
        .is_some_and(|p| crate::policy::probability_at_least(p, crate::policy::REVIEW_PROBABILITY))
}

/// A function's hardcoded-value review or consider whose value was not
/// named: the locate Choice picked none clearly, or there were too many
/// values to offer. Its finding is a note, since a reader cannot tell what
/// to change: one level lower, 8 of lobsters' 10 such considers were wrong.
pub(super) fn unnamed_value(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    matches!(unit.detail, Detail::Values { .. })
        && located_value(unit, judgments).is_none()
        && matches!(
            resolved(unit, judgments).0,
            Outcome::Review(_) | Outcome::Consider(_)
        )
}

/// A hardcoded-value review or consider that rests only on whether a value
/// needs a name. Naming a value is a cleanup, so it is at most a consider:
/// labeled by hand, 17 such reviews were right and 18 wrong, most of the
/// wrong ones tuning in game, audio and animation code (a scheduler's
/// 500 ms, a hash seed, a mix gain, a float epsilon).
pub(super) fn named_value_only(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    matches!(unit.detail, Detail::Values { .. }) && rests_only_on(unit, judgments, "magic")
}

/// A hardcoded-value review or consider that rests only on whether a value
/// changes between environments: a file's constants always do.
pub(super) fn environment_only(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    rests_only_on(unit, judgments, "environment")
}

/// A hardcoded-value review or consider whose raised questions are all
/// `question`.
pub(super) fn rests_only_on(unit: &UnitPlan, judgments: &[Judgment], question: &str) -> bool {
    if !matches!(
        unit.detail,
        Detail::Values { .. } | Detail::Constants { .. }
    ) {
        return false;
    }
    let (outcome, answers) = resolved(unit, judgments);
    if !matches!(outcome, Outcome::Review(_) | Outcome::Consider(_)) {
        return false;
    }
    let get = |q: &str| answers.get(q).copied();
    crate::units::outcome::value_signals(&get, &unit.detail, true)
        .unwrap_or_default()
        .iter()
        .filter(|(_, o, _)| matches!(o, Outcome::Review(_) | Outcome::Consider(_)))
        .all(|(raised, ..)| *raised == question)
}

/// Such a finding about a value its file writes once is a note: labeled by
/// hand on 35 projects, those considers were right 19 times in 52, against
/// 34 in 49 for a value its file repeats. A delay given to `setTimeout`, a
/// size given to an attribute or a CSS class reads where it is used; a value
/// written twice can drift apart.
pub(super) fn single_use_value(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    let Detail::Values { repeated, .. } = &unit.detail else {
        return false;
    };
    named_value_only(unit, judgments)
        && located_option(unit, judgments, ("value", 'v'))
            .is_some_and(|i| repeated.get(i) == Some(&false))
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

/// Why a hardcoded-value finding is below the level its answers reached,
/// with that level.
pub(super) fn lowered_value(
    unit: &UnitPlan,
    judgments: &[Judgment],
) -> Option<(Strength, &'static str)> {
    let why = if unnamed_value(unit, judgments) {
        "No single value stood out, so it is a note."
    } else if single_use_value(unit, judgments) {
        "It is written once in its file, so it is a note."
    } else if readable_value(unit, judgments) {
        "It reads for itself where it is used, so it is a note."
    } else if same_everywhere(unit, judgments) {
        "It likely stays the same wherever the program runs, or is only a fallback, so it is a note."
    } else if named_value_only(unit, judgments)
        && matches!(resolved(unit, judgments).0, Outcome::Review(_))
    {
        ""
    } else {
        return None;
    };
    strength_of(resolved(unit, judgments).0).map(|(s, _)| (s, why))
}

/// A review lowered to a consider; other outcomes as they are.
pub(super) fn at_most_consider(outcome: Outcome) -> Outcome {
    match outcome {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    }
}

/// A file-organization consider that says only that some members could
/// move, naming no group: the module Choice was not asked (one group or
/// none) or spread wider than two groups, and no kind of file decided it.
/// Its finding is a note, since a reader cannot tell which members to move.
/// A review, or a consider the kind decided, says to split the whole file.
pub(super) fn unnamed_outline(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    let Detail::Outline {
        groups,
        members,
        parts,
        ..
    } = &unit.detail
    else {
        return false;
    };
    let (outcome, answers) = resolved(unit, judgments);
    let get = |q: &str| answers.get(q).copied();
    matches!(outcome, Outcome::Consider(_))
        && several_kind(get("split"), get("kind")).is_none()
        && outline_groups(get("module"), groups, *members).is_empty()
        && deciding_part(&answers, parts).is_none()
}

/// Files shorter than this many lines read easily whole.
pub(super) const OUTLINE_NOTE_LINES: usize = 250;

/// A file-organization finding on a file of fewer than 250 lines is a note:
/// of 32 such findings labeled by hand on 25 projects, 3 were right, while
/// splitting a 138-line module or a 175-line test helper file would only
/// scatter it; 21 of 29 on longer files were right.
pub(super) fn short_outline(unit: &UnitPlan) -> bool {
    matches!(unit.detail, Detail::Outline { .. }) && unit.lines < OUTLINE_NOTE_LINES
}

/// A split of a Bend 2 file is at most a consider: a language that writes
/// each match arm, binding and effect on a line of its own runs to long
/// files, and on 64 Bend 2 projects 14 of 43 file-organization reviews were
/// right, 8 of 13 on the 41 its floor and sections were tuned on and 6 of
/// 30 on 23 it had never seen.
pub(super) fn bend_outline(unit: &UnitPlan) -> bool {
    matches!(unit.detail, Detail::Outline { .. })
        && unit
            .locations
            .first()
            .is_some_and(|l| crate::analysis::bend::file(&l.path))
}

/// Section rules that show a Bend 2 file laid out in titled parts.
pub(super) const SECTIONS: usize = 2;

/// A file-organization finding on a Bend 2 file its author laid out in
/// titled sections is a note: the groups proposed from its calls rarely
/// follow those sections, and on 41 Bend 2 projects 7 of 43 such findings
/// were right, against 10 of 17 on files without them.
pub(super) fn sectioned_outline(unit: &UnitPlan) -> bool {
    matches!(unit.detail, Detail::Outline { sections, .. } if sections >= SECTIONS)
}
