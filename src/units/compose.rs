//! Pure composition from typed judgments to unit outcomes, rule dimensions,
//! findings and a file status. Thresholds are the shared 0.80 policy on a
//! Score whose top level is the actionable concern: review when the top level
//! reaches it, consider when the middle-or-top mass does, clear when the top
//! level is ruled out (its complement reaches it), otherwise uncertain.
use super::{Detail, FilePlan, Presence, UnitPlan};
use crate::{
    catalog,
    response::{LOCATION_PROBABILITY, REVIEW_PROBABILITY, probability_at_least},
    schema::{Answer, Dimension, Finding, Judgment, Pass, Status, Strength, UnitCounts, hash},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Review(f64),
    Consider(f64),
    Clear,
    /// Carries the concern probability that stayed below the thresholds.
    Uncertain(f64),
    /// No answers were recorded, for example after a failed request.
    Missing,
}

impl Outcome {
    fn decisive(self) -> bool {
        matches!(self, Self::Review(_) | Self::Consider(_) | Self::Clear)
    }

    fn concern(self) -> f64 {
        match self {
            Self::Review(p) | Self::Consider(p) | Self::Uncertain(p) => p,
            Self::Clear | Self::Missing => 0.0,
        }
    }
}

pub struct Composed {
    pub dimensions: BTreeMap<String, Dimension>,
    pub findings: Vec<Finding>,
    pub status: Status,
}

fn at_least(value: f64) -> bool {
    probability_at_least(value, REVIEW_PROBABILITY)
}

fn levels(answer: &Answer) -> Option<[f64; 3]> {
    let Answer::Score { probabilities, .. } = answer else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    if mass <= 0.0 {
        return None;
    }
    let level = |i: usize| probabilities.get(&i.to_string()).copied().unwrap_or(0.0) / mass;
    Some([level(0), level(1), level(2)])
}

pub fn score(answer: &Answer) -> Outcome {
    let Some([bottom, middle, top]) = levels(answer) else {
        return Outcome::Missing;
    };
    if at_least(top) {
        Outcome::Review(top)
    } else if at_least(middle + top) {
        Outcome::Consider(middle + top)
    } else if at_least(bottom + middle) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(top)
    }
}

pub fn noul(answer: &Answer) -> Outcome {
    let Answer::Noul { noul } = answer else {
        return Outcome::Missing;
    };
    if at_least(*noul) {
        Outcome::Review(*noul)
    } else if at_least(1.0 - noul) {
        Outcome::Clear
    } else {
        Outcome::Uncertain(*noul)
    }
}

fn choice(answer: Option<&Answer>) -> Option<(&str, f64)> {
    let Answer::Choice {
        choice,
        probabilities,
        ..
    } = answer?
    else {
        return None;
    };
    let mass: f64 = probabilities.values().sum();
    let p = probabilities.get(choice).copied().unwrap_or(0.0) / mass.max(f64::MIN_POSITIVE);
    (choice != "none" && probability_at_least(p, LOCATION_PROBABILITY)).then_some((choice, p))
}

type Answers<'a> = BTreeMap<&'a str, &'a Answer>;

fn answers<'a>(judgments: &'a [Judgment], unit: &str, pass: Pass) -> Answers<'a> {
    judgments
        .iter()
        .filter(|j| j.unit == unit && j.pass == pass)
        .map(|j| (j.question.as_str(), &j.answer))
        .collect()
}

fn unit_outcome(unit: &UnitPlan, answers: &Answers<'_>) -> Outcome {
    let get = |q: &str| answers.get(q).copied();
    let result = match unit.rule {
        catalog::FUNCTION_SIMPLIFICATION => (|| {
            let split = score(get("split")?);
            // Flattening is asked only for deeply nested functions.
            let flatten = get("flatten").map(score);
            Some(match (split, flatten) {
                (Outcome::Review(p), _) | (_, Some(Outcome::Review(p))) => Outcome::Review(p),
                (Outcome::Consider(p), _) | (_, Some(Outcome::Consider(p))) => Outcome::Consider(p),
                (Outcome::Clear, None | Some(Outcome::Clear)) => Outcome::Clear,
                (split, flatten) => {
                    Outcome::Uncertain(split.concern().max(flatten.map_or(0.0, Outcome::concern)))
                }
            })
        })(),
        catalog::FILE_ORGANIZATION => get("split").map(score),
        catalog::SHARED_LOGIC => (|| {
            // Repetition the behavior requires is not a shared-logic concern.
            if matches!(noul(get("required")?), Outcome::Review(_)) {
                return Some(Outcome::Clear);
            }
            let same = score(get("same")?);
            // Cases written out in one test are a style choice, never a required change.
            Some(match (same, &unit.detail) {
                (
                    Outcome::Review(p),
                    Detail::Pair {
                        within_test: true, ..
                    },
                ) => Outcome::Consider(p),
                (same, _) => same,
            })
        })(),
        catalog::TEST_VALUE => (|| {
            let hollow = [noul(get("own_logic")?), noul(get("mock_only")?)];
            let weak = [noul(get("internal")?), noul(get("several")?)];
            let strongest = |outcomes: &[Outcome]| {
                outcomes
                    .iter()
                    .filter_map(|o| match o {
                        Outcome::Review(p) => Some(*p),
                        _ => None,
                    })
                    .reduce(f64::max)
            };
            // Weak signals can raise a consider; their uncertainty does not block a clear.
            Some(if let Some(p) = strongest(&hollow) {
                Outcome::Review(p)
            } else if let Some(p) = strongest(&weak) {
                Outcome::Consider(p)
            } else if hollow.iter().all(|o| *o == Outcome::Clear) {
                Outcome::Clear
            } else {
                Outcome::Uncertain(hollow.iter().map(|o| o.concern()).fold(0.0, f64::max))
            })
        })(),
        catalog::TEST_REDUNDANCY => get("overlap").map(score),
        _ => None,
    };
    result.unwrap_or(Outcome::Missing)
}

/// The first-pass outcome, replaced by a decisive recheck when one exists.
fn resolved<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> (Outcome, Answers<'a>) {
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

/// Judged units whose first pass stayed undecided and that have no recheck yet.
pub fn uncertain_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| {
            matches!(
                unit_outcome(u, &answers(judgments, &u.id, Pass::First)),
                Outcome::Uncertain(_)
            ) && answers(judgments, &u.id, Pass::Recheck).is_empty()
        })
        .map(|u| u.id.clone())
        .collect()
}

pub fn compose(plan: &FilePlan, judgments: &[Judgment]) -> Composed {
    let mut counts = BTreeMap::<&str, UnitCounts>::new();
    let mut concern = BTreeMap::<&str, f64>::new();
    let mut findings = Vec::new();
    let mut redundant = Vec::new();
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
                findings.push(finding(plan, unit, Strength::Review, p, &answers));
            }
            Outcome::Consider(p) => {
                count.consider += 1;
                findings.push(finding(plan, unit, Strength::Consider, p, &answers));
            }
            Outcome::Clear => count.clear += 1,
            Outcome::Uncertain(_) | Outcome::Missing => count.uncertain += 1,
        }
        if let (Detail::TestPair { names, subject }, Outcome::Review(p) | Outcome::Consider(p)) =
            (&unit.detail, outcome)
        {
            redundant.push((unit, names, subject, p));
        }
    }
    findings.extend(over_tested(plan, &redundant));
    let mut dimensions = BTreeMap::new();
    for rule in plan.rules.keys() {
        let count = counts.remove(rule).unwrap_or_default();
        let status = if count.review > 0 {
            Status::Review
        } else if count.consider > 0 {
            Status::Consider
        } else if count.needs_context > 0 {
            Status::NeedsContext
        } else if count.uncertain > 0 {
            Status::Uncertain
        } else if count.clear > 0 {
            Status::Clear
        } else {
            Status::NotApplicable
        };
        dimensions.insert(
            rule.to_string(),
            Dimension {
                decision_basis: basis(rule, &count),
                status,
                concern_probability: concern.get(rule).copied().unwrap_or(0.0),
                rule_version: catalog::rule_version(rule).into(),
                units: count,
            },
        );
    }
    findings.sort_by(|a, b| b.rank.total_cmp(&a.rank));
    let status = file_status(&dimensions, &findings);
    Composed {
        dimensions,
        findings,
        status,
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
    } else if !findings.is_empty() {
        Status::Consider
    } else if any(Status::NeedsContext) {
        Status::NeedsContext
    } else if any(Status::Uncertain) {
        Status::Uncertain
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
    hash(format!("{rule}\u{0}{}\u{0}{identity}", plan.path.display()).as_bytes())
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
) -> Finding {
    let review = strength == Strength::Review;
    let name = &unit.name;
    let mut locations = unit.locations.clone();
    let mut symbol = Some(name.clone());
    let (message, action) = match (&unit.detail, unit.rule) {
        (Detail::Function, _) => {
            let flatten = answers.get("flatten").map(|a| score(a));
            let flattening = match strength {
                Strength::Review => {
                    matches!(flatten, Some(Outcome::Review(_)))
                        && !matches!(
                            answers.get("split").map(|a| score(a)),
                            Some(Outcome::Review(_))
                        )
                }
                Strength::Consider => {
                    matches!(flatten, Some(Outcome::Consider(_)))
                        && !matches!(
                            answers.get("split").map(|a| score(a)),
                            Some(Outcome::Consider(_))
                        )
                }
            };
            match (review, flattening) {
                (true, false) => (
                    format!(
                        "`{name}` mixes separate jobs in long blocks; splitting it would make it easier to understand ({p:.2})."
                    ),
                    "Extract each separate job into its own named function",
                ),
                (true, true) => (
                    format!(
                        "`{name}` has nested or repeated branches that hide its main path ({p:.2})."
                    ),
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
        (Detail::Outline { groups }, _) => {
            symbol = None;
            let chosen = choice(answers.get("module").copied())
                .and_then(|(id, _)| groups.iter().position(|g| g.id == id));
            let mut detail = String::new();
            if let Some(index) = chosen {
                let group = &groups[index];
                locations = group.locations.clone();
                symbol = Some(group.id.clone());
                let shown: Vec<_> = group
                    .names
                    .iter()
                    .take(6)
                    .map(|n| format!("`{n}`"))
                    .collect();
                let more = group.names.len().saturating_sub(shown.len());
                detail = format!(
                    " {} ({}{}) would be most useful as its own module.",
                    group.id,
                    shown.join(", "),
                    if more > 0 {
                        format!(" and {more} more")
                    } else {
                        String::new()
                    }
                );
            }
            if review {
                (
                    format!(
                        "This file holds two or more unrelated responsibilities ({p:.2}).{detail}"
                    ),
                    if chosen.is_some() {
                        "Move that group into its own module"
                    } else {
                        "Split the file along its separate purposes"
                    },
                )
            } else {
                (
                    format!(
                        "Some members of this file could live in a separate module ({p:.2}).{detail}"
                    ),
                    "Consider moving that set of members into its own module",
                )
            }
        }
        (
            Detail::Pair {
                differences,
                within_test,
                in_tests,
            },
            _,
        ) => {
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
            if *within_test {
                (
                    format!("{name} repeat the same steps inside one test ({p:.2}).{renamed}"),
                    "Consider a table of cases or a local helper for the repeated steps",
                )
            } else if review {
                (
                    format!(
                        "{name} perform the same steps for the same purpose ({p:.2}).{renamed}"
                    ),
                    if *in_tests {
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
        (Detail::Test, _) => {
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
        (Detail::TestPair { .. }, _) => {
            symbol = None;
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
    };
    let lines = locations
        .iter()
        .map(|l| l.end_line + 1 - l.start_line)
        .sum::<usize>()
        .max(unit.lines.min(1));
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
        fingerprint: fingerprint(unit.rule, plan, &unit.identity),
        rank: rank(p, lines),
        baselined: false,
    }
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
