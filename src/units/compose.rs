//! Pure composition from typed judgments to unit outcomes, rule dimensions,
//! findings and a file status; each unit's outcome comes from `outcome`.
use super::{
    Access, Block, Detail, FilePlan, Presence, UnitPlan,
    outcome::{
        Answers, Outcome, benefit, checks, choice, lowered, noul, open, origin_outcome, score,
        several_kind, unit_outcome, value_signals,
    },
    wording::{comment_reason, comment_wording},
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
    // The settle answers sit beside the checks they settle, under their own names.
    merged.extend(answers(judgments, &unit.id, Pass::Settle));
    merged
}

/// The settle Choices a security unit calls for, not yet asked: those whose
/// checks stay undecided after the trace and recheck while the unit is
/// uncertain, and for a consider or note resting on an undecided check,
/// where its text goes (the finding claims it likely reaches a client) and
/// where code that requests a URL runs, and every Choice for an injection
/// note in Django code; and those asked whenever their checks are not
/// clear, such as what a PHP page joins into HTML.
pub fn unsettled(unit: &UnitPlan, judgments: &[Judgment]) -> BTreeSet<&'static str> {
    use crate::units::security::{SETTLES, SettleWhen};
    if unit.presence != Presence::Judged
        || !security(unit.rule)
        || answers(judgments, &unit.id, Pass::Trace).is_empty()
    {
        return BTreeSet::new();
    }
    let merged = security_answers(unit, judgments);
    // Nearly every Django view places request values somewhere, so an
    // injection note that no check found ("values from another party …,
    // but no check found one placed unhandled") rests on its undecided
    // checks, such as a redirect to its own path with an id in it.
    let django_note = unit.rule == catalog::INJECTION
        && matches!(unit.detail, Detail::Security { django: true, .. });
    let open = |when: SettleWhen| match unit_outcome(unit, &merged) {
        Outcome::Uncertain(_) => true,
        Outcome::Note(_) if django_note => true,
        Outcome::Consider(_) | Outcome::Note(_) => when == SettleWhen::UndecidedOrFinding,
        _ => false,
    };
    let undecided = |q: &str| {
        merged
            .get(q)
            .is_some_and(|a| matches!(noul(a), Outcome::Uncertain(_)))
    };
    let not_clear = |q: &str| merged.get(q).is_some_and(|a| noul(a) != Outcome::Clear);
    SETTLES
        .iter()
        .filter(|kind| kind.rule == unit.rule && !merged.contains_key(kind.question))
        .filter(|kind| match kind.when {
            SettleWhen::NotClear => kind.checks.iter().any(|q| not_clear(q)),
            when => open(when) && kind.checks.iter().any(|q| undecided(q)),
        })
        .map(|kind| kind.question)
        .collect()
}

/// A unit's outcome and the answers it rests on: its first-pass answers with
/// the follow-ups its rule reads beside or in place of them.
fn resolved<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> (Outcome, Answers<'a>) {
    let merged = if security(unit.rule) {
        security_answers(unit, judgments)
    } else if let Some(pass) = beside(unit.rule) {
        beside_answers(unit, judgments, pass)
    } else if unit.rule == catalog::COMMENTS {
        comment_answers(unit, judgments)
    } else if unit.rule == catalog::TEST_VALUE {
        test_value_answers(unit, judgments)
    } else {
        return rechecked(unit, judgments);
    };
    (unit_outcome(unit, &merged), merged)
}

/// The pass of the follow-ups whose questions sit beside the first answers
/// under their own ids: document section and pair checks, the kind of a
/// large document, and benign-kind value checks.
fn beside(rule: &str) -> Option<Pass> {
    if [
        catalog::DOC_STALENESS,
        catalog::DOC_DUPLICATION,
        catalog::LARGE_DOCS,
    ]
    .contains(&rule)
    {
        Some(Pass::Trace)
    } else if [catalog::HARDCODED_VALUES, catalog::AGENT_CONTEXT].contains(&rule) {
        Some(Pass::Recheck)
    } else {
        None
    }
}

fn beside_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment], pass: Pass) -> Answers<'a> {
    let mut merged = answers(judgments, &unit.id, Pass::First);
    merged.extend(answers(judgments, &unit.id, pass));
    // How a pair's sections relate, or what a section treats its missing
    // names as, asked when its checks stay undecided.
    merged.extend(answers(judgments, &unit.id, Pass::Settle));
    merged
}

/// A comment's recheck replaces its first answers when the first stayed open
/// and the recheck decides, or neither decides; the kind of comment, asked
/// when it stays undecided, sits beside them.
fn comment_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> Answers<'a> {
    let first = answers(judgments, &unit.id, Pass::First);
    let before = unit_outcome(unit, &first);
    let recheck = answers(judgments, &unit.id, Pass::Recheck);
    let mut merged = if !recheck.is_empty()
        && open(unit, &first, before)
        && (unit_outcome(unit, &recheck).decisive() || !before.decisive())
    {
        recheck
    } else {
        first
    };
    merged.extend(answers(judgments, &unit.id, Pass::Settle));
    merged
}

/// A test recheck asks the hollow-test questions again with the code under
/// test and the setup; each answer replaces the first one unless only the
/// first is decisive.
fn test_value_answers<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> Answers<'a> {
    let mut merged = answers(judgments, &unit.id, Pass::First);
    for (question, answer) in answers(judgments, &unit.id, Pass::Recheck) {
        let first = merged.get(question).map(|a| noul(a));
        if noul(answer).decisive() || !first.is_some_and(Outcome::decisive) {
            merged.insert(question, answer);
        }
    }
    merged
}

/// The first-pass outcome, or the recheck's when the first called for one
/// and the recheck decides.
fn rechecked<'a>(unit: &UnitPlan, judgments: &'a [Judgment]) -> (Outcome, Answers<'a>) {
    let first = answers(judgments, &unit.id, Pass::First);
    let outcome = unit_outcome(unit, &first);
    let mut recheck = answers(judgments, &unit.id, Pass::Recheck);
    if unit.rule == catalog::FILE_ORGANIZATION {
        // The kind is asked apart from the recheck and read beside its split,
        // or beside the first split of a file too long for a recheck.
        if unit.recheck.is_none() {
            recheck.extend(first.iter().map(|(q, a)| (*q, *a)));
        }
        recheck.extend(answers(judgments, &unit.id, Pass::Trace));
    }
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
                origin_unsettled(u, judgments)
            } else if u.rule == catalog::HARDCODED_VALUES {
                value_undecided(u, judgments)
            } else {
                let first = answers(judgments, &u.id, Pass::First);
                open(u, &first, unit_outcome(u, &first))
            }
        })
        .map(|u| u.id.clone())
        .collect()
}

/// An injection unit whose checks are not all clear while its traced origin
/// stayed unclear or was the function's parameters.
fn origin_unsettled(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    let merged = security_answers(unit, judgments);
    let get = |q: &str| merged.get(q).copied();
    unit.rule == catalog::INJECTION
        && !checks(unit.rule, &get).iter().all(|o| *o == Outcome::Clear)
        && matches!(
            merged.get("origin").map(|a| origin_outcome(a)),
            Some(Outcome::Uncertain(_) | Outcome::Consider(_))
        )
}

/// A hardcoded-value unit with a question its first pass left undecided.
fn value_undecided(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    let first = answers(judgments, &unit.id, Pass::First);
    let get = |q: &str| first.get(q).copied();
    value_signals(&get, &unit.detail, false).is_some_and(|signals| {
        signals
            .iter()
            .any(|(_, o, _)| matches!(o, Outcome::Uncertain(_)))
    })
}

/// Outlines whose recheck left the split Score undecided, or whose first
/// answer did when the file is too long for a recheck, and large documents
/// whose split Score stayed undecided, whose kind has not been asked yet;
/// section pairs and stale sections whose checks stayed undecided and whose
/// settle has not been asked yet.
pub fn unkinded_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| match u.detail {
            Detail::DocPair { .. } | Detail::Stale { .. } => unsettled_check(u, judgments),
            Detail::Comment { .. } => unsettled_comment(u, judgments),
            _ => unkinded_split(u, judgments),
        })
        .map(|u| u.id.clone())
        .collect()
}

/// An outline or large document not yet asked its kind whose split Score
/// stayed undecided: the recheck's for an outline that has one, else the
/// first.
fn unkinded_split(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    if ![catalog::FILE_ORGANIZATION, catalog::LARGE_DOCS].contains(&unit.rule)
        || !answers(judgments, &unit.id, Pass::Trace).is_empty()
    {
        return false;
    }
    let pass = if unit.recheck.is_some() && unit.rule != catalog::LARGE_DOCS {
        Pass::Recheck
    } else {
        Pass::First
    };
    answers(judgments, &unit.id, pass)
        .get("split")
        .is_some_and(|a| matches!(benefit(a), Outcome::Uncertain(_)))
}

/// A section pair or stale section whose checks were asked, stayed
/// undecided, and whose settle has not been asked.
fn unsettled_check(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    !answers(judgments, &unit.id, Pass::Trace).is_empty()
        && answers(judgments, &unit.id, Pass::Settle).is_empty()
        && matches!(resolved(unit, judgments).0, Outcome::Uncertain(_))
}

/// A comment still undecided after its recheck, or without one, whose kind
/// has not been asked.
fn unsettled_comment(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    answers(judgments, &unit.id, Pass::Settle).is_empty()
        && (unit.recheck.is_none() || !answers(judgments, &unit.id, Pass::Recheck).is_empty())
        && matches!(resolved(unit, judgments).0, Outcome::Uncertain(_))
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

/// A shared-logic finding whose every copy lies inside tests a redundancy
/// finding already names says the same thing twice: on sqlite-utils, 12
/// test pairs were reported by both rules. The redundancy finding stays,
/// since it says which test to merge or delete.
fn drop_copies_of_redundant_tests(findings: &mut Vec<Finding>) {
    let tests: Vec<crate::schema::Location> = findings
        .iter()
        .filter(|f| f.rule == catalog::id(catalog::TEST_REDUNDANCY))
        .flat_map(|f| f.locations.iter().cloned())
        .collect();
    let named = |l: &crate::schema::Location| {
        tests
            .iter()
            .any(|t| t.path == l.path && t.start_line <= l.start_line && l.end_line <= t.end_line)
    };
    findings.retain(|f| {
        f.rule != catalog::id(catalog::SHARED_LOGIC)
            || f.locations.is_empty()
            || !f.locations.iter().all(named)
    });
}

/// A redundant test pair, with the index of its finding (a note) when it
/// reached a consider, which a group of three or more tests reports instead.
struct Redundant<'p> {
    unit: &'p UnitPlan,
    names: &'p [String; 2],
    subject: &'p String,
    p: f64,
    finding: Option<usize>,
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
        let outcome = if unnamed_value(unit, judgments) {
            lowered(lowered(outcome))
        } else if unnamed_outline(unit, judgments) || few.contains(unit.id.as_str()) {
            lowered(outcome)
        } else {
            outcome
        };
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
        catalog::DOC_DUPLICATION => super::outcome::pair_signals(&get),
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
        Detail::Values { values, .. } | Detail::Constants { values }
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
            function_wording(name, strength, p, answers, block)
        }
        Detail::Outline { tests, groups, .. } => {
            let chosen = outline_groups(answers.get("module").copied(), groups);
            symbol = chosen.first().map(|group| group.id.clone());
            if !chosen.is_empty() {
                locations = chosen.iter().flat_map(|g| g.locations.clone()).collect();
            }
            let several = several_kind(answers.get("split").copied(), answers.get("kind").copied());
            outline_wording(&chosen, *tests, several, strength, p)
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
            let unnamed = unnamed_value(unit, judgments)
                .then(|| strength_of(resolved(unit, judgments).0).map(|(s, _)| s))
                .flatten();
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
        Detail::Comment { .. } => {
            let reason = comment_reason(answers, documented(unit));
            comment_wording(name, &[(&unit.locations[0], reason)], strength, p)
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
/// values to offer. Its finding is a note, since a reader cannot tell what
/// to change: one level lower, 8 of lobsters' 10 such considers were wrong.
fn unnamed_value(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    matches!(unit.detail, Detail::Values { .. })
        && located_value(unit, judgments).is_none()
        && matches!(
            resolved(unit, judgments).0,
            Outcome::Review(_) | Outcome::Consider(_)
        )
}

/// A file-organization consider that says only that some members could
/// move, naming no group: the module Choice was not asked (one group or
/// none) or spread wider than two groups, and no kind of file decided it.
/// Its finding is a note, since a reader cannot tell which members to move.
/// A review, or a consider the kind decided, says to split the whole file.
fn unnamed_outline(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    let Detail::Outline { groups, .. } = &unit.detail else {
        return false;
    };
    let (outcome, answers) = resolved(unit, judgments);
    let get = |q: &str| answers.get(q).copied();
    matches!(outcome, Outcome::Consider(_))
        && several_kind(get("split"), get("kind")).is_none()
        && outline_groups(get("module"), groups).is_empty()
}

/// The group a module Choice picks clearly, or else the two it leans toward
/// when together they reach the location probability: flask's `cli.py`
/// split 0.45 and 0.23 over two of six groups. None when it spreads wider.
fn outline_groups<'g>(
    module: Option<&Answer>,
    groups: &'g [super::GroupInfo],
) -> Vec<&'g super::GroupInfo> {
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

/// Whether a block spans most of its function, three quarters or more:
/// naming it as the part to extract says no more than the finding does, as
/// lines 206–390 of just's 198-line `Justfile::run` did.
fn most_of(block: &crate::schema::Location, function: &[crate::schema::Location]) -> bool {
    let lines = |l: &crate::schema::Location| l.end_line + 1 - l.start_line;
    function
        .first()
        .is_some_and(|f| lines(block) * 4 >= lines(f) * 3)
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

/// Three or more tests linked by overlapping pairs on one subject: the tests
/// a chain of such pairs connects. Two pairs of one subject that share no
/// test stay two pairs; grouped by subject alone, sinatra's pair of redirect
/// tests and pair of deny tests of `get` read as four overlapping tests.
/// Also returns the indices of the consider pair findings each group reports.
fn over_tested(plan: &FilePlan, redundant: &[Redundant<'_>]) -> (Vec<Finding>, BTreeSet<usize>) {
    type Cluster<'a> = (
        &'a String,
        BTreeSet<&'a String>,
        Vec<crate::schema::Location>,
        f64,
        Vec<Option<usize>>,
    );
    let mut clusters: Vec<Cluster<'_>> = Vec::new();
    for Redundant {
        unit,
        names,
        subject,
        p,
        finding,
    } in redundant
    {
        let mut joined: Cluster<'_> = (subject, BTreeSet::new(), Vec::new(), *p, vec![*finding]);
        let mut index = 0;
        while index < clusters.len() {
            let (other, tests, ..) = &clusters[index];
            if *other == *subject && names.iter().any(|name| tests.contains(name)) {
                let (_, tests, locations, q, findings) = clusters.remove(index);
                joined.1.extend(tests);
                joined.2.extend(locations);
                joined.3 = joined.3.min(q);
                joined.4.extend(findings);
            } else {
                index += 1;
            }
        }
        for (name, location) in names.iter().zip(&unit.locations) {
            if joined.1.insert(name) {
                joined.2.push(location.clone());
            }
        }
        clusters.push(joined);
    }
    clusters.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    clusters.retain(|(_, tests, ..)| tests.len() >= 3);
    let grouped = clusters
        .iter()
        .flat_map(|(.., findings)| findings.iter().flatten().copied())
        .collect();
    let groups = clusters
        .into_iter()
        .map(|(subject, tests, mut locations, p, _)| {
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
        .collect();
    (groups, grouped)
}

fn documented(unit: &UnitPlan) -> bool {
    matches!(
        unit.detail,
        Detail::Comment {
            documentation: true,
            ..
        }
    )
}

/// Lines a unit's comments may span in all and still be few: a comment or
/// two a reader skips in a moment cost little.
const FEW_COMMENT_LINES: usize = 3;

/// Comments raised to a consider whose unit's considered comments span
/// fewer than `FEW_COMMENT_LINES` lines in all: their finding is a note.
fn few_comment_lines<'a>(plan: &'a FilePlan, judgments: &[Judgment]) -> BTreeSet<&'a str> {
    let mut considered = BTreeMap::<&str, Vec<&UnitPlan>>::new();
    for unit in &plan.units {
        if let Detail::Comment { owner, .. } = &unit.detail
            && unit.presence == Presence::Judged
            && matches!(resolved(unit, judgments).0, Outcome::Consider(_))
        {
            considered.entry(owner.as_str()).or_default().push(unit);
        }
    }
    considered
        .into_values()
        .filter(|units| units.iter().map(|u| u.lines).sum::<usize>() < FEW_COMMENT_LINES)
        .flatten()
        .map(|u| u.id.as_str())
        .collect()
}

/// One finding per unit and strength for its comments a reader could do
/// without, listing each with what makes it so, at the lowest probability
/// among them.
fn comment_findings(
    plan: &FilePlan,
    commented: &[(&UnitPlan, Strength, f64, &'static str)],
) -> Vec<Finding> {
    let mut grouped =
        BTreeMap::<(&str, Strength), Vec<&(&UnitPlan, Strength, f64, &'static str)>>::new();
    for entry in commented {
        let Detail::Comment { owner, .. } = &entry.0.detail else {
            continue;
        };
        grouped
            .entry((owner.as_str(), entry.1))
            .or_default()
            .push(entry);
    }
    grouped
        .into_iter()
        .map(|((owner, strength), mut entries)| {
            entries.sort_by_key(|(unit, ..)| unit.locations[0].start_line);
            let p = entries.iter().map(|(_, _, p, _)| *p).fold(1.0, f64::min);
            let listed: Vec<(&crate::schema::Location, &'static str)> = entries
                .iter()
                .map(|(unit, _, _, reason)| (&unit.locations[0], *reason))
                .collect();
            let (message, action) = comment_wording(owner, &listed, strength, p);
            let locations: Vec<crate::schema::Location> =
                listed.iter().map(|(l, _)| (*l).clone()).collect();
            let lines = entries.iter().map(|(unit, ..)| unit.lines).sum();
            let identities: Vec<&str> = std::iter::once(owner)
                .chain(entries.iter().map(|(unit, ..)| unit.identity.as_str()))
                .collect();
            Finding {
                rule: catalog::id(catalog::COMMENTS).into(),
                strength,
                line: locations[0].start_line,
                message,
                action: action.into(),
                symbol: (owner != super::comments::TOP_LEVEL).then(|| owner.to_string()),
                rule_version: catalog::rule_version(catalog::COMMENTS).into(),
                concern_probability: p,
                locations,
                quote: entries[0].0.quote.clone(),
                category: None,
                values: Vec::new(),
                fingerprint: fingerprint(catalog::COMMENTS, plan, &super::identity(&identities)),
                rank: rank(p, lines),
                baselined: false,
            }
        })
        .collect()
}
