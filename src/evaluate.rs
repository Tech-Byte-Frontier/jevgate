use super::{
    inventory::Input,
    options::CheckArgs,
    schema::{self, FileResult, Report, Status},
    storage::Store,
    token_budget::{Limits, TokenBudget},
    transport::Evaluator,
};
use crate::config::ConfigContext;
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct Session<'a> {
    pub args: &'a CheckArgs,
    pub context: &'a ConfigContext,
    pub store: &'a Store,
    pub evaluator: &'a mut dyn Evaluator,
    pub requests: u32,
    /// What this invocation's requests were billed.
    pub paid: crate::requests::Usage,
    pub budget: TokenBudget,
    /// Uploaded bytes and billed input tokens of fresh requests, for calibration.
    pub observed: (u64, u64),
    /// The questions this invocation answered, by state.
    pub answered: crate::requests::Answered,
    /// With `--max-cost`, what the requests sent are estimated to cost.
    pub spend: Option<crate::requests::Spend>,
    /// Why nothing could be sent, such as a missing key: every request
    /// after it fails with it unsent, and `check` ends with it alone.
    pub halted: Option<anyhow::Error>,
    /// Whether stderr has said the request budget will stop this check.
    pub budget_noted: bool,
}

/// A round of follow-up requests, planned from the answers so far.
type FollowUps = fn(&crate::units::Plan, &[FileResult]) -> Vec<crate::units::Planned>;

pub struct SnapshotContext<'a> {
    pub root: &'a std::path::Path,
    pub generation: u64,
    pub requests: u32,
}

pub fn previous_judgments(report: Option<&Report>, refresh: bool) -> BTreeMap<PathBuf, FileResult> {
    if refresh {
        return BTreeMap::new();
    }
    report
        .map(|report| {
            report
                .files
                .iter()
                .map(|file| (file.path.clone(), file.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn snapshot(
    inputs: &[Input],
    _previous: &BTreeMap<PathBuf, FileResult>,
    args: &CheckArgs,
    current: SnapshotContext<'_>,
) -> Report {
    // Always recompose from cached answers, so a composition change is never
    // hidden behind a reused report.
    let files = inputs.iter().map(|input| input.result.clone()).collect();
    let mut report = empty_report(args, &current, files);
    if args.documentation() {
        report.context_load = inputs
            .iter()
            .find_map(|i| i.repository.as_ref())
            .map(|r| r.load.clone())
            .or_else(|| crate::docs::scan(current.root).ok().map(|r| r.load));
    }
    match crate::revision::Changes::of_check(current.root, args) {
        Some(Ok(changes)) => {
            report.base_revision = Some(changes.revision);
            report.deleted_files = changes.deleted;
        }
        Some(Err(error)) => report.errors.push(error.to_string()),
        None => {}
    }
    report.update_status();
    if args.dry_run {
        preview(inputs, args, current.root, &mut report);
        report.update_status();
    }
    report
}

/// A new report for this generation, before any status or evaluation.
fn empty_report(args: &CheckArgs, current: &SnapshotContext<'_>, files: Vec<FileResult>) -> Report {
    Report {
        quick: args.quick,
        base_revision: args.base.clone(),
        staged: args.now == crate::revision::Now::Index,
        pushed_revision: match &args.now {
            crate::revision::Now::Commit(commit) => Some(commit.clone()),
            _ => None,
        },
        deleted_files: Vec::new(),
        scope: if args.changed_lines() {
            schema::Scope::ChangedLines
        } else {
            schema::Scope::WholeFiles
        },
        guards: Vec::new(),
        schema_version: schema::SCHEMA_VERSION,
        command: "check".into(),
        rubric_version: schema::RUBRIC.into(),
        root: current.root.into(),
        generation: current.generation,
        watcher_pid: args.watch.then_some(std::process::id()),
        errors: Vec::new(),
        generated_at: schema::now(),
        status: String::new(),
        complete: false,
        judgments_complete: false,
        acceptance_evaluated: false,
        dry_run: args.dry_run,
        initial_requests: Vec::new(),
        requested_model: args.model().to_owned(),
        provider: args.provider.name().into(),
        api_requests: current.requests,
        concurrency: args.concurrency(),
        paid_input_tokens: 0,
        paid_output_tokens: 0,
        paid_models: BTreeMap::new(),
        unmetered_requests: 0,
        estimated_usd: Some(0.0),
        stages: BTreeMap::new(),
        settled: false,
        files,
        changes: Vec::new(),
        decision_policy: crate::catalog::policy(),
        fail_on: args.fail_on_names(),
        fail_on_rules: args.rule_fail_on_names(),
        fail_on_paths: args.path_fail_on_names(),
        fail_on_mature: args.mature_level_names(),
        gate: None,
        rules: crate::catalog::with_custom(args.questions)
            .into_iter()
            .filter(|r| args.enabled(r.key))
            .map(|r| r.id.to_string())
            .collect(),
        context_load: None,
    }
}

/// Planned first-pass requests, without credentials, network or writes; the
/// cache is read so answered questions are not counted as cost. A file whose
/// purpose the cache answers is planned as a run plans it; requests that
/// depend on new answers (an unanswered file purpose, rechecks, locating
/// blocks) are not known yet.
fn preview(inputs: &[Input], args: &CheckArgs, root: &std::path::Path, report: &mut Report) {
    let budget = &TokenBudget::load(root);
    let unanswered = |request: &serde_json::Value| crate::requests::unanswered(root, args, request);
    let limits = Limits::new(budget, &unanswered);
    let mut planned = Vec::new();
    let mut views = BTreeMap::new();
    for (owner, input) in inputs.iter().enumerate() {
        if report.files[owner].status != Status::Pending {
            continue;
        }
        match schedule(input, args, limits, &mut report.files[owner]) {
            Ok(Scheduled::None) => {}
            Ok(Scheduled::Purpose(request)) => {
                match cached_purpose(input, args, root, &request, &mut report.files[owner]) {
                    Ok(Some(view)) => {
                        views.insert(owner, view);
                    }
                    Ok(None) => {}
                    Err(error) => report.errors.push(error.to_string()),
                }
                planned.push(request);
            }
            Ok(Scheduled::Ready(view)) => {
                views.insert(owner, *view);
            }
            Err(error) => report.errors.push(error.to_string()),
        }
    }
    let plan = crate::units::plan(inputs, &views, args, budget, root);
    record_plan(&plan, report);
    planned.extend(plan.requests.into_iter().map(|p| p.request));
    count_planned(report, args, root, budget, planned);
}

/// Count `planned` requests in their stages, as priced by `budget`, and
/// keep them for `--show-requests`.
fn count_planned(
    report: &mut Report,
    args: &CheckArgs,
    root: &std::path::Path,
    budget: &TokenBudget,
    planned: impl IntoIterator<Item = serde_json::Value>,
) {
    for request in planned {
        let stage = report
            .stages
            .entry(crate::requests::stage(&request).into())
            .or_default();
        count_request(
            stage,
            &request,
            crate::requests::unanswered(root, args, &request),
            budget,
        );
        if args.show_requests {
            report
                .initial_requests
                .push(crate::requests::provider_request(&request).into_owned());
        }
    }
}

/// Count one planned request in its stage: its questions, those the cache
/// answers, and the estimated tokens of `sent`, what a run would send of it
/// (none when the cache answers every question).
fn count_request(
    stage: &mut crate::schema::StageMetrics,
    request: &serde_json::Value,
    sent: Option<serde_json::Value>,
    budget: &TokenBudget,
) {
    let questions = crate::requests::question_count(request);
    stage.planned_requests += 1;
    stage.planned_evidence_bytes += crate::requests::evidence_bytes(request);
    stage.planned_questions += questions;
    match sent {
        None => {
            stage.planned_cached += 1;
            stage.planned_cached_questions += questions;
        }
        Some(sent) => {
            stage.planned_cached_questions += questions - crate::requests::question_count(&sent);
            stage.planned_tokens += budget.request_tokens(&sent) as u64;
        }
    }
}

/// A dry run's guards: what code finds in the change within `scope`, and
/// the questions about rewritten tests a run would ask, counted like the
/// first pass.
pub fn preview_guards(
    report: &mut Report,
    args: &CheckArgs,
    context: &ConfigContext,
    scope: &[PathBuf],
) {
    let scan = crate::guards::scan(&context.root, args, &context.config, scope);
    let budget = TokenBudget::load(&context.root);
    let requests = weaker_requests(&scan.changed_tests, args, &budget);
    count_planned(
        report,
        args,
        &context.root,
        &budget,
        requests.into_iter().map(|r| r.1),
    );
    report.guards = scan.guards;
}

/// The question whether each of `tests` checks less than before, for those
/// the budget can send.
fn weaker_requests<'t>(
    tests: &'t [crate::guards::ChangedTest],
    args: &CheckArgs,
    budget: &TokenBudget,
) -> Vec<(&'t crate::guards::ChangedTest, serde_json::Value)> {
    tests
        .iter()
        .map(|test| (test, crate::units::weaker_request(args.model(), test)))
        .filter(|(_, request)| budget.fits(request))
        .collect()
}

/// A file's view as a run decides it after its purpose request, when the
/// cache answers that request; none when it does not.
fn cached_purpose(
    input: &Input,
    args: &CheckArgs,
    root: &std::path::Path,
    request: &serde_json::Value,
    file: &mut FileResult,
) -> Result<Option<crate::file_kind::View>> {
    let Some(body) = crate::requests::cached(root, args, request) else {
        return Ok(None);
    };
    crate::file_kind::record_purpose(file, request, &body)?;
    crate::file_kind::decide_after_purpose(input, args, file)
}

impl Session<'_> {
    pub fn evaluate(&mut self, inputs: &[Input], report: &mut Report) -> Result<()> {
        self.evaluator.begin_review();
        // A watcher's next snapshot looks for a key again.
        self.halted = None;
        self.publish(report)?;
        let (purpose, mut views) = self.schedule_files(inputs, report);
        if !purpose.is_empty() {
            crate::progress::phase("asking what test files hold");
            self.resolve_purposes(inputs, report, purpose, &mut views)?;
        }
        crate::progress::phase("planning");
        let mut plan =
            crate::units::plan(inputs, &views, self.args, &self.budget, &self.context.root);
        record_plan(&plan, report);
        for &owner in plan.files.keys() {
            report.files[owner].cached = true;
        }
        crate::progress::phase("first pass");
        let first: Vec<_> = plan.requests.iter().map(Task::unit).collect();
        let oversized = self.dispatch(report, first, |file, asked, body| {
            crate::units::record(file, &asked, body)
        })?;
        unsent_units(&mut plan, report, oversized);
        // Traces judge where a security concern's values come from; rechecks
        // settle uncertain units; a security check still undecided is asked
        // where its URL comes from or its output goes, and an outline its kind;
        // a long file left without a finding is asked about its parts;
        // locate follow-ups then point split findings at a block, and a
        // located value is asked what it is. Each depends on the answers
        // before it.
        for (phase, follow_up) in [
            ("checking documents", crate::units::doc_checks as FollowUps),
            ("tracing values", crate::units::traces),
            ("rechecking undecided units", crate::units::rechecks),
            ("settling security checks", crate::units::settles),
            ("asking what outlines are", crate::units::kinds),
            ("asking about file parts", crate::units::parts),
            ("locating findings", crate::units::locates),
            ("asking what values are", crate::units::value_kinds),
        ] {
            let tasks: Vec<_> = follow_up(&plan, &report.files)
                .iter()
                .map(Task::unit)
                .collect();
            if !tasks.is_empty() {
                crate::progress::phase(phase);
                let oversized = self.dispatch(report, tasks, |file, asked, body| {
                    crate::units::record(file, &asked, body)
                })?;
                unsent_units(&mut plan, report, oversized);
            }
        }
        crate::progress::phase("composing findings");
        compose_files(&plan, report);
        self.guard(&plan, report);
        self.calibrate()?;
        self.progress(report)
    }

    /// Keep the bytes per token of this session's fresh requests, so later
    /// runs estimate what fits the provider limit from real usage.
    pub fn calibrate(&mut self) -> Result<()> {
        if self.observed.1 > 0 {
            self.budget.observe(self.observed.0, self.observed.1);
            self.budget.save(self.store)?;
        }
        Ok(())
    }

    /// What the change does to the checks around the code (`guards`): what
    /// code finds, the tests whose rewritten assertions Jev reads as checking
    /// less, and the text it reads as written to steer a reviewer.
    fn guard(&mut self, plan: &crate::units::Plan, report: &mut Report) {
        let scope = crate::inventory::scope(self.args, self.context).unwrap_or_default();
        let scan = crate::guards::scan(&self.context.root, self.args, &self.context.config, &scope);
        let mut guards = scan.guards;
        guards.extend(self.weaker_tests(&scan.changed_tests, report));
        guards.extend(crate::guards::steering(plan, &report.files));
        crate::guards::sort(&mut guards);
        report.guards = guards;
    }

    /// Ask whether each test whose assertions the change rewrote now checks
    /// less; at 0.80 it is a guard. A question left unanswered leaves the
    /// run incomplete, as any other does, but for one refused as beyond the
    /// model's context, which is not asked, as a unit too large to send.
    fn weaker_tests(
        &mut self,
        tests: &[crate::guards::ChangedTest],
        report: &mut Report,
    ) -> Vec<crate::guards::Guard> {
        let asked = weaker_requests(tests, self.args, &self.budget);
        if asked.is_empty() {
            return Vec::new();
        }
        let receipts = self.queries(&asked.iter().map(|(_, r)| r).collect::<Vec<_>>());
        let mut guards = Vec::new();
        for ((test, request), receipt) in asked.iter().zip(receipts) {
            let stage = crate::requests::stage(request);
            add_metrics(
                report.stages.entry(stage.into()).or_default(),
                &receipt.metrics,
            );
            match receipt.result {
                Ok((body, ..)) => guards.extend(
                    crate::units::weaker_answer(&body)
                        .and_then(|p| crate::guards::Guard::weaker(test, p)),
                ),
                Err(error) if beyond_context(&error) => {}
                Err(error) => report.errors.push(format!(
                    "Cannot ask whether test `{}` in {} checks less than before: {error:#}",
                    test.name,
                    test.path.display()
                )),
            }
        }
        guards
    }

    /// Classify every pending file: ready with a gate view, waiting on a
    /// file-purpose request, excluded, or failed.
    fn schedule_files(
        &self,
        inputs: &[Input],
        report: &mut Report,
    ) -> (
        Vec<Task<serde_json::Value>>,
        BTreeMap<usize, crate::file_kind::View>,
    ) {
        let mut purpose = Vec::new();
        let mut views = BTreeMap::new();
        let root = &self.context.root;
        let unanswered =
            |request: &serde_json::Value| crate::requests::unanswered(root, self.args, request);
        let limits = Limits::new(&self.budget, &unanswered);
        for (owner, file) in report.files.iter_mut().enumerate() {
            if file.status != Status::Pending {
                continue;
            }
            file.judgments.clear();
            match schedule(&inputs[owner], self.args, limits, file) {
                Ok(Scheduled::None) => file.cached = false,
                Ok(Scheduled::Purpose(request)) => {
                    file.cached = true;
                    purpose.push(Task {
                        owner,
                        payload: request.clone(),
                        request,
                    });
                }
                Ok(Scheduled::Ready(view)) => {
                    views.insert(owner, *view);
                }
                Err(error) => fail(file, error),
            }
        }
        (purpose, views)
    }

    /// Ask what each ambiguous test path contains, then add its gate view.
    fn resolve_purposes(
        &mut self,
        inputs: &[Input],
        report: &mut Report,
        purpose: Vec<Task<serde_json::Value>>,
        views: &mut BTreeMap<usize, crate::file_kind::View>,
    ) -> Result<()> {
        let owners: Vec<usize> = purpose.iter().map(|t| t.owner).collect();
        let oversized = self.dispatch(report, purpose, |file, request, body| {
            crate::file_kind::record_purpose(file, &request, body)
        })?;
        // A file-purpose request too large to answer leaves its file
        // unjudged, as one the budget does not send.
        for (owner, _) in oversized {
            report.files[owner].status = Status::NeedsContext;
        }
        for owner in owners {
            let file = &mut report.files[owner];
            let answered = file
                .classification
                .as_ref()
                .is_some_and(|class| class.stage == "answered");
            if file.status == Status::Error || !answered {
                continue;
            }
            match crate::file_kind::decide_after_purpose(&inputs[owner], self.args, file) {
                Ok(Some(view)) => {
                    views.insert(owner, view);
                }
                Ok(None) => file.cached = false,
                Err(error) => fail(file, error),
            }
        }
        Ok(())
    }

    /// Send `tasks` and apply each answer to its file. The tasks the provider
    /// refused as beyond the model's context are returned rather than
    /// failed: their units were too large to send, as the budget would have
    /// found them had its estimate been exact.
    fn dispatch<T>(
        &mut self,
        report: &mut Report,
        tasks: Vec<Task<T>>,
        mut apply: impl FnMut(&mut FileResult, T, &serde_json::Value) -> Result<()>,
    ) -> Result<Vec<(usize, T)>> {
        crate::cancellation::check()?;
        let mut ready = Vec::new();
        // Shared evidence is read once while preparing this batch. Actual
        // uploads recheck it, and progress verifies it again before publication.
        let mut source_hashes = crate::requests::SourceHashes::new();
        for task in tasks {
            if report.files[task.owner].status == Status::Error {
                continue;
            }
            match crate::requests::require_current(self, &task.request, &mut source_hashes) {
                Ok(()) => ready.push(task),
                Err(error) => fail(&mut report.files[task.owner], error),
            }
        }
        let receipts = self.queries(&ready.iter().map(|t| &t.request).collect::<Vec<_>>());
        let mut spans = BTreeMap::<&str, (u64, u64)>::new();
        let mut oversized = Vec::new();
        for (task, receipt) in ready.into_iter().zip(receipts) {
            let name = crate::requests::stage(&task.request);
            let m = &receipt.metrics;
            add_metrics(report.stages.entry(name.into()).or_default(), m);
            if m.successful_requests + m.failed_attempts > 0 {
                let span = spans
                    .entry(name)
                    .or_insert((m.queue_wait_ms, m.queue_wait_ms + m.service_ms));
                span.0 = span.0.min(m.queue_wait_ms);
                span.1 = span.1.max(m.queue_wait_ms + m.service_ms);
            }
            let file = &mut report.files[task.owner];
            file.elapsed_ms += m.service_ms;
            if let Some(payload) = apply_receipt(file, receipt.result, task.payload, &mut apply) {
                oversized.push((task.owner, payload));
            }
        }
        // Concurrent stage spans overlap; service_ms is the additive request duration.
        for (name, (start, end)) in spans {
            report.stages.get_mut(name).unwrap().elapsed_ms += end - start;
        }
        self.progress(report)?;
        Ok(oversized)
    }

    fn progress(&self, report: &mut Report) -> Result<()> {
        report.api_requests = self.requests;
        report.paid_input_tokens = self.paid.input_tokens;
        report.paid_output_tokens = self.paid.output_tokens;
        report.paid_models = self.paid.models.clone();
        report.unmetered_requests = self.paid.unmetered;
        report.estimated_usd = self.paid.usd();
        self.verify_current(report);
        report.update_status();
        self.publish(report)
    }

    fn verify_current(&self, report: &mut Report) {
        // Many files share the same candidates. Read each path once per publication,
        // while comparing every recorded hash against those current bytes.
        let mut hashes = BTreeMap::<PathBuf, Option<String>>::new();
        let mut current = |path: &PathBuf, expected: &str| {
            hashes
                .entry(path.clone())
                .or_insert_with(|| {
                    self.args
                        .read(
                            &self.context.root.join(path),
                            self.args.max_context_bytes.max(self.args.max_file_bytes),
                        )
                        .ok()
                        .map(|s| schema::hash(s.as_bytes()))
                })
                .as_deref()
                == Some(expected)
        };
        for file in &mut report.files {
            // A source that was deliberately not uploaded has no judgment hash to recheck.
            if file
                .classification
                .as_ref()
                .is_some_and(|class| class.kind == "oversized")
            {
                continue;
            }
            if matches!(
                file.status,
                Status::Clear | Status::Review | Status::NeedsContext | Status::Uncertain
            ) {
                let source_current = current(&file.path, &file.source_hash);
                let context_current = file
                    .context_files
                    .iter()
                    .all(|c| current(&c.path, &c.source_hash));
                if !source_current || !context_current {
                    file.status = Status::Error;
                    file.error = Some(
                        "Source or context changed or disappeared during this batch; assessment is stale"
                            .into(),
                    );
                }
            }
        }
    }

    pub fn publish(&self, report: &Report) -> Result<()> {
        self.store.publish(report)?;
        if self.args.report {
            self.store.publish_html(report)?;
        }
        if self.args.output_format() == super::options::Format::Jsonl {
            super::output::emit(report, self.args)?;
        }
        Ok(())
    }
}

enum Scheduled {
    None,
    Purpose(serde_json::Value),
    Ready(Box<crate::file_kind::View>),
}

/// Record one answered request on its file, or its first failure; a
/// request refused as beyond the model's context is handed back instead.
fn apply_receipt<T>(
    file: &mut FileResult,
    result: Result<(serde_json::Value, u64, bool)>,
    payload: T,
    apply: &mut impl FnMut(&mut FileResult, T, &serde_json::Value) -> Result<()>,
) -> Option<T> {
    match result {
        Ok((body, timestamp, cached)) => {
            file.cached &= cached;
            file.input_tokens += body["usage"]["input_tokens"].as_u64().unwrap_or(0);
            file.output_tokens += body["usage"]["output_tokens"].as_u64().unwrap_or(0);
            file.evaluated_at = Some(timestamp);
            if file.status != Status::Error
                && let Err(error) = apply(file, payload, &body)
            {
                fail(file, error);
            }
        }
        Err(error) if beyond_context(&error) => return Some(payload),
        // Later skipped work must not overwrite this file's first failure.
        Err(error) if file.status != Status::Error => fail(file, error),
        Err(_) => {}
    }
    None
}

/// A request the provider refused as beyond the model's context. The token
/// budget estimates size from bytes, and dense text can hold more tokens per
/// byte than it assumes: a Bend 2 proof of SHA-256 and a 328-member outline
/// were refused, which failed their whole runs.
fn beyond_context(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::provider_error::ProviderError>()
        .is_some_and(|e| e.context_limit)
}

/// The units of requests refused as beyond the model's context that no
/// other request answered: they need context, as a unit the budget does not
/// send. A refused follow-up leaves its unit with the answers it has.
fn unsent_units(
    plan: &mut crate::units::Plan,
    report: &Report,
    oversized: Vec<(usize, crate::units::Asked)>,
) {
    for (owner, asked) in oversized {
        let Some(file_plan) = plan.files.get_mut(&owner) else {
            continue;
        };
        let judgments = &report.files[owner].judgments;
        for question in &asked.questions {
            if judgments.iter().any(|j| j.unit == question.unit) {
                continue;
            }
            for unit in file_plan.units.iter_mut().filter(|u| u.id == question.unit) {
                unit.presence = crate::units::Presence::NeedsContext;
            }
        }
    }
}

/// Add one request's metrics to its stage totals.
fn add_metrics(stage: &mut crate::schema::StageMetrics, m: &crate::schema::StageMetrics) {
    stage.service_ms += m.service_ms;
    stage.queue_wait_ms += m.queue_wait_ms;
    stage.successful_requests += m.successful_requests;
    stage.failed_attempts += m.failed_attempts;
    stage.retries += m.retries;
    stage.cache_hits += m.cache_hits;
    stage.cached_judgments += m.cached_judgments;
    stage.evaluated_judgments += m.evaluated_judgments;
    stage.asked_questions += m.asked_questions;
    stage.cached_questions += m.cached_questions;
    stage.input_tokens += m.input_tokens;
    stage.output_tokens += m.output_tokens;
    stage.evidence_bytes += m.evidence_bytes;
}

/// Compose each planned file's recorded judgments into dimensions and
/// findings, with the requests its units were first asked in.
fn compose_files(plan: &crate::units::Plan, report: &mut Report) {
    let mut first = BTreeMap::<usize, Vec<&crate::units::Planned>>::new();
    for planned in &plan.requests {
        first.entry(planned.owner).or_default().push(planned);
    }
    for (&owner, file_plan) in &plan.files {
        let file = &mut report.files[owner];
        if file.status == Status::Error {
            continue;
        }
        let asked = first.get(&owner).map_or(&[][..], Vec::as_slice);
        let composed = crate::units::compose::compose(file_plan, &file.judgments, asked);
        file.syntax_checked = true;
        file.dimensions = composed.dimensions;
        file.findings = composed.findings;
        file.status = composed.status;
    }
    crate::units::grouping::group_repeats(&mut report.files);
}

fn apply_classification(file: &mut FileResult, class: crate::file_kind::Classification) {
    file.contains_tests = class.kind == "tests" || !class.separated_tests.is_empty();
    file.classification = Some(class);
}

fn schedule(
    input: &Input,
    args: &CheckArgs,
    budget: Limits<'_>,
    file: &mut FileResult,
) -> Result<Scheduled> {
    if file.status != Status::Pending {
        return Ok(Scheduled::None);
    }
    let plan = match crate::file_kind::plan(input, args, budget) {
        Ok(plan) => plan,
        Err(error) => {
            // Invalid syntax cannot be located; it is reported, not judged.
            unread(file, crate::syntax::skip_reason(&error));
            return Ok(Scheduled::None);
        }
    };
    match plan {
        crate::file_kind::Plan::Skip(class) => {
            apply_classification(file, class);
            file.status = Status::NotApplicable;
            Ok(Scheduled::None)
        }
        crate::file_kind::Plan::Unsent(class) => {
            apply_classification(file, class);
            file.status = Status::NeedsContext;
            Ok(Scheduled::None)
        }
        crate::file_kind::Plan::Purpose(class, request) => {
            file.contains_tests = true;
            file.classification = Some(class);
            Ok(Scheduled::Purpose(request))
        }
        crate::file_kind::Plan::Ready(view) => {
            file.contains_tests = file.contains_tests
                || view.classification.kind == "tests"
                || !view.classification.separated_tests.is_empty();
            file.classification = Some(view.classification.clone());
            Ok(Scheduled::Ready(Box::new(view)))
        }
    }
}

struct Task<T> {
    owner: usize,
    request: serde_json::Value,
    payload: T,
}

impl Task<crate::units::Asked> {
    fn unit(planned: &crate::units::Planned) -> Self {
        Self {
            owner: planned.owner,
            request: planned.request.clone(),
            payload: planned.asked.clone(),
        }
    }
}

fn fail(file: &mut FileResult, error: anyhow::Error) {
    file.status = Status::Error;
    file.cached = false;
    file.error = Some(error.to_string());
}

/// Skip the files planning skipped, and record what syntax errors left out
/// of the others.
fn record_plan(plan: &crate::units::Plan, report: &mut Report) {
    for (owner, reason) in &plan.skipped {
        unread(&mut report.files[*owner], reason);
    }
    for (&owner, file) in &plan.files {
        report.files[owner].left_out.clone_from(&file.left_out);
    }
}

/// A file planning could not read, for `reason`: skipped, but for syntax
/// nested too deep to read, which fails the run. That file's code is read
/// by no rule, so a pull request could hide a long function beside one
/// literal of 1,001 parentheses and pass, where before the limit the run
/// crashed; the corpus's deepest file nests 405 levels.
fn unread(file: &mut FileResult, reason: &str) {
    if reason == crate::syntax::TOO_DEEP {
        fail(file, anyhow::anyhow!(reason.to_string()));
    } else {
        skip(file, reason);
    }
}

/// Unsupported or unparseable files are reported with a reason and never make a run incomplete.
fn skip(file: &mut FileResult, reason: &str) {
    file.status = Status::Skipped;
    file.cached = false;
    file.dimensions.clear();
    file.findings.clear();
    if let Some(class) = file.classification.as_mut() {
        class.reason = reason.into();
    }
    file.error = Some(reason.into());
}
