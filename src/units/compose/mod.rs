//! Pure composition from typed judgments to unit outcomes, rule dimensions,
//! findings and a file status; each unit's outcome comes from `outcome`.
//! `answers` gathers what an outcome rests on, `due` picks the units each
//! follow-up stage asks next, `caps` holds the measured limits on a
//! finding's level, `located` where a finding points, and `redundant` and
//! `comments` report tests and comments together.
use super::{
    Access, Block, Detail, FilePlan, Presence, UnitPlan,
    outcome::{
        Answers, Outcome, at_most_note, benefit, checks, choice, choice_mass, confirmable,
        document_split, logs_found, lowered, noul, open, organization_outcome, origin_outcome,
        part_answers, score, separable_part, settled_checks, several_kind, unit_outcome,
        value_signals,
    },
    wording::{Wording, comment_reason, comment_wording},
    wording::{
        doc_pair_wording, document_wording, function_wording, handler_wording, law_wording,
        module_wording, outline_wording, pair_wording, part_wording, plan_wording,
        privilege_wording, question_label, section_wording, security_wording, stale_wording,
        test_pair_wording, test_wording, values_wording,
    },
};
use crate::{
    catalog,
    schema::{
        Answer, Dimension, Finding, Judgment, Location, Pass, Status, Strength, Undecided,
        UnitCounts, hash,
    },
};
use std::collections::{BTreeMap, BTreeSet};

mod answers;
mod caps;
mod comments;
mod due;
mod located;
mod redundant;
use answers::*;
use caps::*;
use comments::*;
pub use due::{
    finished_plans, uncertain_units, unconfirmed_units, unkinded_units, unkinded_values,
    unlocated_units, unparted_units, unsettled, untraced_units,
};
use located::*;
use redundant::*;

pub struct Composed {
    pub dimensions: BTreeMap<String, Dimension>,
    pub findings: Vec<Finding>,
    pub status: Status,
}

pub fn compose(plan: &FilePlan, judgments: &[Judgment]) -> Composed {
    let few = few_comment_lines(plan, judgments);
    let mut tally = Tally::default();
    for (rule, omitted) in &plan.rules {
        tally.counts.entry(rule).or_default().omitted = *omitted;
    }
    for unit in &plan.units {
        tally.add(plan, unit, judgments, &few);
    }
    let Tally {
        mut counts,
        concern,
        mut findings,
        redundant,
        commented,
        mut undecided,
    } = tally;
    // A pair of tests in a group of three or more is reported by the group.
    let (groups, grouped) = over_tested(plan, &redundant);
    // A pair in a group reaches the group's consider; a lone pair is a note.
    if let Some(count) = counts.get_mut(catalog::TEST_REDUNDANCY) {
        count.note -= grouped.len();
        count.consider += grouped.len();
    }
    let mut index = 0;
    findings.retain(|_| {
        index += 1;
        !grouped.contains(&(index - 1))
    });
    findings.extend(groups);
    drop_copies_of_redundant_tests(&mut findings);
    findings.extend(comment_findings(plan, &commented));
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

/// What a file's units add up to, per rule, before comment findings and
/// redundant tests are grouped.
#[derive(Default)]
struct Tally<'p> {
    counts: BTreeMap<&'p str, UnitCounts>,
    concern: BTreeMap<&'p str, f64>,
    findings: Vec<Finding>,
    redundant: Vec<Redundant<'p>>,
    commented: Vec<(&'p UnitPlan, Strength, f64, &'static str)>,
    undecided: BTreeMap<&'p str, Vec<Undecided>>,
}

impl<'p> Tally<'p> {
    /// Counts one unit under its rule and keeps what it contributes: a
    /// finding, a comment to group, a redundant test pair or an undecided unit.
    fn add(
        &mut self,
        plan: &FilePlan,
        unit: &'p UnitPlan,
        judgments: &[Judgment],
        few: &BTreeSet<&str>,
    ) {
        let count = self.counts.entry(unit.rule).or_default();
        if !counted_as_judged(unit, judgments, count) {
            return;
        }
        let (outcome, answers) = resolved(unit, judgments);
        let outcome = capped(unit, judgments, few, outcome);
        // Two tests that check one behavior with different inputs are a note
        // on their own; three or more linked by such pairs are grouped into a
        // consider below. Labeled by hand on just, express, gson and
        // lobsters, lone pairs were mostly style, with few worth merging.
        let grouping = outcome;
        let outcome = match (&unit.detail, outcome) {
            (Detail::TestPair { .. }, Outcome::Consider(p)) => Outcome::Note(p),
            _ => outcome,
        };
        let top = self.concern.entry(unit.rule).or_default();
        *top = top.max(outcome.concern());
        match strength_of(outcome) {
            Some((strength, p)) => {
                *match strength {
                    Strength::Review => &mut count.review,
                    Strength::Consider => &mut count.consider,
                    Strength::Note => &mut count.note,
                } += 1;
                if unit.rule == catalog::COMMENTS {
                    self.commented.push((
                        unit,
                        strength,
                        p,
                        comment_reason(&answers, documented(unit)),
                    ));
                } else {
                    self.findings
                        .push(finding(plan, unit, strength, p, &answers, judgments));
                }
            }
            None if outcome == Outcome::Clear => count.clear += 1,
            None => {
                count.uncertain += 1;
                self.undecided
                    .entry(unit.rule)
                    .or_default()
                    .push(undecided_unit(unit, &answers));
            }
        }
        if let (
            Detail::TestPair { names, subject, .. },
            Outcome::Review(p) | Outcome::Consider(p),
        ) = (&unit.detail, grouping)
        {
            // A review pair stays its own finding: it says a test adds nothing.
            let finding = matches!(grouping, Outcome::Consider(_)).then(|| self.findings.len() - 1);
            self.redundant.push(Redundant {
                unit,
                names,
                subject,
                p,
                finding,
            });
        }
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
        catalog::COMMENTS => &["restates", "verbose", "history", "disabled"],
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
            "type",
            "redirect",
            "deserialize",
            "xxe",
        ],
        catalog::SENSITIVE_DATA => &[
            "logs_secret",
            "error_details",
            "logs_object_secret",
            "exception_to_client",
            "environment_to_client",
            "handler_leaks",
        ],
        catalog::UNSAFE_SETTINGS => &[
            "weakened",
            "tls",
            "hash",
            "random",
            "cors",
            "cookie",
            "debug",
            "token",
            "key",
            "csrf",
            "literal_secret",
        ],
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
        catalog::LAWS => &["states"],
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
    // Instruction sections and section pairs settle some signals by others.
    let settled_sections = match unit.rule {
        catalog::AGENT_CONTEXT => super::outcome::section_signals(&get),
        catalog::DOC_DUPLICATION => super::outcome::pair_signals(
            &get,
            matches!(
                unit.detail,
                Detail::DocPair {
                    translated: true,
                    ..
                }
            ),
        ),
        _ => None,
    };
    let mut questions: Vec<String> = match (settled_values, settled_sections) {
        (Some(signals), _) => signals
            .iter()
            .filter(|(_, o, _)| matches!(o, Outcome::Uncertain(_)))
            .map(|(q, ..)| question_label(q).to_string())
            .collect(),
        (None, Some(signals)) => signals
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Uncertain(_)))
            .map(|(q, _)| question_label(q).to_string())
            .collect(),
        (None, None) => undecided_questions(unit.rule, answers),
    };
    if answers.is_empty() {
        questions.push("no answer".into());
    }
    let values = match &unit.detail {
        Detail::Values { values, .. } | Detail::Constants { values, .. }
            if values.len() <= SHOWN_VALUES =>
        {
            values.clone()
        }
        _ => Vec::new(),
    };
    Undecided {
        unit: match &unit.detail {
            Detail::Outline { .. } => "file outline".into(),
            // Policies are often named by what they allow, the same on each table.
            Detail::Access(Access::Policy { table }) => format!("{} on {table}", unit.name),
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
        catalog::COMMENTS => "comment",
        catalog::INJECTION | catalog::SENSITIVE_DATA | catalog::UNSAFE_SETTINGS => "security unit",
        catalog::ACCESS_CONTROL => "access statement",
        catalog::WORKFLOWS => "workflow job",
        catalog::LAWS => "law",
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
            block = located_block(unit, blocks, judgments, "block")
                .filter(|b| !most_of(&b.location, &unit.locations));
            let bend = crate::analysis::bend::file(&plan.path);
            function_wording(name, strength, p, answers, (block, bend))
        }
        Detail::Outline {
            tests,
            groups,
            members,
            parts,
            ..
        } => {
            if let Some(part) = deciding_part(answers, parts) {
                symbol = part.names.first().cloned();
                locations = part.locations.clone();
                part_wording(part, strength, p)
            } else {
                let chosen = outline_groups(answers.get("module").copied(), groups, *members);
                symbol = chosen.first().map(|group| group.id.clone());
                if !chosen.is_empty() {
                    locations = chosen.iter().flat_map(|g| g.locations.clone()).collect();
                }
                let several =
                    several_kind(answers.get("split").copied(), answers.get("kind").copied());
                outline_wording(&chosen, *tests, several, strength, p)
            }
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
            let (wording, constant) = values_finding(unit, strength, p, answers, judgments);
            if let Some(location) = constant {
                symbol = location.symbol.clone();
                locations = vec![location];
            }
            wording
        }
        Detail::Security {
            sites, messages, ..
        } => {
            let (wording, site, named) =
                security_finding(unit, (sites, messages), strength, p, answers);
            block = site;
            category = Some(named);
            wording
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
            let (wording, named) = job_wording(name, expressions, strength, p, answers);
            category = Some(named);
            wording
        }
        Detail::Comment { .. } => {
            let reason = comment_reason(answers, documented(unit));
            comment_wording(name, &[(&unit.locations[0], reason)], strength, p)
        }
        Detail::Test { .. } => test_wording(name, strength, p, answers),
        Detail::Law => law_wording(name, strength, p, answers),
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
        suppressed: None,
    }
}
