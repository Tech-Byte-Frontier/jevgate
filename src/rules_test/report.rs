//! What `rules test` found: the JSON `--format json` prints, and the table
//! for people.
use super::Case;
use crate::{
    custom::{Expected, Kind, Text},
    options::{CheckArgs, RulesFormat},
    output,
    schema::Strength,
    token_budget::TokenBudget,
};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, io::Write, path::PathBuf};

/// An example whose answer is this close to its threshold is marked: it is
/// the first to flip when a model's answers move.
const MARGIN: f64 = 0.05;

/// What an example's answers say, as the report names it.
const RIGHT: &str = "right";
const WRONG: &str = "wrong";
/// Not asked or not answered; `error` says why.
const ERROR: &str = "error";
/// A dry run's example that can be asked.
const READY: &str = "ready";

/// The requests a dry run would send.
#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Planned {
    /// Requests the examples need.
    pub requests: usize,
    /// Of those, the ones the cache answers.
    pub cached: usize,
    /// Estimated input tokens of the rest, each with only the questions the
    /// cache does not answer.
    pub new_input_tokens: u64,
}

impl Planned {
    /// What `requests` take beyond the answers the cache holds.
    pub(super) fn of(
        requests: &[crate::units::Planned],
        budget: &TokenBudget,
        unanswered: &dyn Fn(&Value) -> Option<Value>,
    ) -> Self {
        let mut counts = Self {
            requests: requests.len(),
            ..Self::default()
        };
        for planned in requests {
            match unanswered(&planned.request) {
                None => counts.cached += 1,
                Some(sent) => counts.new_input_tokens += budget.request_tokens(&sent) as u64,
            }
        }
        counts
    }
}

/// What asking cost, or for a dry run, what it would take.
#[derive(Default)]
pub(super) struct Usage {
    api_requests: u32,
    paid_input_tokens: u64,
    /// Dollars, priced by the model that answered each request; none when
    /// unknown.
    usd: Option<f64>,
    planned: Option<Planned>,
}

impl Usage {
    pub(super) fn asked(api_requests: u32, paid: &crate::requests::Usage) -> Self {
        Self {
            api_requests,
            paid_input_tokens: paid.input_tokens,
            usd: paid.usd(),
            planned: None,
        }
    }

    pub(super) fn planned(planned: Planned) -> Self {
        Self {
            planned: Some(planned),
            ..Self::default()
        }
    }
}

/// The report `--format json` prints.
#[derive(Serialize)]
pub(super) struct Report {
    dry_run: bool,
    /// Every example was asked and answered; in a dry run, every one can be.
    complete: bool,
    /// Complete, and every example right; none in a dry run.
    passed: Option<bool>,
    requested_model: String,
    /// The provider of the key the examples are asked with.
    provider: &'static str,
    /// The models that answered.
    models: BTreeSet<String>,
    api_requests: u32,
    paid_input_tokens: u64,
    /// Dollars, priced as `check` prices them: by the model that answered, or
    /// for a dry run by the requested one; null when unknown.
    estimated_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    planned: Option<Planned>,
    questions: Vec<Tested>,
    /// Selected questions without examples.
    untested: Vec<String>,
}

/// A question and its examples' results.
#[derive(Serialize)]
struct Tested {
    rule: String,
    unit: Kind,
    level: Strength,
    threshold: f64,
    version: String,
    /// The file that defines it.
    source: PathBuf,
    examples: Vec<Judged>,
    /// What it asks about and when it fails, for the table.
    #[serde(skip)]
    summary: String,
}

/// One example's result.
#[derive(Serialize)]
struct Judged {
    expected: Expected,
    /// Its place among the question's examples of its kind, from 1.
    number: usize,
    /// The file it stands for.
    path: PathBuf,
    /// The file its text comes from, when it is not written inline.
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<PathBuf>,
    /// `right`, `wrong` or `error`; `ready` or `error` in a dry run.
    result: &'static str,
    /// The unit whose answer leans most to yes, as a finding names it; none
    /// for a whole file.
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    /// That unit's probability of yes.
    #[serde(skip_serializing_if = "Option::is_none")]
    yes: Option<f64>,
    /// Whether a check reports the example: one of its units is a finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    found: Option<bool>,
    /// Whether that probability is within `MARGIN` of the threshold.
    close: bool,
    /// Whether every answer came from the cache.
    cached: bool,
    /// Every unit's answer.
    units: Vec<UnitAnswer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct UnitAnswer {
    unit: String,
    yes: f64,
    found: bool,
}

impl Report {
    /// The report of `cases`, in order, with the questions `untested` left
    /// out for want of examples.
    pub(super) fn new(
        cases: &[Case],
        untested: Vec<String>,
        args: &CheckArgs,
        usage: Usage,
    ) -> Self {
        let dry_run = usage.planned.is_some();
        let mut questions: Vec<Tested> = Vec::new();
        for case in cases {
            let judged = Judged::of(case, dry_run);
            match questions.last_mut() {
                Some(tested) if tested.rule == case.question.rule => tested.examples.push(judged),
                _ => questions.push(Tested::of(case.question, judged)),
            }
        }
        let results = || questions.iter().flat_map(|q| &q.examples).map(|e| e.result);
        let complete = !results().any(|result| result == ERROR);
        let right = !results().any(|result| result == WRONG);
        Self {
            dry_run,
            complete,
            passed: (!dry_run).then_some(complete && right),
            requested_model: args.model().to_string(),
            provider: args.provider.name(),
            models: cases
                .iter()
                .flat_map(|c| c.models.iter().cloned())
                .collect(),
            api_requests: usage.api_requests,
            paid_input_tokens: usage.paid_input_tokens,
            estimated_usd: match &usage.planned {
                Some(planned) => crate::model::usd(args.model(), planned.new_input_tokens),
                None => usage.usd,
            },
            planned: usage.planned,
            questions,
            untested,
        }
    }

    /// As `check`: 2 when incomplete, 1 when a question got an example
    /// wrong, else 0.
    pub(super) fn exit_code(&self) -> u8 {
        match self.passed {
            _ if !self.complete => 2,
            Some(false) => 1,
            _ => 0,
        }
    }

    /// Print the report to stdout; a reader that closes the pipe early
    /// leaves the exit code as it is.
    pub(super) fn emit(&self, format: RulesFormat) -> Result<()> {
        let mut out = std::io::stdout().lock();
        let written = match format {
            RulesFormat::Json => serde_json::to_writer_pretty(&mut out, self)
                .map_err(anyhow::Error::from)
                .and_then(|()| Ok(writeln!(out)?)),
            RulesFormat::Table => self.table(&mut out),
        };
        match written {
            Err(error) if output::broken_pipe(&error) => Ok(()),
            other => other,
        }
    }

    /// The headline, then each question with a line per example, then the
    /// questions without examples.
    pub(super) fn table(&self, out: &mut impl Write) -> Result<()> {
        writeln!(out, "{}", self.headline())?;
        for tested in &self.questions {
            writeln!(
                out,
                "\n{} ({}): {}",
                tested.rule,
                tested.summary,
                tested.status()
            )?;
            for example in &tested.examples {
                writeln!(out, "  {}", example.line(tested.threshold))?;
            }
        }
        if !self.untested.is_empty() {
            writeln!(out, "\nWithout examples: {}.", self.untested.join(", "))?;
        }
        Ok(())
    }

    /// The verdict, what was tested and what it cost, on one line.
    fn headline(&self) -> String {
        if self.questions.is_empty() {
            return "JevGate: rules test · no custom question has examples".into();
        }
        let examples = || self.questions.iter().flat_map(|q| &q.examples);
        let count = |result: &str| examples().filter(|e| e.result == result).count();
        let total = examples().count();
        let questions = output::count(self.questions.len(), "question");
        let of =
            |n: usize, what: &str| format!("{n} of {} {what}", output::count(total, "example"));
        if let Some(planned) = &self.planned {
            let ready = match count(ERROR) {
                0 => output::count(total, "example"),
                errors => of(errors, "cannot be asked"),
            };
            return format!(
                "JevGate: rules test dry run · {ready} · {questions} · {} requests, {} answered by the cache · ~{} new input tokens{}",
                planned.requests,
                planned.cached,
                planned.new_input_tokens,
                output::cost(self.estimated_usd)
            );
        }
        let verdict = match (count(ERROR), count(WRONG)) {
            (0, 0) => format!("all {} right", output::count(total, "example")),
            (0, wrong) => of(wrong, "wrong"),
            (errors, _) => format!("incomplete: {}", of(errors, "not answered")),
        };
        let models: Vec<&str> = self.models.iter().map(String::as_str).collect();
        let models = match models.as_slice() {
            [] => String::new(),
            models => format!(" · answered by {}", models.join(", ")),
        };
        let via =
            crate::provider::Provider::named(self.provider).map_or(String::new(), output::through);
        format!(
            "JevGate: rules test · {verdict} · {questions} · {} API requests{via} · {} input tokens{}{models}",
            self.api_requests,
            self.paid_input_tokens,
            output::cost(self.estimated_usd)
        )
    }
}

impl Tested {
    fn of(question: &crate::custom::Question, first: Judged) -> Self {
        Self {
            rule: question.rule.clone(),
            unit: question.unit,
            level: question.level,
            threshold: question.threshold,
            version: question.version.clone(),
            source: question.source.clone(),
            examples: vec![first],
            summary: question.summary(),
        }
    }

    /// How many of its examples it got right, or which it did not.
    fn status(&self) -> String {
        let total = self.examples.len();
        let count = |result: &str| self.examples.iter().filter(|e| e.result == result).count();
        let of =
            |n: usize, what: &str| format!("{n} of {} {what}", output::count(total, "example"));
        match (count(ERROR), count(WRONG), count(RIGHT)) {
            (0, 0, 0) => output::count(total, "example"),
            (0, 0, _) => format!("all {} right", output::count(total, "example")),
            (0, wrong, _) => of(wrong, "wrong"),
            (errors, _, _) => of(errors, "not answered"),
        }
    }
}

impl Judged {
    fn of(case: &Case, dry_run: bool) -> Self {
        let top = case.top();
        let found = case.found();
        let result = match () {
            _ if case.error.is_some() => ERROR,
            _ if dry_run => READY,
            _ if !case.answered() => ERROR,
            _ if found == (case.example.expected == Expected::Failing) => RIGHT,
            _ => WRONG,
        };
        let judged = matches!(result, RIGHT | WRONG);
        let threshold = case.question.threshold;
        Self {
            expected: case.example.expected,
            number: case.example.number,
            path: case.example.path.clone(),
            file: match &case.example.text {
                Text::File(file) => Some(file.clone()),
                Text::Inline(_) => None,
            },
            result,
            // A whole file is named by its path.
            unit: top
                .filter(|_| case.question.unit != Kind::File)
                .map(|(unit, _)| unit.subject.clone()),
            yes: top.map(|(_, (yes, _))| yes),
            found: judged.then_some(found),
            close: top.is_some_and(|(_, (yes, _))| (yes - threshold).abs() < MARGIN),
            cached: judged && case.cached,
            units: case
                .units
                .iter()
                .filter_map(|unit| {
                    let &(yes, found) = case.answers.get(&unit.id)?;
                    Some(UnitAnswer {
                        unit: unit.subject.clone(),
                        yes,
                        found,
                    })
                })
                .collect(),
            error: case.error.clone(),
        }
    }

    /// `ok     failing 1  yes 0.93  src/api/orders.ts: `createOrder``, and
    /// for an example the question gets wrong, what a check would do.
    fn line(&self, threshold: f64) -> String {
        let status = match self.result {
            RIGHT => "ok",
            other => other,
        };
        let label = format!("{} {}", self.expected.name(), self.number);
        let yes = self
            .yes
            .map_or(String::new(), |yes| format!("yes {yes:.2}"));
        let place = match self.unit.as_deref() {
            Some(unit) => format!("{}: {}", self.path.display(), lowercase_first(unit)),
            None => self.path.display().to_string(),
        };
        let note = match (self.result, self.expected) {
            (WRONG, Expected::Failing) => format!(" (a check misses it below {threshold:.2})"),
            (WRONG, Expected::Passing) => {
                format!(" (a check reports it at {threshold:.2} or more)")
            }
            _ if self.close => format!(" (within {MARGIN:.2} of {threshold:.2})"),
            _ => String::new(),
        };
        let error = self
            .error
            .as_deref()
            .map_or(String::new(), |error| format!(": {error}"));
        format!("{status:6} {label:10} {yes:8}  {place}{note}{error}")
    }
}

/// `A comment in `total`` as it reads after a path: `a comment in `total``.
fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or(String::new(), |first| {
        first.to_lowercase().chain(chars).collect()
    })
}
