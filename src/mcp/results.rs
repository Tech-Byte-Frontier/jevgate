//! A report as the MCP tools return it to an agent: the headline, the gate,
//! why files failed or were skipped, the findings in the order to act on
//! them, and the units Jev left undecided as verify items, with the question
//! each left open, the evidence it named and the probability of each answer.
//! Claude Code shows the model only the structured result when a tool
//! returns one, so it stands on its own.
use crate::{
    gate::Gate,
    output,
    schema::{Answer, Finding, Gating, OpenQuestion, Report, Status, Strength, Undecided},
};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// Findings a result lists unless the call asks for another number.
pub(super) const DEFAULT_FINDINGS: usize = 20;
/// Verify items a result lists unless the call asks for another number.
pub(super) const DEFAULT_VERIFY: usize = 5;
/// The most findings or verify items one call can ask for.
pub(super) const MAX_LISTED: usize = 200;
/// Answers less likely than this are left out of a verify item: a Choice's
/// other options would only repeat its criteria.
const SHOWN_ANSWER: f64 = 0.05;

/// What one call asks for: which files, whether notes count, and how many
/// findings and verify items to list.
pub(super) struct Selection<'a> {
    prefix: Option<&'a str>,
    notes: bool,
    max_findings: usize,
    max_verify: usize,
}

impl<'a> Selection<'a> {
    /// The selection a call's arguments ask for; `max_findings` and
    /// `max_verify` must be whole numbers within their bounds.
    pub(super) fn new(arguments: &Value, prefix: Option<&'a str>, notes: bool) -> Result<Self> {
        Ok(Self {
            prefix,
            notes,
            max_findings: bounded(arguments, "max_findings", 1, DEFAULT_FINDINGS)?,
            max_verify: bounded(arguments, "max_verify", 0, DEFAULT_VERIFY)?,
        })
    }

    fn includes(&self, path: &Path) -> bool {
        self.prefix.is_none_or(|prefix| path.starts_with(prefix))
    }
}

/// A whole-number argument from `least` to [`MAX_LISTED`], or `default`
/// when the call leaves it out.
fn bounded(arguments: &Value, name: &str, least: usize, default: usize) -> Result<usize> {
    match &arguments[name] {
        Value::Null => Ok(default),
        value => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (least..=MAX_LISTED).contains(n))
            .with_context(|| {
                format!("`{name}` must be a whole number from {least} to {MAX_LISTED}")
            }),
    }
}

/// The exit code `jevgate check` gives a report: a dry run exits 0.
pub(super) fn exit_code(report: &Report) -> u8 {
    if report.dry_run {
        0
    } else {
        crate::gate::exit_code(report)
    }
}

/// The structured result of `jevgate_check` and `jevgate_findings`.
#[derive(Serialize)]
pub(super) struct Structured<'r> {
    headline: String,
    status: &'r str,
    complete: bool,
    dry_run: bool,
    exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    gate: Option<&'r Gate>,
    errors: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    skipped: Vec<String>,
    usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    planned: Option<output::Preview>,
    findings: Vec<FindingView<'r>>,
    total_findings: usize,
    verify: Vec<VerifyView<'r>>,
    total_verify: usize,
}

#[derive(Serialize)]
struct Usage {
    api_requests: u32,
    input_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    usd: Option<f64>,
}

pub(super) fn structured<'r>(
    report: &'r Report,
    selection: &Selection,
    exit_code: u8,
) -> Structured<'r> {
    let findings = findings(report, selection);
    let undecided = undecided(report, selection);
    let lines = |status: Status, label: &str| -> Vec<String> {
        output::reasons(report, status)
            .into_iter()
            .map(|(reason, n)| format!("{label} {n}: {reason}"))
            .collect()
    };
    Structured {
        headline: output::headline(report),
        status: &report.status,
        complete: report.complete,
        dry_run: report.dry_run,
        exit_code,
        gate: report.gate.as_ref(),
        errors: report
            .errors
            .iter()
            .cloned()
            .chain(lines(Status::Error, "Failed"))
            .collect(),
        skipped: lines(Status::Skipped, "Skipped"),
        usage: Usage {
            api_requests: report.api_requests,
            input_tokens: report.paid_input_tokens,
            usd: report.estimated_usd,
        },
        planned: report.dry_run.then(|| output::preview(report)),
        total_findings: findings.len(),
        findings: findings
            .into_iter()
            .take(selection.max_findings)
            .map(|(path, finding)| FindingView::new(path, finding))
            .collect(),
        total_verify: undecided.len(),
        verify: undecided
            .into_iter()
            .take(selection.max_verify)
            .map(|(path, rule, unit)| VerifyView::new(path, rule, unit))
            .collect(),
    }
}

/// The selected findings in the order to act on them: those that fail the
/// gate first, then new before accepted (baselined or allowed), reviews
/// before considers before notes, each by rank, so a cap never leaves out a
/// failure for a finding that only warns.
fn findings<'r>(report: &'r Report, selection: &Selection) -> Vec<(&'r Path, &'r Finding)> {
    let mut findings: Vec<_> = output::ranked(report)
        .into_iter()
        .filter(|(path, f)| {
            selection.includes(path) && (selection.notes || f.strength != Strength::Note)
        })
        .collect();
    // A stable sort keeps the rank order within each group.
    findings.sort_by_key(|(_, f)| (!f.fails_gate(), f.accepted(), std::cmp::Reverse(f.strength)));
    findings
}

/// The selected undecided units, highest concern first, with their file and
/// rule key.
fn undecided<'r>(
    report: &'r Report,
    selection: &Selection,
) -> Vec<(&'r Path, &'r str, &'r Undecided)> {
    let mut units: Vec<_> = report
        .files
        .iter()
        .filter(|file| selection.includes(&file.path))
        .flat_map(|file| {
            file.dimensions.iter().flat_map(move |(rule, dimension)| {
                dimension
                    .undecided
                    .iter()
                    .map(move |unit| (file.path.as_path(), rule.as_str(), unit))
            })
        })
        .collect();
    units.sort_by(|a, b| b.2.concern.total_cmp(&a.2.concern));
    units
}

/// Probabilities to two decimals: an agent weighs 0.42, not 0.41999998.
fn rounded(probability: f64) -> f64 {
    (probability * 100.0).round() / 100.0
}

#[derive(Serialize)]
struct FindingView<'r> {
    id: &'r str,
    path: &'r Path,
    line: usize,
    end_line: usize,
    rule: &'r str,
    strength: Strength,
    message: &'r str,
    action: &'r str,
    probability: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<&'r str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'r str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gate: Option<Gating>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    baselined: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    suppressed: Option<&'r str>,
}

impl<'r> FindingView<'r> {
    fn new(path: &'r Path, finding: &'r Finding) -> Self {
        Self {
            id: &finding.fingerprint,
            path,
            line: finding.line,
            end_line: finding
                .locations
                .first()
                .map_or(finding.line, |l| l.end_line),
            rule: &finding.rule,
            strength: finding.strength,
            message: &finding.message,
            action: &finding.action,
            probability: rounded(finding.concern_probability),
            symbol: finding.symbol.as_deref(),
            category: finding.category.as_deref(),
            gate: finding.gate,
            baselined: finding.baselined,
            suppressed: finding.suppressed.as_deref(),
        }
    }
}

#[derive(Serialize)]
struct VerifyView<'r> {
    #[serde(skip_serializing_if = "str::is_empty")]
    id: &'r str,
    path: &'r Path,
    line: usize,
    end_line: usize,
    rule: &'static str,
    unit: &'r str,
    concern: f64,
    questions: Vec<QuestionView<'r>>,
}

impl<'r> VerifyView<'r> {
    /// A report written before 0.27 holds no open questions: the labels of
    /// the questions left undecided stand in for them.
    fn new(path: &'r Path, rule: &str, unit: &'r Undecided) -> Self {
        let questions = if unit.open.is_empty() {
            unit.questions
                .iter()
                .map(|label| QuestionView {
                    question: label,
                    evidence: &[],
                    answers: Vec::new(),
                })
                .collect()
        } else {
            unit.open.iter().map(QuestionView::new).collect()
        };
        Self {
            id: &unit.fingerprint,
            path,
            line: unit.line,
            end_line: unit.locations.first().map_or(unit.line, |l| l.end_line),
            rule: crate::catalog::id(rule),
            unit: &unit.unit,
            concern: rounded(unit.concern),
            questions,
        }
    }
}

#[derive(Serialize)]
struct QuestionView<'r> {
    question: &'r str,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    evidence: &'r [String],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    answers: Vec<AnswerView<'r>>,
}

impl<'r> QuestionView<'r> {
    fn new(open: &'r OpenQuestion) -> Self {
        let probabilities: Vec<(&str, f64)> = match &open.answer {
            Answer::Noul { noul } => vec![("true", *noul), ("false", 1.0 - noul)],
            Answer::Choice { probabilities, .. } | Answer::Score { probabilities, .. } => {
                probabilities
                    .iter()
                    .map(|(o, p)| (o.as_str(), *p))
                    .collect()
            }
        };
        let mut answers: Vec<AnswerView> = probabilities
            .into_iter()
            .filter(|(_, p)| *p >= SHOWN_ANSWER)
            .map(|(option, p)| AnswerView {
                option,
                meaning: open.options.get(option).map_or("", String::as_str),
                probability: rounded(p),
            })
            .collect();
        answers.sort_by(|a, b| b.probability.total_cmp(&a.probability));
        Self {
            question: &open.text,
            evidence: &open.evidence,
            answers,
        }
    }
}

#[derive(Serialize)]
struct AnswerView<'r> {
    option: &'r str,
    #[serde(skip_serializing_if = "str::is_empty")]
    meaning: &'r str,
    probability: f64,
}

impl AnswerView<'_> {
    /// A short name for the answer in text: a Score level's verdict, which
    /// opens its meaning (`No.`, `Slightly.`, `Yes.`); yes or no for a
    /// Noul; else the option's own name.
    fn label(&self) -> &str {
        let verdict = self
            .meaning
            .split_once('.')
            .map(|(first, _)| first)
            .filter(|word| !word.is_empty() && word.chars().all(char::is_alphabetic));
        match (verdict, self.option) {
            (Some(word), _) => word,
            (None, "true") => "yes",
            (None, "false") => "no",
            (None, option) => option,
        }
    }
}

impl Structured<'_> {
    /// The verify items as text, for clients that show the model only the
    /// text: each unit, then each open question with its answers' odds.
    pub(super) fn verify_text(&self) -> Option<String> {
        if self.verify.is_empty() {
            return None;
        }
        let shown = if self.verify.len() < self.total_verify {
            format!(", top {}", self.verify.len())
        } else {
            String::new()
        };
        let mut text = format!(
            "Verify ({} undecided{shown}; read the code and decide, they never fail the gate):",
            self.total_verify
        );
        for item in &self.verify {
            text.push_str(&format!(
                "\n  {}:{} [{}] {} (concern {:.0}%)",
                item.path.display(),
                item.line,
                item.rule,
                item.unit,
                item.concern * 100.0
            ));
            for question in &item.questions {
                let answers: Vec<String> = question
                    .answers
                    .iter()
                    .map(|a| format!("{} {:.0}%", a.label(), a.probability * 100.0))
                    .collect();
                text.push_str(&format!("\n    {}", question.question));
                if !answers.is_empty() {
                    text.push_str(&format!(" {}", answers.join(" · ")));
                }
            }
        }
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        schema::{Dimension, Location, Pass},
        tests::finding,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn all() -> Selection<'static> {
        Selection::new(&json!({}), None, false).unwrap()
    }

    /// A settled report of one file, `src/a.rs`, holding `findings` and the
    /// function-simplification units left `undecided`.
    fn report(findings: Vec<Finding>, undecided: Vec<Undecided>) -> Report {
        let project = crate::tests::Project::new();
        project.write("src/a.rs", &crate::tests::function("a"));
        let (_, mut report) = crate::tests::snapshot(&project, &crate::tests::args());
        let file = &mut report.files[0];
        file.status = Status::Review;
        file.findings = findings;
        file.dimensions = BTreeMap::from([(
            "function_simplification".to_string(),
            Dimension {
                status: Status::Uncertain,
                concern_probability: 0.0,
                decision_basis: String::new(),
                rule_version: String::new(),
                units: Default::default(),
                undecided,
            },
        )]);
        report.update_status();
        crate::gate::evaluate(&mut report, &crate::tests::args());
        report
    }

    fn found(strength: Strength, rank: f64, id: &str) -> Finding {
        Finding {
            rank,
            fingerprint: id.into(),
            ..finding(strength)
        }
    }

    /// An undecided unit at `line`, with a split question answered `levels`.
    fn open_unit(name: &str, line: usize, concern: f64, levels: [f64; 3]) -> Undecided {
        Undecided {
            unit: name.into(),
            line,
            questions: vec!["splitting".into()],
            values: Vec::new(),
            fingerprint: format!("{name}-id"),
            locations: vec![Location {
                path: "src/a.rs".into(),
                start_line: line,
                end_line: line + 9,
                symbol: Some(name.into()),
            }],
            concern,
            open: vec![OpenQuestion {
                id: "split".into(),
                pass: Pass::First,
                text: "Would splitting the function in `functions[0].source` help?".into(),
                evidence: vec!["functions[0].source".into()],
                options: BTreeMap::from([
                    ("0".into(), "No. It reads as one job.".into()),
                    ("1".into(), "Slightly. One block could be named.".into()),
                    ("2".into(), "Yes. It mixes separate jobs.".into()),
                ]),
                answer: Answer::Score {
                    score: levels[1] + 2.0 * levels[2],
                    confidence: 0.3,
                    probabilities: BTreeMap::from([
                        ("0".into(), levels[0]),
                        ("1".into(), levels[1]),
                        ("2".into(), levels[2]),
                    ]),
                },
            }],
        }
    }

    fn value(result: &Structured) -> Value {
        serde_json::to_value(result).unwrap()
    }

    #[test]
    fn findings_come_new_and_strongest_first_with_their_fingerprints_as_ids() {
        let mut baselined = found(Strength::Review, 9.0, "old");
        baselined.baselined = true;
        let report = report(
            vec![
                found(Strength::Consider, 5.0, "c1"),
                baselined,
                found(Strength::Note, 7.0, "n1"),
                found(Strength::Review, 1.0, "r1"),
                found(Strength::Review, 2.0, "r2"),
            ],
            Vec::new(),
        );
        let result = value(&structured(&report, &all(), 1));
        let ids: Vec<&str> = result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["r2", "r1", "c1", "old"], "notes only when asked");
        assert_eq!(result["total_findings"], 4);
        assert_eq!(result["findings"][3]["baselined"], true);
        assert!(result["findings"][0].get("baselined").is_none());
        assert_eq!(result["findings"][0]["end_line"], 20);
        let two = Selection::new(&json!({"max_findings": 2}), None, true).unwrap();
        let result = value(&structured(&report, &two, 1));
        assert_eq!(result["findings"].as_array().unwrap().len(), 2);
        assert_eq!(
            result["total_findings"], 5,
            "the note counts when asked for"
        );
        let elsewhere = Selection::new(&json!({}), Some("tests"), true).unwrap();
        assert_eq!(structured(&report, &elsewhere, 1).total_findings, 0);
    }

    #[test]
    fn findings_that_fail_the_gate_come_first_and_say_how_the_gate_counted_them() {
        let failing = Finding {
            rank: 0.5,
            ..crate::tests::finding_of("maintainability/function-simplification", Strength::Review)
        };
        let report = report(
            vec![
                crate::tests::finding_of("maintainability/shared-logic", Strength::Review),
                failing,
            ],
            Vec::new(),
        );
        let result = value(&structured(&report, &all(), 1));
        let gates: Vec<&Value> = result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| &f["gate"])
            .collect();
        assert_eq!(
            gates,
            [&json!("fails"), &json!("measuring")],
            "a failure first, whatever its rank"
        );
        assert_eq!(result["gate"]["passed"], false);
    }

    #[test]
    fn verify_items_come_highest_concern_first_with_their_questions_answered() {
        let report = report(
            Vec::new(),
            vec![
                open_unit("low", 1, 0.40, [0.5, 0.2, 0.3]),
                open_unit("high", 20, 0.72, [0.38, 0.2, 0.42]),
            ],
        );
        let result = structured(&report, &all(), 0);
        let json = value(&result);
        assert_eq!(json["total_verify"], 2);
        let first = &json["verify"][0];
        assert_eq!(first["unit"], "high");
        assert_eq!(first["id"], "high-id");
        assert_eq!(first["rule"], "maintainability/function-simplification");
        assert_eq!(
            (first["line"].as_u64(), first["end_line"].as_u64()),
            (Some(20), Some(29))
        );
        let question = &first["questions"][0];
        assert_eq!(question["evidence"], json!(["functions[0].source"]));
        assert_eq!(
            question["answers"],
            json!([
                {"option": "2", "meaning": "Yes. It mixes separate jobs.", "probability": 0.42},
                {"option": "0", "meaning": "No. It reads as one job.", "probability": 0.38},
                {"option": "1", "meaning": "Slightly. One block could be named.", "probability": 0.2},
            ])
        );
        let text = result.verify_text().unwrap();
        assert!(
            text.starts_with("Verify (2 undecided; read the code and decide, they never fail the gate):\n  src/a.rs:20 [maintainability/function-simplification] high (concern 72%)\n    Would splitting the function in `functions[0].source` help? Yes 42% · No 38% · Slightly 20%\n"),
            "{text}"
        );
        let one = Selection::new(&json!({"max_verify": 1}), None, false).unwrap();
        let capped = structured(&report, &one, 0);
        assert!(
            capped
                .verify_text()
                .unwrap()
                .starts_with("Verify (2 undecided, top 1;")
        );
        let none = Selection::new(&json!({"max_verify": 0}), None, false).unwrap();
        assert!(structured(&report, &none, 0).verify_text().is_none());
    }

    #[test]
    fn a_report_without_open_questions_lists_their_labels() {
        let mut old = open_unit("old", 3, 0.0, [0.5, 0.0, 0.5]);
        old.open.clear();
        old.fingerprint.clear();
        old.locations.clear();
        let report = report(Vec::new(), vec![old]);
        let json = value(&structured(&report, &all(), 0));
        let item = &json["verify"][0];
        assert!(item.get("id").is_none());
        assert_eq!(item["end_line"], 3);
        assert_eq!(item["questions"], json!([{"question": "splitting"}]));
    }

    #[test]
    fn a_noul_is_answered_yes_or_no_and_unlikely_answers_are_left_out() {
        let open = OpenQuestion {
            id: "logs_secret".into(),
            pass: Pass::Trace,
            text: "Does `function.source` log a secret?".into(),
            evidence: vec!["function.source".into()],
            options: BTreeMap::from([
                ("true".into(), "It logs a token.".into()),
                ("false".into(), "It logs no secret.".into()),
            ]),
            answer: Answer::Noul { noul: 0.6 },
        };
        let view = QuestionView::new(&open);
        let labels: Vec<String> = view
            .answers
            .iter()
            .map(|a| format!("{} {}", a.label(), a.probability))
            .collect();
        assert_eq!(labels, ["yes 0.6", "no 0.4"]);
        let choice = OpenQuestion {
            options: BTreeMap::new(),
            answer: Answer::Choice {
                choice: "own".into(),
                confidence: 0.5,
                probabilities: BTreeMap::from([
                    ("own".into(), 0.52),
                    ("raw".into(), 0.46),
                    ("typed".into(), 0.02),
                ]),
            },
            ..open
        };
        let view = QuestionView::new(&choice);
        let options: Vec<&str> = view.answers.iter().map(AnswerView::label).collect();
        assert_eq!(options, ["own", "raw"]);
    }

    #[test]
    fn errors_say_why_files_failed_and_skipped_files_say_why() {
        let mut report = report(Vec::new(), Vec::new());
        let mut failed = report.files[0].clone();
        failed.status = Status::Error;
        failed.error = Some("No API key configured".into());
        let mut skipped = failed.clone();
        skipped.status = Status::Skipped;
        skipped.error = Some("Syntax error at line 3".into());
        report.files.extend([failed.clone(), failed, skipped]);
        report.errors.push("TypeSafe HTTP 402".into());
        report.update_status();
        let json = value(&structured(&report, &all(), 2));
        assert_eq!(
            json["errors"],
            json!(["TypeSafe HTTP 402", "Failed 2: No API key configured"])
        );
        assert_eq!(
            json["skipped"],
            json!(["Skipped 1: Syntax error at line 3"])
        );
        assert_eq!(
            (json["complete"].as_bool(), json["exit_code"].as_u64()),
            (Some(false), Some(2))
        );
    }

    #[test]
    fn arguments_out_of_bounds_are_refused() {
        for arguments in [
            json!({"max_findings": 0}),
            json!({"max_findings": 201}),
            json!({"max_findings": 2.5}),
            json!({"max_verify": -1}),
            json!({"max_verify": "5"}),
        ] {
            assert!(
                Selection::new(&arguments, None, false).is_err(),
                "{arguments}"
            );
        }
        assert!(Selection::new(&json!({"max_verify": 0}), None, false).is_ok());
    }

    #[test]
    fn every_result_conforms_to_the_declared_schema() {
        let mut baselined = found(Strength::Review, 9.0, "old");
        baselined.baselined = true;
        baselined.suppressed = Some("the protocol fixes it".into());
        baselined.category = Some("CWE-89 SQL injection".into());
        let report = report(
            vec![baselined, found(Strength::Note, 1.0, "n")],
            vec![open_unit("u", 1, 0.5, [0.4, 0.2, 0.4])],
        );
        let schema = &super::super::tools::list()[0]["outputSchema"];
        let notes = Selection::new(&json!({}), None, true).unwrap();
        super::super::tools::assert_conforms(&value(&structured(&report, &notes, 1)), schema);
        let mut dry = report.clone();
        dry.dry_run = true;
        dry.gate = None;
        super::super::tools::assert_conforms(&value(&structured(&dry, &all(), 0)), schema);
    }
}
