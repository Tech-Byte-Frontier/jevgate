use super::{
    inventory::Input,
    options::CheckArgs,
    schema::{self, FileResult, Report, Status},
    storage::Store,
    token_budget::TokenBudget,
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
    pub paid_input_tokens: u64,
    pub paid_output_tokens: u64,
    pub budget: TokenBudget,
    /// Uploaded bytes and billed input tokens of fresh requests, for calibration.
    pub observed: (u64, u64),
}

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
    if let Some(base) = &args.base {
        match crate::revision::Changes::load(current.root, base) {
            Ok(changes) => {
                report.base_revision = Some(changes.revision);
                report.deleted_files = changes.deleted;
            }
            Err(error) => report.errors.push(error.to_string()),
        }
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
        deleted_files: Vec::new(),
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
        api_requests: current.requests,
        concurrency: args.concurrency,
        paid_input_tokens: 0,
        paid_output_tokens: 0,
        stages: BTreeMap::new(),
        settled: false,
        files,
        changes: Vec::new(),
        decision_policy: crate::catalog::policy(),
        fail_on: args.fail_on_names(),
        fail_on_rules: args.rule_fail_on_names(),
        fail_on_paths: args.path_fail_on_names(),
        gate: None,
        rules: crate::catalog::rules()
            .into_iter()
            .filter(|r| args.enabled(r.key))
            .map(|r| r.id.to_string())
            .collect(),
        context_load: None,
    }
}

/// Planned first-pass requests, without credentials, network or writes; the
/// cache is read so answered requests are not counted as cost. Requests that
/// depend on answers (after file purpose, rechecks, locating blocks) are not
/// known yet.
fn preview(inputs: &[Input], args: &CheckArgs, root: &std::path::Path, report: &mut Report) {
    let budget = &TokenBudget::load(root);
    let mut planned = Vec::new();
    let mut views = BTreeMap::new();
    for (owner, input) in inputs.iter().enumerate() {
        if report.files[owner].status != Status::Pending {
            continue;
        }
        match schedule(input, args, budget, &mut report.files[owner]) {
            Ok(Scheduled::None) => {}
            Ok(Scheduled::Purpose(request)) => planned.push(request),
            Ok(Scheduled::Ready(view)) => {
                views.insert(owner, *view);
            }
            Err(error) => report.errors.push(error.to_string()),
        }
    }
    let plan = crate::units::plan(inputs, &views, args, budget);
    for (owner, reason) in &plan.skipped {
        skip(&mut report.files[*owner], reason);
    }
    planned.extend(plan.requests.into_iter().map(|p| p.request));
    for request in planned {
        let stage = report
            .stages
            .entry(crate::requests::stage(&request).into())
            .or_default();
        stage.planned_requests += 1;
        stage.planned_evidence_bytes += crate::requests::evidence_bytes(&request);
        if crate::requests::answered(root, args, &request) {
            stage.planned_cached += 1;
        } else {
            stage.planned_tokens += budget.request_tokens(&request) as u64;
        }
        if args.show_requests {
            report.initial_requests.push(
                crate::requests::provider_request_for(&request, args.provider()).into_owned(),
            );
        }
    }
}

impl Session<'_> {
    pub fn evaluate(&mut self, inputs: &[Input], report: &mut Report) -> Result<()> {
        self.evaluator.begin_review();
        self.publish(report)?;
        let (purpose, mut views) = self.schedule_files(inputs, report);
        if !purpose.is_empty() {
            self.resolve_purposes(inputs, report, purpose, &mut views)?;
        }
        let plan = crate::units::plan(inputs, &views, self.args, &self.budget);
        for (owner, reason) in &plan.skipped {
            skip(&mut report.files[*owner], reason);
        }
        for &owner in plan.files.keys() {
            report.files[owner].cached = true;
        }
        let first: Vec<_> = plan.requests.iter().map(Task::unit).collect();
        self.dispatch(report, first, |file, asked, body| {
            crate::units::record(file, &asked, body)
        })?;
        // Traces judge where a security concern's values come from; rechecks
        // settle uncertain units; a security check still undecided is asked
        // where its URL comes from or its output goes, and an outline its kind;
        // locate follow-ups then point split findings at a block. Each depends
        // on the answers before it.
        for follow_up in [
            crate::units::doc_checks,
            crate::units::traces,
            crate::units::rechecks,
            crate::units::settles,
            crate::units::kinds,
            crate::units::locates,
        ] {
            let tasks: Vec<_> = follow_up(&plan, &report.files)
                .iter()
                .map(Task::unit)
                .collect();
            if !tasks.is_empty() {
                self.dispatch(report, tasks, |file, asked, body| {
                    crate::units::record(file, &asked, body)
                })?;
            }
        }
        compose_files(&plan, report);
        if self.observed.1 > 0 {
            self.budget.observe(self.observed.0, self.observed.1);
            self.budget.save(self.store)?;
        }
        self.progress(report)
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
        for (owner, file) in report.files.iter_mut().enumerate() {
            if file.status != Status::Pending {
                continue;
            }
            file.judgments.clear();
            match schedule(&inputs[owner], self.args, &self.budget, file) {
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
        self.dispatch(report, purpose, |file, request, body| {
            crate::file_kind::record_purpose(file, &request, body)
        })?;
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

    fn dispatch<T>(
        &mut self,
        report: &mut Report,
        tasks: Vec<Task<T>>,
        mut apply: impl FnMut(&mut FileResult, T, &serde_json::Value) -> Result<()>,
    ) -> Result<()> {
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
            apply_receipt(file, receipt.result, task.payload, &mut apply);
        }
        // Concurrent stage spans overlap; service_ms is the additive request duration.
        for (name, (start, end)) in spans {
            report.stages.get_mut(name).unwrap().elapsed_ms += end - start;
        }
        self.progress(report)
    }

    fn progress(&self, report: &mut Report) -> Result<()> {
        report.api_requests = self.requests;
        report.paid_input_tokens = self.paid_input_tokens;
        report.paid_output_tokens = self.paid_output_tokens;
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
                    super::inventory::read_source(
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

/// Record one answered request on its file, or its first failure.
fn apply_receipt<T>(
    file: &mut FileResult,
    result: Result<(serde_json::Value, u64, bool)>,
    payload: T,
    apply: &mut impl FnMut(&mut FileResult, T, &serde_json::Value) -> Result<()>,
) {
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
        // Later skipped work must not overwrite this file's first failure.
        Err(error) if file.status != Status::Error => fail(file, error),
        Err(_) => {}
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
    stage.input_tokens += m.input_tokens;
    stage.output_tokens += m.output_tokens;
    stage.evidence_bytes += m.evidence_bytes;
}

/// Compose each planned file's recorded judgments into dimensions and findings.
fn compose_files(plan: &crate::units::Plan, report: &mut Report) {
    for (&owner, file_plan) in &plan.files {
        let file = &mut report.files[owner];
        if file.status == Status::Error {
            continue;
        }
        let composed = crate::units::compose::compose(file_plan, &file.judgments);
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
    budget: &TokenBudget,
    file: &mut FileResult,
) -> Result<Scheduled> {
    if file.status != Status::Pending {
        return Ok(Scheduled::None);
    }
    let plan = match crate::file_kind::plan(input, args, budget) {
        Ok(plan) => plan,
        Err(_) => {
            // Invalid syntax cannot be located; it is reported, not judged.
            skip(file, "Syntax errors; this file was not judged.");
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
