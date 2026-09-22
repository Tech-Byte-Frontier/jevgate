use super::{
    inventory::Input,
    options::CheckArgs,
    schema::{self, FileResult, Report, Status},
    storage::Store,
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
    // Always recompose from cached answers. Skipping apply() on reused
    // FileResults hid composition changes until COMPOSITION was bumped.
    let files = inputs.iter().map(|input| input.result.clone()).collect();
    let mut report = Report {
        quick: args.quick,
        base_revision: args.base.clone(),
        deleted_files: Vec::new(),
        schema_version: 1,
        command: if args.roles_only {
            "classify-roles"
        } else {
            "check"
        }
        .into(),
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
        requested_model: args.model.clone(),
        api_requests: current.requests,
        concurrency: args.concurrency,
        paid_input_tokens: 0,
        paid_output_tokens: 0,
        stages: BTreeMap::new(),
        settled: false,
        files,
        changes: Vec::new(),
        decision_policy: (if args.roles_only {
            crate::roles::policy()
        } else {
            crate::maintainability::policy()
        })
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect(),
    };
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
        for (index, input) in inputs.iter().enumerate() {
            if report.files[index].status != Status::Pending {
                continue;
            }
            match schedule(input, args, &mut report.files[index]) {
                Ok(Scheduled::None) => {}
                Ok(Scheduled::Purpose(request) | Scheduled::Judge(request)) => {
                    let stage = report
                        .stages
                        .entry(crate::requests::stage(&request).into())
                        .or_default();
                    stage.planned_requests += 1;
                    stage.planned_evidence_bytes += crate::requests::evidence_bytes(&request);
                    if args.show_requests {
                        report.initial_requests.push(request);
                    }
                }
                Err(error) => report.errors.push(error.to_string()),
            }
        }
        report.update_status();
    }
    report
}

impl Session<'_> {
    pub fn evaluate(&mut self, inputs: &[Input], report: &mut Report) -> Result<()> {
        self.evaluator.begin_review();
        self.publish(report)?;
        let selected: Vec<_> = report
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, f)| (f.status == Status::Pending).then_some(i))
            .collect();
        let mut purpose = Vec::new();
        let mut gates = Vec::new();
        for &owner in &selected {
            match schedule(&inputs[owner], self.args, &mut report.files[owner]) {
                Ok(Scheduled::None) => report.files[owner].cached = false,
                Ok(Scheduled::Purpose(request)) => {
                    enqueue(&mut report.files[owner], &mut purpose, owner, request);
                }
                Ok(Scheduled::Judge(request)) => {
                    enqueue(&mut report.files[owner], &mut gates, owner, request);
                }
                Err(error) => fail(&mut report.files[owner], error),
            }
        }
        if !purpose.is_empty() {
            self.dispatch(report, purpose, |file, request, body| {
                crate::file_kind::record_purpose(file, &request, body)
            })?;
            for &owner in &selected {
                if report.files[owner].status == Status::Error
                    || report.files[owner]
                        .classification
                        .as_ref()
                        .is_none_or(|class| class.stage != "answered")
                {
                    continue;
                }
                match crate::file_kind::decide_after_purpose(
                    &inputs[owner],
                    self.args,
                    &mut report.files[owner],
                ) {
                    Ok(Some(request)) => gates.push(queued(owner, request)),
                    Ok(None) => {}
                    Err(error) => fail(&mut report.files[owner], error),
                }
            }
        }
        if !gates.is_empty() {
            self.dispatch(report, gates, |file, request, body| {
                if request["state"]["role_version"].is_string() {
                    crate::roles::apply(file, &request, body)
                } else {
                    crate::maintainability::apply(file, &request, body)
                }
            })?;
        }
        if self.args.include_tests {
            let mut portions = Vec::new();
            for (owner, input) in inputs.iter().enumerate() {
                match crate::file_kind::test_portion_request(
                    input,
                    self.args,
                    &mut report.files[owner],
                ) {
                    Ok(Some(request)) => portions.push(queued(owner, request)),
                    Ok(None) => {}
                    Err(error) => report.errors.push(error.to_string()),
                }
            }
            if !portions.is_empty() {
                self.dispatch(report, portions, |file, request, body| {
                    crate::maintainability::apply_test_portion(file, &request, body)
                })?;
            }
        }
        let mut followups = Vec::new();
        for (owner, input) in inputs.iter().enumerate() {
            if report.files[owner].status == Status::Error {
                continue;
            }
            match crate::maintainability::focused_requests(input, &report.files[owner], self.args) {
                Ok(requests) => {
                    for request in requests {
                        followups.push(queued(owner, request));
                    }
                }
                Err(error) => report.errors.push(error.to_string()),
            }
        }
        if !followups.is_empty() {
            self.dispatch(report, followups, |file, request, body| {
                crate::maintainability::apply_focused(file, &request, body)
            })?;
        }
        let mut extractions = Vec::new();
        for (owner, input) in inputs.iter().enumerate() {
            if report.files[owner].status == Status::Error {
                continue;
            }
            match crate::maintainability::extraction_requests(
                input,
                &report.files[owner],
                self.args,
            ) {
                Ok(requests) => {
                    for request in requests {
                        extractions.push(queued(owner, request));
                    }
                }
                Err(error) => report.errors.push(error.to_string()),
            }
        }
        if !extractions.is_empty() {
            self.dispatch(report, extractions, |file, request, body| {
                crate::maintainability::apply_extraction(file, &request, body)
            })?;
        }
        self.progress(report)
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
            let stage = report.stages.entry(name.into()).or_default();
            let m = receipt.metrics;
            stage.service_ms += m.service_ms;
            stage.queue_wait_ms += m.queue_wait_ms;
            if m.successful_requests + m.failed_attempts > 0 {
                let span = spans
                    .entry(name)
                    .or_insert((m.queue_wait_ms, m.queue_wait_ms + m.service_ms));
                span.0 = span.0.min(m.queue_wait_ms);
                span.1 = span.1.max(m.queue_wait_ms + m.service_ms);
            }
            stage.successful_requests += m.successful_requests;
            stage.failed_attempts += m.failed_attempts;
            stage.cache_hits += m.cache_hits;
            stage.cached_judgments += m.cached_judgments;
            stage.evaluated_judgments += m.evaluated_judgments;
            stage.input_tokens += m.input_tokens;
            stage.output_tokens += m.output_tokens;
            stage.evidence_bytes += m.evidence_bytes;
            let file = &mut report.files[task.owner];
            file.elapsed_ms += m.service_ms;
            match receipt.result {
                Ok((body, timestamp, cached)) => {
                    file.cached &= cached;
                    file.input_tokens += body["usage"]["input_tokens"].as_u64().unwrap_or(0);
                    file.output_tokens += body["usage"]["output_tokens"].as_u64().unwrap_or(0);
                    file.evaluated_at = Some(timestamp);
                    if file.status != Status::Error
                        && let Err(error) = apply(file, task.payload, &body)
                    {
                        fail(file, error);
                    }
                }
                // Later skipped work must not overwrite this file's first failure.
                Err(error) if file.status != Status::Error => fail(file, error),
                Err(_) => {}
            }
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
            super::output::emit(report, super::options::Format::Jsonl)?;
        }
        Ok(())
    }
}

enum Scheduled {
    None,
    Purpose(serde_json::Value),
    Judge(serde_json::Value),
}

fn queued(owner: usize, request: serde_json::Value) -> Task<serde_json::Value> {
    Task {
        owner,
        payload: request.clone(),
        request,
    }
}

fn enqueue(
    file: &mut FileResult,
    tasks: &mut Vec<Task<serde_json::Value>>,
    owner: usize,
    request: serde_json::Value,
) {
    file.cached = true;
    tasks.push(queued(owner, request));
}

fn apply_classification(file: &mut FileResult, class: crate::file_kind::Classification) {
    file.contains_tests = class.kind == "tests" || !class.separated_tests.is_empty();
    file.classification = Some(class);
}

fn schedule(input: &Input, args: &CheckArgs, file: &mut FileResult) -> Result<Scheduled> {
    if file.status != Status::Pending {
        return Ok(Scheduled::None);
    }
    if args.roles_only {
        let request = crate::roles::request(input, args)?;
        if !crate::cascade::within_budget(&request) {
            let source = input.source.as_deref().unwrap_or("");
            file.classification = Some(crate::file_kind::unsent(
                &input.result.path,
                source,
                &crate::file_kind::budget_detail(source.len()),
            ));
            file.status = Status::NeedsContext;
            return Ok(Scheduled::None);
        }
        return Ok(Scheduled::Judge(request));
    }
    match crate::file_kind::plan(input, args)? {
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
        crate::file_kind::Plan::Judge(class, request) => {
            file.contains_tests =
                file.contains_tests || class.kind == "tests" || !class.separated_tests.is_empty();
            file.classification = Some(class);
            Ok(Scheduled::Judge(request))
        }
    }
}

struct Task<T> {
    owner: usize,
    request: serde_json::Value,
    payload: T,
}

fn fail(file: &mut FileResult, error: anyhow::Error) {
    file.status = Status::Error;
    file.cached = false;
    file.error = Some(error.to_string());
}
