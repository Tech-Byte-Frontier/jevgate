//! `jevgate rules test`: ask each custom question about its examples, as a
//! check asks the same units, and fail when it no longer separates them: a
//! failing example a check would not find, or a passing one it would.
//! Answers are cached like a check's, so a rerun is free; a new model or a
//! reworded question changes the requests and asks again, which is the drift
//! check. The report and its two formats are in `report`.
mod report;

use crate::{
    boundary::Boundary,
    catalog,
    config::ConfigContext,
    custom::{Example, Question},
    evaluate::Session,
    options::{CheckArgs, RulesTestArgs},
    schema::Answer,
    storage::Store,
    token_budget::{Limits, TokenBudget},
    transport::Evaluator,
    units::{Asked, Planned, examples},
};
use anyhow::{Result, anyhow, ensure};
use report::{Estimate, Report, Usage};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// `rules test`: exit 0 when every example is right, 1 when a question gets
/// one wrong, 2 when one could not be asked.
pub fn run(test: &RulesTestArgs, context: &ConfigContext) -> Result<u8> {
    let args = arguments(test, context)?;
    crate::cancellation::install()?;
    let credentials = crate::check::credential_path(&args, context);
    let mut client =
        crate::transport::Client::new(&credentials, args.env_file.is_some(), args.provider)?;
    let report = examine(&test.rules, &args, context, &mut client)?;
    report.emit(test.format)?;
    Ok(report.exit_code())
}

/// `check`'s arguments with the configuration's model and budgets, narrowed
/// by the test's flags.
fn arguments(test: &RulesTestArgs, context: &ConfigContext) -> Result<CheckArgs> {
    let mut args = CheckArgs::defaults();
    args.model = test.model.clone();
    args.refresh = test.refresh;
    args.cache_only = test.cache_only;
    args.max_requests = test.max_requests;
    args.env_file = test.env_file.clone();
    args.dry_run = test.dry_run;
    context.configure(&mut args)?;
    args.provider = crate::check::planned_provider(&args, context);
    Ok(args)
}

/// Ask the examples of the questions `names` select, `evaluator` answering
/// what the cache does not; with `--dry-run`, only count what that takes.
fn examine(
    names: &[String],
    args: &CheckArgs,
    context: &ConfigContext,
    evaluator: &mut dyn Evaluator,
) -> Result<Report> {
    let (questions, untested) = selected(names, context.questions)?;
    let budget = TokenBudget::load(&context.root);
    let unanswered = |request: &Value| crate::requests::unanswered(&context.root, args, request);
    let limits = Limits::new(&budget, &unanswered);
    let (mut cases, requests) = plan(&questions, args, context, limits)?;
    let usage = if args.dry_run {
        Usage::planned(Estimate::of(&requests, &budget, &unanswered))
    } else {
        ask(&mut cases, &requests, (args, context), (budget, evaluator))?
    };
    Ok(Report::new(&cases, untested, args, usage))
}

/// The questions to test, and those left untested for want of examples:
/// every question by default; the ones `names` select otherwise. A question
/// named by its own ID must have examples.
fn selected(
    names: &[String],
    questions: &'static [Question],
) -> Result<(Vec<&'static Question>, Vec<String>)> {
    let mut chosen = BTreeSet::new();
    if names.is_empty() {
        chosen.extend(questions.iter().map(|q| q.rule.as_str()));
    }
    let rules: Vec<catalog::Rule> = questions.iter().map(Question::rule).collect();
    for name in names {
        let keys = catalog::select_in(&rules, name).ok_or_else(|| {
            anyhow!("{name} names no custom question; `jevgate rules` lists them")
        })?;
        chosen.extend(keys);
    }
    let mut tested = Vec::new();
    let mut untested = Vec::new();
    for question in questions
        .iter()
        .filter(|q| chosen.contains(q.rule.as_str()))
    {
        if !question.examples.is_empty() {
            tested.push(question);
            continue;
        }
        ensure!(
            !names.contains(&question.rule),
            "{} has no examples: add [[question.failing]] and [[question.passing]] tables, or [[failing]] and [[passing]] in its question file",
            question.rule
        );
        untested.push(question.rule.clone());
    }
    Ok((tested, untested))
}

/// Each example of `questions`, read and planned: its units, and the
/// requests that ask them, planned for it (`Planned::owner` is its place in
/// the cases). An example that cannot be read or holds no unit keeps why.
fn plan(
    questions: &[&'static Question],
    args: &CheckArgs,
    context: &ConfigContext,
    limits: Limits<'_>,
) -> Result<(Vec<Case>, Vec<Planned>)> {
    let boundary = Boundary::new(&context.config)?;
    let mut cases = Vec::new();
    let mut requests = Vec::new();
    for &question in questions {
        for example in &question.examples {
            let mut case = Case::new(question, example);
            let owner = cases.len();
            let planned = example
                .read(&context.root, &boundary, args.max_file_bytes)
                .and_then(|text| {
                    let file = examples::ExampleFile {
                        owner,
                        path: &example.path,
                        text: &text,
                    };
                    examples::plan(question, &file, (args.model(), limits))
                });
            match planned {
                Ok(plan) => {
                    case.units = plan.units;
                    requests.extend(plan.requests);
                }
                Err(error) => case.fail(&error),
            }
            cases.push(case);
        }
    }
    Ok((cases, requests))
}

/// Ask `requests` through the answer cache and `evaluator`, as a check
/// does, and record each answer on its example.
fn ask(
    cases: &mut [Case],
    requests: &[Planned],
    (args, context): (&CheckArgs, &ConfigContext),
    (budget, evaluator): (TokenBudget, &mut dyn Evaluator),
) -> Result<Usage> {
    if requests.is_empty() {
        return Ok(Usage::default());
    }
    let store = Store::open(&context.root)?;
    let mut session = Session {
        args,
        context,
        store: &store,
        evaluator,
        requests: 0,
        paid: Default::default(),
        budget,
        observed: (0, 0),
        answered: Default::default(),
    };
    let bodies: Vec<&Value> = requests.iter().map(|p| &p.request).collect();
    for (planned, receipt) in requests.iter().zip(session.queries(&bodies)) {
        let case = &mut cases[planned.owner];
        match receipt.result {
            Ok((body, _, cached)) => case.record(&planned.asked, &body, cached),
            Err(error) => case.fail(&error),
        }
    }
    session.calibrate()?;
    Ok(Usage::asked(session.requests, &session.paid))
}

/// One example of one question: what it asks about and what came back.
struct Case {
    question: &'static Question,
    example: &'static Example,
    /// Its units, in the order the example holds them.
    units: Vec<examples::ExampleUnit>,
    /// The probability of yes and whether a check reports it, by unit id.
    answers: BTreeMap<String, (f64, bool)>,
    /// The models that answered.
    models: BTreeSet<String>,
    /// Whether every answer came from the cache.
    cached: bool,
    /// Why it could not be asked or answered.
    error: Option<String>,
}

impl Case {
    fn new(question: &'static Question, example: &'static Example) -> Self {
        Self {
            question,
            example,
            units: Vec::new(),
            answers: BTreeMap::new(),
            models: BTreeSet::new(),
            cached: true,
            error: None,
        }
    }

    /// The answers of one request about its units.
    fn record(&mut self, asked: &Asked, body: &Value, cached: bool) {
        self.cached &= cached;
        self.models
            .extend(body["model"].as_str().map(str::to_string));
        for question in &asked.questions {
            let answer = serde_json::from_value::<Answer>(body["answers"][&question.key].clone());
            match answer
                .ok()
                .and_then(|a| examples::verdict(self.question, &a))
            {
                Some(verdict) => {
                    self.answers.insert(question.unit.clone(), verdict);
                }
                None => self.fail(&anyhow!("No valid answer for {}", question.key)),
            }
        }
    }

    /// Keep the first reason it could not be judged.
    fn fail(&mut self, error: &anyhow::Error) {
        self.error.get_or_insert_with(|| format!("{error:#}"));
    }

    /// Whether every unit has an answer and nothing failed.
    fn answered(&self) -> bool {
        self.error.is_none() && self.units.iter().all(|u| self.answers.contains_key(&u.id))
    }

    /// The unit whose answer leans most to yes, with that answer.
    fn top(&self) -> Option<(&examples::ExampleUnit, (f64, bool))> {
        self.units
            .iter()
            .filter_map(|unit| self.answers.get(&unit.id).map(|answer| (unit, *answer)))
            .max_by(|a, b| a.1.0.total_cmp(&b.1.0))
    }

    /// Whether a check would report the example: one of its units is a
    /// finding.
    fn found(&self) -> bool {
        self.answers.values().any(|(_, finding)| *finding)
    }
}

#[cfg(test)]
mod tests;
