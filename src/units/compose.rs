//! Pure composition from typed judgments to unit outcomes, rule dimensions,
//! findings and a file status; each unit's outcome comes from `outcome`.
use super::{
    Block, Detail, FilePlan, Presence, UnitPlan,
    outcome::{
        Answers, Outcome, benefit, checks, choice, noul, origin_outcome, score, split_has_users,
        unit_outcome, value_signals,
    },
    wording::{
        function_wording, outline_wording, pair_wording, question_label, section_wording,
        security_wording, test_pair_wording, test_wording, values_wording,
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

/// A security unit's first-pass and trace answers, with each decisive
/// recheck answer (the origin or a check, seen with callers) in place of the
/// traced one.
fn security_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> Answers<'a> {
    let mut merged = answers(judgments, &unit.id, Pass::First);
    merged.extend(answers(judgments, &unit.id, Pass::Trace));
    for (question, answer) in answers(judgments, &unit.id, Pass::Recheck) {
        let outcome = match question {
            "origin" => origin_outcome(answer),
            _ => noul(answer),
        };
        if outcome.decisive() {
            merged.insert(question, answer);
        }
    }
    merged
}

/// The first-pass outcome, replaced by a decisive recheck when one exists.
fn resolved<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> (Outcome, Answers<'a>) {
    if security(unit.rule) {
        let merged = security_answers(unit, judgments);
        return (unit_outcome(unit, &merged), merged);
    }
    if unit.rule == catalog::HARDCODED_VALUES {
        // Benign-kind checks have their own ids beside the first answers.
        let mut merged = answers(judgments, &unit.id, Pass::First);
        merged.extend(answers(judgments, &unit.id, Pass::Recheck));
        return (unit_outcome(unit, &merged), merged);
    }
    let first = answers(judgments, &unit.id, Pass::First);
    let outcome = unit_outcome(unit, &first);
    let recheck = answers(judgments, &unit.id, Pass::Recheck);
    if !outcome.decisive() && !recheck.is_empty() {
        let second = unit_outcome(unit, &recheck);
        if second.decisive() {
            return (second, recheck);
        }
    }
    (outcome, first)
}

/// Functions whose split question raised a review or consider, and whose
/// block has not been located yet.
pub fn unlocated_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| {
            matches!(
                &u.detail,
                Detail::Function {
                    locate: Some(_),
                    ..
                }
            )
        })
        .filter(|u| {
            let (_, resolved) = resolved(u, judgments);
            matches!(
                resolved.get("split").map(|a| benefit(a)),
                Some(Outcome::Review(_) | Outcome::Consider(_))
            ) && answers(judgments, &u.id, Pass::Locate).is_empty()
        })
        .map(|u| u.id.clone())
        .collect()
}

/// Judged units whose first pass stayed undecided and that have no recheck
/// yet; for injection, units whose traced origin stayed unclear or was the
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
                matches!(
                    unit_outcome(u, &answers(judgments, &u.id, Pass::First)),
                    Outcome::Uncertain(_)
                )
            }
        })
        .map(|u| u.id.clone())
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
        match unit.presence {
            Presence::TooSmall => {
                count.too_small += 1;
                continue;
            }
            Presence::NeedsContext => {
                count.needs_context += 1;
                continue;
            }
            Presence::Judged => count.judged += 1,
        }
        let (outcome, answers) = resolved(unit, judgments);
        let top = concern.entry(unit.rule).or_default();
        *top = top.max(outcome.concern());
        match outcome {
            Outcome::Review(p) => {
                count.review += 1;
                findings.push(finding(
                    plan,
                    unit,
                    Strength::Review,
                    p,
                    &answers,
                    judgments,
                ));
            }
            Outcome::Consider(p) => {
                count.consider += 1;
                findings.push(finding(
                    plan,
                    unit,
                    Strength::Consider,
                    p,
                    &answers,
                    judgments,
                ));
            }
            Outcome::Note(p) => {
                count.note += 1;
                findings.push(finding(plan, unit, Strength::Note, p, &answers, judgments));
            }
            Outcome::Clear => count.clear += 1,
            Outcome::Uncertain(_) | Outcome::Missing => {
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
        ],
        catalog::UNSAFE_SETTINGS => &["weakened", "tls", "hash", "random", "cors", "cookie"],
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
        Detail::Values { values } | Detail::Constants { values }
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
    let status = if count.review > 0 {
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
    };
    Dimension {
        decision_basis: basis(rule, &count),
        status,
        concern_probability: concern,
        rule_version: catalog::rule_version(rule).into(),
        units: count,
        undecided,
    }
}

fn file_status(dimensions: &BTreeMap<String, Dimension>, findings: &[Finding]) -> Status {
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
        catalog::AGENT_CONTEXT => "section",
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
            block = located_block(unit, blocks, judgments);
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
            values_wording(name, &unit.detail, strength, p, answers)
        }
        Detail::Security { sites, .. } => {
            let site = choice(answers.get("site").copied())
                .and_then(|(id, _)| sites.iter().find(|s| s.id == id));
            block = site;
            let (wording, named) = security_wording(unit.rule, name, strength, p, answers);
            category = Some(named);
            wording
        }
        Detail::Section { .. } => section_wording(name, &unit.detail, strength, p, answers),
        Detail::Test => test_wording(name, strength == Strength::Review, p, answers),
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
        fingerprint: fingerprint(unit.rule, plan, &unit.identity),
        rank: rank(p, lines),
        baselined: false,
    }
}

/// The block chosen by the locate follow-up, when its choice is clear.
fn located_block<'a>(
    unit: &UnitPlan,
    blocks: &'a [Block],
    judgments: &[Judgment],
) -> Option<&'a Block> {
    let located = answers(judgments, &unit.id, Pass::Locate);
    let (id, _) = choice(located.get("block").copied())?;
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
