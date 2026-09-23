//! Pure composition from typed judgments to unit outcomes, rule dimensions,
//! findings and a file status; each unit's outcome comes from `outcome`.
use super::{
    Access, Block, Detail, FilePlan, Presence, UnitPlan,
    outcome::{
        Answers, Outcome, benefit, checks, choice, lowered, noul, open, origin_outcome, score,
        split_has_users, unit_outcome, value_signals,
    },
    wording::{
        doc_pair_wording, document_wording, function_wording, handler_wording, module_wording,
        outline_wording, pair_wording, plan_wording, privilege_wording, question_label,
        section_wording, security_wording, stale_wording, test_pair_wording, test_wording,
        values_wording,
    },
};
use crate::{
    catalog,
    schema::{
        Answer, Dimension, Finding, Judgment, Pass, Status, Strength, Undecided, UnitCounts, hash,
    },
};
use std::collections::{BTreeMap, BTreeSet};

pub struct Composed {
    pub dimensions: BTreeMap<String, Dimension>,
    pub findings: Vec<Finding>,
    pub status: Status,
}

fn answers<'a>(judgments: &'a [Judgment], unit: &str, pass: Pass) -> Answers<'a> {
    judgments
        .iter()
        .filter(|j| j.unit == unit && j.pass == pass)
        .map(|j| (j.question.as_str(), &j.answer))
        .collect()
}

fn security(rule: &str) -> bool {
    catalog::SECURITY.contains(&rule)
}

/// A security unit's first-pass and trace answers, with each recheck answer
/// (the origin or a check, seen with callers) in place of the traced one,
/// unless the traced answer is decisive and the recheck is not: an undecided
/// traced answer is replaced even by an undecided recheck, whose lean saw
/// more evidence.
fn security_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> Answers<'a> {
    let mut merged = answers(judgments, &unit.id, Pass::First);
    merged.extend(answers(judgments, &unit.id, Pass::Trace));
    let judged = |question: &str, answer: &Answer| match question {
        "origin" => origin_outcome(answer),
        _ => noul(answer),
    };
    for (question, answer) in answers(judgments, &unit.id, Pass::Recheck) {
        let traced = merged.get(question).map(|a| judged(question, a));
        if judged(question, answer).decisive() || !traced.is_some_and(Outcome::decisive) {
            merged.insert(question, answer);
        }
    }
    merged
}

/// The first-pass outcome, replaced by a decisive recheck when the first pass
/// called for one.
fn resolved<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> (Outcome, Answers<'a>) {
    if security(unit.rule) {
        let merged = security_answers(unit, judgments);
        return (unit_outcome(unit, &merged), merged);
    }
    // Follow-ups whose questions sit beside the first answers under their own
    // ids: document section and pair checks, and benign-kind value checks.
    let beside = if [catalog::DOC_STALENESS, catalog::DOC_DUPLICATION].contains(&unit.rule) {
        Some(Pass::Trace)
    } else if unit.rule == catalog::HARDCODED_VALUES {
        Some(Pass::Recheck)
    } else {
        None
    };
    if let Some(pass) = beside {
        let mut merged = answers(judgments, &unit.id, Pass::First);
        merged.extend(answers(judgments, &unit.id, pass));
        return (unit_outcome(unit, &merged), merged);
    }
    // A test recheck asks the hollow-test questions again with the code under
    // test and the setup; each answer replaces the first one unless only the
    // first is decisive.
    if unit.rule == catalog::TEST_VALUE {
        let mut merged = answers(judgments, &unit.id, Pass::First);
        for (question, answer) in answers(judgments, &unit.id, Pass::Recheck) {
            let first = merged.get(question).map(|a| noul(a));
            if noul(answer).decisive() || !first.is_some_and(Outcome::decisive) {
                merged.insert(question, answer);
            }
        }
        return (unit_outcome(unit, &merged), merged);
    }
    let first = answers(judgments, &unit.id, Pass::First);
    let outcome = unit_outcome(unit, &first);
    let recheck = answers(judgments, &unit.id, Pass::Recheck);
    if open(unit, &first, outcome) && !recheck.is_empty() {
        let second = unit_outcome(unit, &recheck);
        if second.decisive() {
            return (second, recheck);
        }
    }
    (outcome, first)
}

/// Functions whose split question raised a review or consider, and whose
/// block has not been located yet; hardcoded-value functions raised to a
/// review or consider whose value has not been named yet.
pub fn unlocated_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| answers(judgments, &u.id, Pass::Locate).is_empty())
        .filter(|u| {
            let (outcome, resolved) = resolved(u, judgments);
            let raised =
                |o: Option<Outcome>| matches!(o, Some(Outcome::Review(_) | Outcome::Consider(_)));
            match &u.detail {
                Detail::Values {
                    locate: Some(_), ..
                } => raised(Some(outcome)),
                Detail::Function {
                    locate: Some(_), ..
                }
                | Detail::Document {
                    locate: Some(_), ..
                } => raised(resolved.get("split").map(|a| benefit(a))),
                _ => false,
            }
        })
        .map(|u| u.id.clone())
        .collect()
}

/// Judged units whose first pass stayed undecided, or became a note from a
/// torn function or file-organization answer, and that have no recheck yet;
/// for injection, units whose traced origin stayed unclear or was the
/// function's parameters, so callers can settle it.
pub fn uncertain_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| answers(judgments, &u.id, Pass::Recheck).is_empty())
        .filter(|u| {
            if security(u.rule) {
                let merged = security_answers(u, judgments);
                let get = |q: &str| merged.get(q).copied();
                u.rule == catalog::INJECTION
                    && !checks(u.rule, &get).iter().all(|o| *o == Outcome::Clear)
                    && matches!(
                        merged.get("origin").map(|a| origin_outcome(a)),
                        Some(Outcome::Uncertain(_) | Outcome::Consider(_))
                    )
            } else if u.rule == catalog::HARDCODED_VALUES {
                let first = answers(judgments, &u.id, Pass::First);
                let get = |q: &str| first.get(q).copied();
                value_signals(&get, &u.detail, false).is_some_and(|signals| {
                    signals
                        .iter()
                        .any(|(_, o, _)| matches!(o, Outcome::Uncertain(_)))
                })
            } else {
                let first = answers(judgments, &u.id, Pass::First);
                open(u, &first, unit_outcome(u, &first))
            }
        })
        .map(|u| u.id.clone())
        .collect()
}

/// Documents whose plan question found a plan whose work Git shows finished.
pub fn finished_plans(
    plan: &super::Plan,
    files: &[crate::schema::FileResult],
) -> BTreeSet<std::path::PathBuf> {
    plan.files
        .iter()
        .filter(|(owner, file_plan)| {
            file_plan.units.iter().any(|u| {
                matches!(u.detail, Detail::Plan { .. })
                    && matches!(
                        resolved(u, &files[**owner].judgments).0,
                        Outcome::Consider(_) | Outcome::Review(_)
                    )
            })
        })
        .map(|(_, file_plan)| file_plan.path.clone())
        .collect()
}

/// Security units whose presence is not clear, to trace.
pub fn untraced_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged && security(u.rule))
        .filter(|u| answers(judgments, &u.id, Pass::Trace).is_empty())
        .filter(|u| {
            let first = answers(judgments, &u.id, Pass::First);
            let presence: Vec<Outcome> = super::security::presence_questions(u.rule)
                .iter()
                .filter_map(|q| first.get(q).map(|a| noul(a)))
                .collect();
            if presence.is_empty() {
                return false;
            }
            presence.iter().any(|o| *o != Outcome::Clear)
        })
        .map(|u| u.id.clone())
        .collect()
}

pub fn compose(plan: &FilePlan, judgments: &[Judgment]) -> Composed {
    let mut counts = BTreeMap::<&str, UnitCounts>::new();
    let mut concern = BTreeMap::<&str, f64>::new();
    let mut findings = Vec::new();
    let mut redundant = Vec::new();
    let mut undecided = BTreeMap::<&str, Vec<Undecided>>::new();
    for (rule, omitted) in &plan.rules {
        counts.entry(rule).or_default().omitted = *omitted;
    }
    for unit in &plan.units {
        let count = counts.entry(unit.rule).or_default();
        if !counted_as_judged(unit, judgments, count) {
            continue;
        }
        let (outcome, answers) = resolved(unit, judgments);
        let outcome = if unnamed_value(unit, judgments) {
            lowered(outcome)
        } else {
            outcome
        };
        let top = concern.entry(unit.rule).or_default();
        *top = top.max(outcome.concern());
        match strength_of(outcome) {
            Some((strength, p)) => {
                *match strength {
                    Strength::Review => &mut count.review,
                    Strength::Consider => &mut count.consider,
                    Strength::Note => &mut count.note,
                } += 1;
                findings.push(finding(plan, unit, strength, p, &answers, judgments));
            }
            None if outcome == Outcome::Clear => count.clear += 1,
            None => {
                count.uncertain += 1;
                undecided
                    .entry(unit.rule)
                    .or_default()
                    .push(undecided_unit(unit, &answers));
            }
        }
        if let (Detail::TestPair { names, subject }, Outcome::Review(p) | Outcome::Consider(p)) =
            (&unit.detail, outcome)
        {
            redundant.push((unit, names, subject, p));
        }
    }
    findings.extend(over_tested(plan, &redundant));
    let dimensions = plan
        .rules
        .keys()
        .map(|rule| {
            let count = counts.remove(rule).unwrap_or_default();
            let concern = concern.get(rule).copied().unwrap_or(0.0);
            let undecided = undecided.remove(rule).unwrap_or_default();
            (rule.to_string(), dimension(rule, count, concern, undecided))
        })
        .collect();
    // Strongest first, so a note never sits above a review or consider.
    findings.sort_by(|a, b| b.strength.cmp(&a.strength).then(b.rank.total_cmp(&a.rank)));
    let status = file_status(&dimensions, &findings);
    Composed {
        dimensions,
        findings,
        status,
    }
}

/// Counts a unit that is too small, needs context or was left unasked under
/// a finished plan, and returns false for it; otherwise counts it as judged.
fn counted_as_judged(unit: &UnitPlan, judgments: &[Judgment], count: &mut UnitCounts) -> bool {
    match unit.presence {
        Presence::TooSmall => count.too_small += 1,
        Presence::NeedsContext => count.needs_context += 1,
        // A check left unasked because its document is a finished plan.
        Presence::Judged
            if matches!(unit.detail, Detail::Stale { .. } | Detail::DocPair { .. })
                && answers(judgments, &unit.id, Pass::Trace).is_empty() =>
        {
            count.covered += 1
        }
        Presence::Judged => {
            count.judged += 1;
            return true;
        }
    }
    false
}

/// The finding strength and probability of an outcome that raises one.
fn strength_of(outcome: Outcome) -> Option<(Strength, f64)> {
    match outcome {
        Outcome::Review(p) => Some((Strength::Review, p)),
        Outcome::Consider(p) => Some((Strength::Consider, p)),
        Outcome::Note(p) => Some((Strength::Note, p)),
        _ => None,
    }
}

/// Questions whose undecided answer leaves a unit of the rule undecided; the
/// other questions are weak signals or only matter when decisive.
fn deciding_questions(rule: &str) -> &'static [&'static str] {
    match rule {
        catalog::FUNCTION_SIMPLIFICATION => &["split", "flatten"],
        catalog::FILE_ORGANIZATION => &["split"],
        catalog::SHARED_LOGIC => &["same"],
        catalog::HARDCODED_VALUES => &["environment", "magic", "special"],
        catalog::TEST_VALUE => &["own_logic", "mock_only"],
        catalog::INJECTION => &[
            "interpreted",
            "resource",
            "origin",
            "sql",
            "shell",
            "code",
            "markup",
            "path",
            "url",
        ],
        catalog::SENSITIVE_DATA => &[
            "logs_secret",
            "error_details",
            "logs_object_secret",
            "exception_to_client",
            "handler_leaks",
        ],
        catalog::UNSAFE_SETTINGS => &["weakened", "tls", "hash", "random", "cors", "cookie"],
        catalog::ACCESS_CONTROL => &[
            "others",
            "editable",
            "search_path",
            "unchecked",
            "broad",
            "data",
            "exposed",
            "rows",
            "returns_others",
            "reach",
            "argument_rows",
            "operator_only",
        ],
        catalog::WORKFLOWS => &["outside", "untrusted"],
        catalog::LARGE_DOCS => &["split", "history"],
        catalog::DOC_STALENESS => &["plan", "relies"],
        catalog::DOC_DUPLICATION => &["a_covers", "b_covers", "conflict"],
        catalog::AGENT_CONTEXT => &[
            "inferable",
            "describes",
            "commands",
            "generic",
            "history",
            "enforced",
        ],
        _ => &["overlap"],
    }
}

/// Candidate values are listed with an undecided unit only when this few,
/// so the entry names what the question was about without repeating the code.
const SHOWN_VALUES: usize = 3;

/// The unit and its undecided questions; with no answers, why.
fn undecided_unit(unit: &UnitPlan, answers: &Answers<'_>) -> Undecided {
    let get = |q: &str| answers.get(q).copied();
    let settled_values = (unit.rule == catalog::HARDCODED_VALUES)
        .then(|| value_signals(&get, &unit.detail, true))
        .flatten();
    let mut questions: Vec<String> = match settled_values {
        Some(signals) => signals
            .iter()
            .filter(|(_, o, _)| matches!(o, Outcome::Uncertain(_)))
            .map(|(q, ..)| question_label(q).to_string())
            .collect(),
        None => undecided_questions(unit.rule, answers),
    };
    if answers.is_empty() {
        questions.push("no answer".into());
    }
    let values = match &unit.detail {
        Detail::Values { values, .. } | Detail::Constants { values }
            if values.len() <= SHOWN_VALUES =>
        {
            values.clone()
        }
        _ => Vec::new(),
    };
    Undecided {
        unit: match unit.detail {
            Detail::Outline { .. } => "file outline".into(),
            _ => unit.name.clone(),
        },
        values,
        line: unit.locations.first().map_or(1, |l| l.start_line),
        questions,
    }
}

/// The deciding questions of a rule whose own answers stayed undecided.
fn undecided_questions(rule: &str, answers: &Answers<'_>) -> Vec<String> {
    deciding_questions(rule)
        .iter()
        .filter(|q| {
            answers.get(*q).is_some_and(|a| {
                let outcome = match a {
                    Answer::Noul { .. } => noul(a),
                    _ if **q == "origin" => origin_outcome(a),
                    _ if ["data", "rows", "reach"].contains(q) => {
                        super::outcome::acceptable_levels(a)
                    }
                    _ => score(a),
                };
                matches!(outcome, Outcome::Uncertain(_))
            })
        })
        .map(|q| question_label(q).to_string())
        .collect()
}

/// A rule's status is its most severe unit outcome.
fn dimension(rule: &str, count: UnitCounts, concern: f64, undecided: Vec<Undecided>) -> Dimension {
    Dimension {
        decision_basis: basis(rule, &count),
        status: counted_status(&count),
        concern_probability: concern,
        rule_version: catalog::rule_version(rule).into(),
        units: count,
        undecided,
    }
}

pub(super) fn counted_status(count: &UnitCounts) -> Status {
    if count.review > 0 {
        Status::Review
    } else if count.consider > 0 {
        Status::Consider
    } else if count.needs_context > 0 {
        Status::NeedsContext
    } else if count.uncertain > 0 {
        Status::Uncertain
    } else if count.note > 0 {
        Status::Note
    } else if count.clear > 0 {
        Status::Clear
    } else {
        Status::NotApplicable
    }
}

pub(super) fn file_status(
    dimensions: &BTreeMap<String, Dimension>,
    findings: &[Finding],
) -> Status {
    let any = |status: Status| dimensions.values().any(|d| d.status == status);
    if dimensions
        .values()
        .all(|d| d.status == Status::NotApplicable)
    {
        Status::NotApplicable
    } else if findings.iter().any(|f| f.strength == Strength::Review) {
        Status::Review
    } else if findings.iter().any(|f| f.strength == Strength::Consider) {
        Status::Consider
    } else if any(Status::NeedsContext) {
        Status::NeedsContext
    } else if any(Status::Uncertain) {
        Status::Uncertain
    } else if !findings.is_empty() {
        Status::Note
    } else {
        Status::Clear
    }
}

fn basis(rule: &str, count: &UnitCounts) -> String {
    let noun = match rule {
        catalog::FILE_ORGANIZATION => "outline",
        catalog::FUNCTION_SIMPLIFICATION => "function",
        catalog::SHARED_LOGIC => "candidate pair",
        catalog::TEST_VALUE => "test",
        catalog::HARDCODED_VALUES => "value unit",
        catalog::INJECTION | catalog::SENSITIVE_DATA | catalog::UNSAFE_SETTINGS => "security unit",
        catalog::ACCESS_CONTROL => "access statement",
        catalog::WORKFLOWS => "workflow job",
        catalog::AGENT_CONTEXT => "section",
        catalog::LARGE_DOCS => "document",
        catalog::DOC_STALENESS => "document check",
        catalog::DOC_DUPLICATION => "section pair",
        _ => "test pair",
    };
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut parts = Vec::new();
    if count.judged == 0 && count.needs_context == 0 {
        parts.push(format!("No {noun}s to judge."));
    } else {
        let mut outcomes = Vec::new();
        for (n, label) in [
            (count.review, "review"),
            (count.consider, "consider"),
            (count.note, "note"),
            (count.clear, "clear"),
            (count.uncertain, "uncertain"),
        ] {
            if n > 0 {
                outcomes.push(format!("{n} {label}"));
            }
        }
        parts.push(format!(
            "{} {noun}{} judged{}.",
            count.judged,
            plural(count.judged),
            if outcomes.is_empty() {
                String::new()
            } else {
                format!(": {}", outcomes.join(", "))
            }
        ));
    }
    if count.needs_context > 0 {
        parts.push(format!(
            "{} {noun}{} exceed the request limit and were not sent.",
            count.needs_context,
            plural(count.needs_context)
        ));
    }
    if count.too_small > 0 {
        parts.push(format!(
            "{} {noun}{} too small to judge.",
            count.too_small,
            plural(count.too_small)
        ));
    }
    if count.covered > 0 {
        parts.push(format!(
            "{} check{} inside finished plans not asked.",
            count.covered,
            plural(count.covered)
        ));
    }
    if count.omitted > 0 {
        parts.push(format!(
            "{} candidate{} omitted by caps.",
            count.omitted,
            plural(count.omitted)
        ));
    }
    parts.join(" ")
}

fn fingerprint(rule: &str, plan: &FilePlan, identity: &str) -> String {
    let separator = crate::schema::HASH_SEPARATOR;
    hash(
        format!(
            "{rule}{separator}{}{separator}{identity}",
            plan.path.display()
        )
        .as_bytes(),
    )
}

fn rank(probability: f64, lines: usize) -> f64 {
    probability * (1.0 + lines as f64).ln()
}

fn finding(
    plan: &FilePlan,
    unit: &UnitPlan,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
    judgments: &[Judgment],
) -> Finding {
    let name = &unit.name;
    let mut locations = unit.locations.clone();
    let mut symbol = Some(name.clone());
    let mut block = None;
    let mut category = None;
    let (message, action) = match &unit.detail {
        Detail::Function { blocks, .. } => {
            block = located_block(unit, blocks, judgments, "block");
            function_wording(name, strength, p, answers, block)
        }
        Detail::Outline { groups } => {
            let module = answers.get("module").copied();
            let chosen = choice(module).and_then(|(id, _)| groups.iter().find(|g| g.id == id));
            symbol = chosen.map(|group| group.id.clone());
            if let Some(group) = chosen {
                locations = group.locations.clone();
            }
            let own_users = split_has_users(groups, module);
            outline_wording(chosen, strength, own_users, p)
        }
        Detail::Pair {
            differences,
            within_test,
            in_tests,
            in_cases,
        } => pair_wording(
            name,
            differences,
            (*within_test, *in_tests, *in_cases),
            strength,
            p,
        ),
        Detail::Values { .. } | Detail::Constants { .. } => {
            let unnamed = unnamed_value(unit, judgments);
            let (message, action) =
                values_wording(name, &unit.detail, (strength, unnamed), p, answers);
            match located_value(unit, judgments) {
                Some(value) => (format!("{message} The value is {value}."), action),
                None => (message, action),
            }
        }
        Detail::Security {
            sites, messages, ..
        } => {
            let site = choice(answers.get("site").copied())
                .and_then(|(id, _)| sites.iter().find(|s| s.id == id));
            block = site;
            let ((message, action), named) =
                security_wording(unit.rule, name, strength, p, answers);
            // The error message the Choice found carrying another error's text.
            let carried = choice(answers.get("messages").copied())
                .and_then(|(id, _)| messages.get(id.strip_prefix('m')?.parse::<usize>().ok()?))
                .filter(|_| named.starts_with("CWE-209") && strength != Strength::Note);
            category = Some(named);
            match carried {
                Some(text) => (format!("{message} The message is {text}."), action),
                None => (message, action),
            }
        }
        Detail::Section { .. } => section_wording(name, &unit.detail, strength, p, answers),
        Detail::Plan { facts } => {
            symbol = None;
            category = Some(super::grouping::FINISHED_PLAN.into());
            plan_wording(name, facts, p)
        }
        Detail::Stale { missing, .. } => stale_wording(name, missing, p),
        Detail::DocPair { other, .. } => {
            let (wording, conflict) = doc_pair_wording(name, other, answers, p);
            if conflict {
                category = Some("conflict".into());
            }
            wording
        }
        Detail::Document { parts, .. } => {
            symbol = None;
            block = located_block(unit, parts, judgments, "part");
            document_wording(name, strength, p, answers, block)
        }
        Detail::Handler { registered } => {
            category = Some("CWE-209 error details exposed".into());
            handler_wording(name, registered, strength, p)
        }
        Detail::Access(access @ (Access::Table | Access::View | Access::Reducer)) => {
            let (wording, named) = module_wording(access, name, strength, p, answers);
            category = Some(named);
            wording
        }
        Detail::Access(access) => {
            let subject = match access {
                Access::Policy { table } => format!("Policy `{name}` on `{table}`"),
                Access::Definer => format!("SECURITY DEFINER function `{name}`"),
                _ => format!("A grant on `{name}`"),
            };
            let (wording, named) = privilege_wording(&subject, strength, p, answers);
            category = Some(named);
            wording
        }
        Detail::Job { expressions } => {
            let ((message, action), named) =
                privilege_wording(&format!("Job `{name}`"), strength, p, answers);
            category = Some(named);
            let outside = matches!(
                answers.get("outside").map(|a| noul(a)),
                Some(Outcome::Review(_))
            );
            let listed = expressions
                .iter()
                .map(|e| format!("`${{{{ {e} }}}}`"))
                .collect::<Vec<_>>()
                .join(", ");
            if outside {
                (
                    format!("{message} Expressions in its scripts: {listed}."),
                    action,
                )
            } else {
                (message, action)
            }
        }
        Detail::Test => test_wording(name, strength, p, answers),
        Detail::TestPair { .. } => {
            symbol = None;
            test_pair_wording(name, strength == Strength::Review, p)
        }
    };
    let lines = locations
        .iter()
        .map(|l| l.end_line + 1 - l.start_line)
        .sum::<usize>()
        .max(unit.lines.min(1));
    // The located block comes first, so an agent acts on it; the function follows.
    if let Some(block) = block {
        locations.insert(0, block.location.clone());
    }
    Finding {
        rule: catalog::id(unit.rule).into(),
        strength,
        line: locations.first().map_or(1, |l| l.start_line),
        message,
        action: action.into(),
        symbol,
        rule_version: catalog::rule_version(unit.rule).into(),
        concern_probability: p,
        locations,
        quote: unit.quote.clone(),
        category,
        // Only a special-cased identity groups across files: the same number
        // or path can mean different things in different code.
        values: located_value(unit, judgments)
            .filter(|_| {
                matches!(
                    answers.get("special").map(|a| noul(a)),
                    Some(Outcome::Review(_) | Outcome::Consider(_))
                )
            })
            .into_iter()
            .collect(),
        fingerprint: fingerprint(unit.rule, plan, &unit.identity),
        rank: rank(p, lines),
        baselined: false,
    }
}

/// A function's hardcoded-value review or consider whose value was not
/// named: the locate Choice picked none clearly, or there were too many
/// values to offer. Its finding is one level lower, since a reader cannot
/// tell what to change.
fn unnamed_value(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    matches!(unit.detail, Detail::Values { .. })
        && located_value(unit, judgments).is_none()
        && matches!(
            resolved(unit, judgments).0,
            Outcome::Review(_) | Outcome::Consider(_)
        )
}

/// The value a hardcoded-value finding is about, when the locate choice is clear.
fn located_value(unit: &UnitPlan, judgments: &[Judgment]) -> Option<String> {
    let Detail::Values { choices, .. } = &unit.detail else {
        return None;
    };
    let located = answers(judgments, &unit.id, Pass::Locate);
    let (id, _) = choice(located.get("value").copied())?;
    let index: usize = id.strip_prefix('v')?.parse().ok()?;
    choices.get(index).cloned()
}

/// The block chosen by the locate follow-up `question`, when its choice is clear.
fn located_block<'a>(
    unit: &UnitPlan,
    blocks: &'a [Block],
    judgments: &[Judgment],
    question: &str,
) -> Option<&'a Block> {
    let located = answers(judgments, &unit.id, Pass::Locate);
    let (id, _) = choice(located.get(question).copied())?;
    blocks.iter().find(|b| b.id == id)
}

/// Three or more tests linked by overlapping pairs on one subject.
fn over_tested(
    plan: &FilePlan,
    redundant: &[(&UnitPlan, &[String; 2], &String, f64)],
) -> Vec<Finding> {
    let mut clusters =
        BTreeMap::<&String, (BTreeSet<&String>, Vec<crate::schema::Location>, f64)>::new();
    for (unit, names, subject, p) in redundant {
        let cluster = clusters
            .entry(subject)
            .or_insert_with(|| (BTreeSet::new(), Vec::new(), 1.0));
        for (name, location) in names.iter().zip(&unit.locations) {
            if cluster.0.insert(name) {
                cluster.1.push(location.clone());
            }
        }
        cluster.2 = cluster.2.min(*p);
    }
    clusters
        .into_iter()
        .filter(|(_, (tests, ..))| tests.len() >= 3)
        .map(|(subject, (tests, mut locations, p))| {
            locations.sort();
            let names: Vec<String> = tests.iter().map(|t| format!("`{t}`")).collect();
            let lines = locations
                .iter()
                .map(|l| l.end_line + 1 - l.start_line)
                .sum();
            let identity: Vec<&str> = std::iter::once(subject.as_str())
                .chain(tests.iter().map(|t| t.as_str()))
                .collect();
            Finding {
                rule: catalog::id(catalog::TEST_REDUNDANCY).into(),
                strength: Strength::Consider,
                line: locations.first().map_or(1, |l| l.start_line),
                message: format!(
                    "{} tests of `{subject}` overlap: {} ({p:.2}).",
                    tests.len(),
                    names.join(", ")
                ),
                action: "Consider one parameterized test for these cases".into(),
                symbol: Some(subject.clone()),
                rule_version: catalog::rule_version(catalog::TEST_REDUNDANCY).into(),
                concern_probability: p,
                locations,
                quote: None,
                category: None,
                values: Vec::new(),
                fingerprint: fingerprint(
                    catalog::TEST_REDUNDANCY,
                    plan,
                    &super::identity(&identity),
                ),
                rank: rank(p, lines),
                baselined: false,
            }
        })
        .collect()
}
