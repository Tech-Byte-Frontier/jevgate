//! Where a finding points and what it names: the block, value, constant,
//! group or part its locate answers chose.
use super::*;

/// The candidate part that raised an outline's finding: the outline's split
/// and kind raised none, and the part does a job of its own.
pub(super) fn deciding_part<'p>(
    answers: &Answers<'_>,
    parts: &'p [crate::units::Part],
) -> Option<&'p crate::units::Part> {
    let get = |q: &str| answers.get(q).copied();
    let split = organization_outcome(get("split"), get("kind"), &[]);
    if matches!(split, Some(Outcome::Review(_) | Outcome::Consider(_))) {
        return None;
    }
    // A part too long to ask is left out of `parts`, so find it by its ids.
    let (position, _) = separable_part(&part_answers(&get))?;
    let ids = crate::units::outline::PART_QUESTIONS[position];
    parts.iter().find(|part| part.questions == ids)
}

/// The group a module Choice picks clearly, or else the two it leans toward
/// when together they reach the location probability: flask's `cli.py`
/// split 0.45 and 0.23 over two of six groups. None when it spreads wider.
/// A group holding three quarters or more of the outline's `members` is
/// left out: moving 14 of a file's 15 members, or 9 of its 11 tests, moves
/// the file rather than splitting it.
pub(super) fn outline_groups<'g>(
    module: Option<&Answer>,
    groups: &'g [crate::units::GroupInfo],
    members: usize,
) -> Vec<&'g crate::units::GroupInfo> {
    let mut chosen = chosen_groups(module, groups);
    chosen.retain(|g| !three_quarters(g.names.len(), members));
    chosen
}

pub(super) fn chosen_groups<'g>(
    module: Option<&Answer>,
    groups: &'g [crate::units::GroupInfo],
) -> Vec<&'g crate::units::GroupInfo> {
    let find = |id: &str| groups.iter().find(|g| g.id == id);
    if let Some((id, _)) = choice(module) {
        return find(id).into_iter().collect();
    }
    let Some(Answer::Choice { probabilities, .. }) = module else {
        return Vec::new();
    };
    let mass: f64 = probabilities.values().sum::<f64>().max(f64::MIN_POSITIVE);
    let mut ranked: Vec<(&str, f64)> = probabilities
        .iter()
        .filter(|(id, _)| id.as_str() != "none")
        .map(|(id, p)| (id.as_str(), p / mass))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    match ranked.as_slice() {
        [first, second, ..]
            if crate::policy::probability_at_least(
                first.1 + second.1,
                crate::policy::LOCATION_PROBABILITY,
            ) =>
        {
            [first.0, second.0].into_iter().filter_map(find).collect()
        }
        _ => Vec::new(),
    }
}

/// A hardcoded-value finding's wording, naming the value or constant the
/// locate Choice named, and the location of that constant.
pub(super) fn values_finding(
    unit: &UnitPlan,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
    judgments: &[Judgment],
) -> (Wording, Option<Location>) {
    let lowered = lowered_value(unit, judgments);
    let (message, action) = values_wording(
        &unit.name,
        &unit.detail,
        (strength, lowered.map(|(reached, _)| reached)),
        p,
        answers,
    );
    let why = lowered
        .filter(|(_, why)| !why.is_empty())
        .map_or(String::new(), |(_, why)| format!(" {why}"));
    if let Some(index) = located_constant(unit, judgments) {
        // The finding points at the constant the Choice named.
        let location = unit.locations[index].clone();
        let constant = location.symbol.as_deref().unwrap_or("");
        let message = format!("{message} The constant is `{constant}`.{why}");
        return ((message, action), Some(location));
    }
    let wording = match located_value(unit, judgments) {
        Some(value) => (format!("{message} The value is {value}.{why}"), action),
        None => (format!("{message}{why}"), action),
    };
    (wording, None)
}

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
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let ((message, action), named) =
        privilege_wording(&format!("Job `{name}`"), strength, p, answers);
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

/// The position of the constant the locate Choice names, when it is clear.
pub(super) fn located_constant(unit: &UnitPlan, judgments: &[Judgment]) -> Option<usize> {
    if !matches!(unit.detail, Detail::Constants { .. }) {
        return None;
    }
    located_option(unit, judgments, ("constant", 'c')).filter(|&i| i < unit.locations.len())
}

/// The value a hardcoded-value finding is about, when the locate choice is clear.
pub(super) fn located_value(unit: &UnitPlan, judgments: &[Judgment]) -> Option<String> {
    let Detail::Values { choices, .. } = &unit.detail else {
        return None;
    };
    choices
        .get(located_option(unit, judgments, ("value", 'v'))?)
        .cloned()
}

/// The index of the option `{prefix}N` a clear locate Choice `question` names.
pub(super) fn located_option(
    unit: &UnitPlan,
    judgments: &[Judgment],
    (question, prefix): (&str, char),
) -> Option<usize> {
    let located = answers(judgments, &unit.id, Pass::Locate);
    let (id, _) = choice(located.get(question).copied())?;
    id.strip_prefix(prefix)?.parse().ok()
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
