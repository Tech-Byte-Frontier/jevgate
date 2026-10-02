//! Where a finding points and what it names: the block or site its locate
//! answers chose.
use super::*;

/// A security finding's wording, the site the Choice named and its
/// category; error details quote the message that carries another error's
/// text.
pub(super) fn security_finding<'a>(
    unit: &UnitPlan,
    (sites, messages): (&'a [Block], &[String]),
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, Option<&'a Block>, String) {
    let site =
        choice(answers.get("site").copied()).and_then(|(id, _)| sites.iter().find(|s| s.id == id));
    let ((message, action), named) = security_wording(unit.rule, &unit.name, strength, p, answers);
    // The error message the Choice found carrying another error's text.
    let carried = choice(answers.get("messages").copied())
        .and_then(|(id, _)| messages.get(id.strip_prefix('m')?.parse::<usize>().ok()?))
        .filter(|_| named.starts_with("CWE-209") && strength != Strength::Note);
    let wording = match carried {
        Some(text) => (format!("{message} The message is {text}."), action),
        None => (message, action),
    };
    (wording, site, named)
}

/// A workflow job's wording and category, listing the expressions of its
/// scripts when outsiders can write them.
pub(super) fn job_wording(
    name: &str,
    expressions: &[String],
    strength: Strength,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let ((message, action), named) = privilege_wording(&format!("Job `{name}`"), strength, answers);
    let outside = matches!(
        answers.get("outside").map(|a| noul(a)),
        Some(Outcome::Review(_))
    );
    if !outside {
        return ((message, action), named);
    }
    let listed = expressions
        .iter()
        .map(|e| format!("`${{{{ {e} }}}}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let message = format!("{message} Expressions in its scripts: {listed}.");
    ((message, action), named)
}

/// Whether a block spans most of its function, three quarters or more:
/// naming it as the part to extract says no more than the finding does, as
/// lines 206–390 of just's 198-line `Justfile::run` did.
pub(super) fn most_of(
    block: &crate::schema::Location,
    function: &[crate::schema::Location],
) -> bool {
    let lines = |l: &crate::schema::Location| l.end_line + 1 - l.start_line;
    function
        .first()
        .is_some_and(|f| three_quarters(lines(block), lines(f)))
}

/// Whether `part` is three quarters or more of `whole`.
pub(super) fn three_quarters(part: usize, whole: usize) -> bool {
    part * 4 >= whole * 3
}

/// The block chosen by the locate follow-up `question`, when its choice is clear.
pub(super) fn located_block<'a>(
    unit: &UnitPlan,
    blocks: &'a [Block],
    judgments: &[Judgment],
    question: &str,
) -> Option<&'a Block> {
    let located = answers(judgments, &unit.id, Pass::Locate);
    let (id, _) = choice(located.get(question).copied())?;
    blocks.iter().find(|b| b.id == id)
}
