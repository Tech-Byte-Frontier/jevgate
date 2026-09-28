//! Cached, validated and budgeted TypeSafe requests. Each answer is cached by
//! the state it is about and its question, so a request sends only the
//! questions the cache does not answer; `lookup` finds the answers it holds.
mod lookup;

pub(super) use lookup::{Answered, cached, question_count, unanswered};

use crate::{evaluate::Session, response, schema};
use anyhow::{Result, ensure};
use lookup::Lookup;
use serde_json::Value;
use std::{borrow::Cow, collections::BTreeMap};

pub(super) type SourceHashes = BTreeMap<String, Option<String>>;

/// Local metadata (stage, freshness hashes) lives under `jevgate` and is
/// never uploaded. Cache identity and previews use the provider copy.
pub(super) fn provider_request(request: &Value) -> std::borrow::Cow<'_, Value> {
    if request.get("jevgate").is_none() {
        return std::borrow::Cow::Borrowed(request);
    }
    let mut copy = request.clone();
    copy.as_object_mut().unwrap().remove("jevgate");
    std::borrow::Cow::Owned(copy)
}

pub(super) fn evidence_bytes(request: &Value) -> u64 {
    serde_json::to_vec(&provider_request(request)["state"])
        .unwrap()
        .len() as u64
}

pub(super) struct Receipt {
    pub result: Result<(Value, u64, bool)>,
    pub metrics: schema::StageMetrics,
}

/// Request kinds reported in `stages`, in dispatch order.
pub(crate) const STAGES: [&str; 22] = [
    "file-purpose",
    "functions",
    "outline",
    "duplicate-pair",
    "tests",
    "test-pair",
    "recheck",
    "locate",
    "constants",
    "comments",
    "security",
    "trace",
    "parts",
    "settle",
    "instructions",
    "docs",
    "doc-checks",
    "access",
    "workflows",
    "laws",
    "steering",
    "guards",
];

pub(super) fn stage(request: &Value) -> &'static str {
    match request["jevgate"]["stage"].as_str() {
        Some(stage) => STAGES
            .iter()
            .find(|s| **s == stage)
            .copied()
            .unwrap_or("other"),
        None if request["state"]["role_version"].is_string() => "roles",
        None if request["state"]["purpose_version"].is_number() => "file-purpose",
        None => "maintainability",
    }
}

/// A planned request whose questions the cache does not all answer.
struct Pending<'r> {
    /// Its index in the batch.
    index: usize,
    /// The request as sent: only its unanswered questions.
    sent: Cow<'r, Value>,
    lookup: Lookup,
}

impl Session<'_> {
    pub(super) fn queries(&mut self, requests: &[&Value]) -> Vec<Receipt> {
        let mut receipts: Vec<_> = requests
            .iter()
            .map(|_| Receipt {
                result: Err(anyhow::anyhow!("No receipt")),
                metrics: Default::default(),
            })
            .collect();
        let mut pending = self.answer_from_cache(requests, &mut receipts);
        let allowed = pending.len().min(
            self.args
                .max_requests
                .map_or(pending.len(), |n| n.saturating_sub(self.requests) as usize),
        );
        for unsent in pending.drain(allowed..) {
            receipts[unsent.index].result = Err(anyhow::anyhow!(
                "Session API request budget exhausted; restart with an explicit larger --max-requests"
            ));
        }
        if !pending.is_empty() {
            self.send(pending, &mut receipts);
        }
        receipts
    }

    /// Fill receipts from cached answers; return the requests still to send,
    /// each with only its unanswered questions.
    fn answer_from_cache<'r>(
        &self,
        requests: &[&'r Value],
        receipts: &mut [Receipt],
    ) -> Vec<Pending<'r>> {
        let cache = self.store.reader();
        let mut pending = Vec::new();
        for (index, request) in requests.iter().enumerate() {
            let mut lookup = Lookup::new(self.args, request, Some(&cache), &self.answered);
            let carried = std::mem::take(&mut lookup.carried);
            if !carried.is_empty() {
                // A copy that cannot be written is made again from the
                // whole-request entry on the next run; this run has the answers.
                let _ = self.store.copy_answers(&lookup.state, carried);
            }
            let receipt = &mut receipts[index];
            match lookup.unanswered(request) {
                None => {
                    let (body, created_at) = lookup.body(request);
                    receipt.metrics.cache_hits = 1;
                    receipt.metrics.cached_judgments = 1;
                    receipt.metrics.cached_questions = lookup.found.len() as u64;
                    receipt.result = Ok((body, created_at, true));
                }
                Some(_) if self.args.cache_only => {
                    receipt.result = Err(anyhow::anyhow!(
                        "No current cached response; rerun without --cache-only to allow an API request"
                    ));
                }
                Some(sent) => pending.push(Pending {
                    index,
                    sent,
                    lookup,
                }),
            }
        }
        pending
    }

    /// Upload `pending` through the evaluator, rechecking each source first,
    /// and record every outcome in its receipt.
    fn send(&mut self, pending: Vec<Pending<'_>>, receipts: &mut [Receipt]) {
        let root = &self.context.root;
        let max_bytes = self.args.max_context_bytes.max(self.args.max_file_bytes);
        let before = |request: &Value| {
            crate::cancellation::check()?;
            // A queued upload must recheck the current bytes even when its
            // source was already verified while preparing the batch.
            require_paths(root, max_bytes, request, &mut SourceHashes::new())
        };
        let (sent, mut lookups): (Vec<_>, Vec<_>) = pending
            .into_iter()
            .map(|asked| (asked.sent, (asked.index, asked.lookup)))
            .unzip();
        let batch: Vec<&Value> = sent.iter().map(AsRef::as_ref).collect();
        let store = self.store;
        let answered = &mut self.answered;
        let requests_count = &mut self.requests;
        let paid = &mut self.paid;
        let observed = &mut self.observed;
        self.evaluator.evaluate_queue(
            &batch,
            self.args.concurrency() as usize,
            &before,
            &mut |at, outcome| {
                let (index, lookup) = &mut lookups[at];
                *requests_count += u32::from(outcome.attempted);
                let receipt = &mut receipts[*index];
                let billed = record(store, answered, &sent[at], lookup, outcome, receipt);
                // An answer without usage says nothing of its tokens, so it
                // stays out of the bytes-per-token calibration.
                let metered = billed.as_ref().is_some_and(|b| b.input_tokens.is_some());
                if let Some(billed) = billed {
                    paid.bill(billed);
                }
                if metered && receipt.metrics.evaluated_judgments > 0 {
                    observed.0 += serde_json::to_vec(&provider_request(&sent[at]))
                        .map_or(0, |v| v.len() as u64);
                    observed.1 += receipt.metrics.input_tokens;
                }
            },
        );
    }
}

/// What one answered request was billed: the model that answered, and the
/// tokens its response reported.
pub(super) struct Billed {
    model: String,
    /// None when the response reported no usage.
    input_tokens: Option<u64>,
    output_tokens: u64,
}

/// What this invocation's requests were billed, and by which models.
#[derive(Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens by the model that answered them.
    pub models: BTreeMap<String, u64>,
    /// Answers whose response reported no usage.
    pub unmetered: u32,
}

impl Usage {
    fn bill(&mut self, billed: Billed) {
        self.output_tokens += billed.output_tokens;
        match billed.input_tokens {
            Some(tokens) => {
                self.input_tokens += tokens;
                *self.models.entry(billed.model).or_default() += tokens;
            }
            None => self.unmetered += 1,
        }
    }

    /// Dollars, priced by the model that answered each request; unknown when
    /// an answer reported no usage or a model has no published price. The
    /// fold starts at 0.0: a float `sum` of nothing is -0.0, shown as "$-0.0000".
    pub fn usd(&self) -> Option<f64> {
        if self.unmetered > 0 {
            return None;
        }
        self.models.iter().try_fold(0.0, |total, (model, tokens)| {
            Some(total + crate::model::usd(model, *tokens)?)
        })
    }
}

/// Record one outcome of sending `sent`, the unanswered questions of `lookup`'s
/// request: timing, token usage, and the validated answers saved to the cache
/// and joined with the cached ones. Returns what an answered request was
/// billed, even when its answers failed validation.
fn record(
    store: &crate::storage::Store,
    answered: &mut Answered,
    sent: &Value,
    lookup: &mut Lookup,
    outcome: crate::transport::Outcome,
    receipt: &mut Receipt,
) -> Option<Billed> {
    receipt.metrics.service_ms = outcome.elapsed_ms;
    receipt.metrics.queue_wait_ms = outcome.started_ms;
    receipt.metrics.evidence_bytes = if outcome.attempted {
        evidence_bytes(sent)
    } else {
        0
    };
    let mut billed = None;
    receipt.result = outcome.result.and_then(|body| {
        let input_tokens = response::input_tokens(&body);
        let output_tokens = response::output_tokens(&body);
        receipt.metrics.input_tokens += input_tokens.unwrap_or(0);
        receipt.metrics.output_tokens += output_tokens;
        // A name that fails validation is billed to "unknown", which has no price.
        let model = body["model"]
            .as_str()
            .filter(|name| crate::model::valid_name(name))
            .unwrap_or("unknown");
        billed = Some(Billed {
            model: model.to_owned(),
            input_tokens,
            output_tokens,
        });
        response::validate(&body, sent)?;
        let cached = lookup.found.len() as u64;
        let timestamp = lookup.keep(store, answered, sent, &body)?;
        receipt.metrics.evaluated_judgments += 1;
        receipt.metrics.asked_questions = question_count(sent);
        receipt.metrics.cached_questions = cached;
        Ok((lookup.body(sent).0, timestamp, false))
    });
    receipt.metrics.retries = u64::from(outcome.retries);
    if outcome.attempted {
        if receipt.result.is_ok() {
            receipt.metrics.successful_requests = 1;
        } else {
            receipt.metrics.failed_attempts = 1;
        }
    }
    billed
}

pub(super) fn require_current(
    session: &Session<'_>,
    request: &Value,
    hashes: &mut SourceHashes,
) -> Result<()> {
    require_paths(
        &session.context.root,
        session
            .args
            .max_context_bytes
            .max(session.args.max_file_bytes),
        request,
        hashes,
    )
}
fn require_paths(
    root: &std::path::Path,
    max_bytes: u64,
    request: &Value,
    hashes: &mut SourceHashes,
) -> Result<()> {
    for file in request["jevgate"]["sources"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let (Some(path), Some(hash)) = (file["path"].as_str(), file["source_hash"].as_str()) {
            ensure!(
                hashes
                    .entry(path.into())
                    .or_insert_with(|| {
                        crate::inventory::read_source(&root.join(path), max_bytes)
                            .ok()
                            .map(|s| schema::hash(s.as_bytes()))
                    })
                    .as_deref()
                    == Some(hash),
                "Source or context changed before request; assessment is stale"
            );
        }
    }
    Ok(())
}
