//! Cached, validated and budgeted TypeSafe requests. Each answer is cached by
//! the state it is about and its question, so a request sends only the
//! questions the cache does not answer; `lookup` finds the answers it holds.
mod billing;
mod lookup;

pub use billing::{Spend, Usage};
pub(super) use lookup::{Answered, cached, question_count, unanswered};

use crate::{evaluate::Session, options::CheckArgs, response, schema};
use anyhow::{Result, ensure};
use billing::Billed;
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
pub(crate) const STAGES: [&str; 23] = [
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
    "custom",
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
        if !pending.is_empty() && self.halted.is_none() {
            // Asked once: a store that refused, or a dialog declined, is not
            // asked again for every batch.
            self.halted = self.evaluator.unavailable();
        }
        if let Some(error) = self.halted.as_ref().filter(|_| !pending.is_empty()) {
            // Without a key nothing can be sent: each file fails with the
            // reason, and `check` ends with it once.
            for unsent in &pending {
                receipts[unsent.index].result = Err(anyhow::anyhow!("{error:#}"));
            }
            return receipts;
        }
        let allowed = pending.len().min(
            self.args
                .max_requests
                .map_or(pending.len(), |n| n.saturating_sub(self.requests) as usize),
        );
        // The agent hook answers on its own terms; a check says it on stderr.
        if allowed < pending.len()
            && !crate::hook::invoked()
            && !std::mem::replace(&mut self.budget_noted, true)
        {
            note!(
                "jevgate: {}",
                budget_short(self.args, self.requests as usize + pending.len())
            );
        }
        for unsent in pending.drain(allowed..) {
            receipts[unsent.index].result = Err(anyhow::anyhow!(budget_reached(self.args)));
        }
        if !pending.is_empty() {
            crate::progress::sending(pending.len());
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
        let mut copied = false;
        let mut lookups: Vec<Lookup> = requests
            .iter()
            .map(|request| {
                let (lookup, copy) = self.look_up(request, &cache);
                copied |= copy;
                lookup
            })
            .collect();
        if copied {
            // A request looked up before another one of the batch copied an
            // earlier version's answers about its state reads them now: the
            // batch then gives a question one answer, which a rerun reads,
            // and does not buy an answer the cache holds.
            for (lookup, request) in lookups.iter_mut().zip(requests) {
                if lookup.lacks_answers() {
                    *lookup = self.look_up(request, &cache).0;
                }
            }
        }
        let mut pending = Vec::new();
        for (index, (request, lookup)) in requests.iter().zip(lookups).enumerate() {
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

    /// The cached answers to `request`, after copying those it carried over
    /// from an earlier version's whole-request entry into its state's file;
    /// and whether any were copied.
    fn look_up(&self, request: &Value, cache: &crate::storage::CacheReader) -> (Lookup, bool) {
        let mut lookup = Lookup::new(self.args, request, Some(cache), &self.answered);
        let carried = std::mem::take(&mut lookup.carried);
        // A copy that cannot be written is made again from the whole-request
        // entry on the next run; this run has the answers.
        let copied = !carried.is_empty() && self.store.copy_answers(&lookup.state, carried).is_ok();
        (lookup, copied)
    }

    /// Upload `pending` through the evaluator, rechecking each source first,
    /// and record every outcome in its receipt.
    fn send(&mut self, pending: Vec<Pending<'_>>, receipts: &mut [Receipt]) {
        let (args, root) = (self.args, &self.context.root);
        let (spend, budget) = (self.spend.as_ref(), &self.budget);
        let before = |request: &Value| {
            crate::cancellation::check()?;
            // A queued upload must recheck the current bytes even when its
            // source was already verified while preparing the batch.
            require_paths(args, root, request, &mut SourceHashes::new())?;
            match spend {
                Some(spend) => spend.charge(args, request, || {
                    let tokens = budget.tokens_of(&provider_request(request)) as u64;
                    crate::model::usd(args.model(), tokens)
                        .unwrap_or(tokens as f64 * crate::model::INPUT_USD_PER_MILLION / 1e6)
                }),
                None => Ok(()),
            }
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
                crate::progress::answered();
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

/// The request budget and where it is set: `--max-requests` only lowers
/// the ceiling jevgate.toml sets, so raising the flag cannot help then.
fn budget_limit(args: &CheckArgs) -> String {
    let limit = args.max_requests.unwrap_or_default();
    if args.max_requests_in_config {
        format!("max_requests = {limit} in jevgate.toml")
    } else {
        format!("--max-requests {limit}")
    }
}

/// Why a request was not sent: the run reached its request budget. The
/// answers it got are cached, so a rerun asks only for the rest.
fn budget_reached(args: &CheckArgs) -> String {
    format!(
        "Request budget reached ({}); rerun to continue from the cached answers, or raise the budget",
        budget_limit(args)
    )
}

/// Said once, before the first request the budget holds back is dropped:
/// that the check will end incomplete, with at least `needed` requests
/// planned, and what finishes it.
fn budget_short(args: &CheckArgs, needed: usize) -> String {
    let raise = if args.max_requests_in_config {
        "raise max_requests in jevgate.toml"
    } else {
        "pass a larger --max-requests"
    };
    format!(
        "this check needs at least {needed} requests, more than {} allows, so it will end incomplete. The answers it gets are cached and a rerun continues from them; {raise} to finish in one run.",
        budget_limit(args)
    )
}

/// Record one outcome of sending `sent`, the unanswered questions of `lookup`'s
/// request: timing, token usage, and the answers [`receive`] takes from it.
/// Returns what an answered request was billed, even when its answers failed
/// validation.
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
    let billed = outcome.result.as_ref().ok().map(Billed::of);
    if let Some(bill) = &billed {
        receipt.metrics.input_tokens += bill.input_tokens.unwrap_or(0);
        receipt.metrics.output_tokens += bill.output_tokens;
    }
    receipt.result = outcome
        .result
        .and_then(|body| receive((store, answered), sent, lookup, &body, &mut receipt.metrics));
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

/// Take the provider's `body` answering `sent`: validate it, keep its
/// answers in the cache, and return them joined with the cached ones, as
/// the planned request's answers, with when they were given. `metrics`
/// counts the questions asked and those the cache answered.
fn receive(
    (store, answered): (&crate::storage::Store, &mut Answered),
    sent: &Value,
    lookup: &mut Lookup,
    body: &Value,
    metrics: &mut schema::StageMetrics,
) -> Result<(Value, u64, bool)> {
    response::validate(body, sent)?;
    let cached = lookup.found.len() as u64;
    let timestamp = lookup.keep(store, answered, sent, body)?;
    metrics.evaluated_judgments += 1;
    metrics.asked_questions = question_count(sent);
    metrics.cached_questions = cached;
    Ok((lookup.body(sent).0, timestamp, false))
}

pub(super) fn require_current(
    session: &Session<'_>,
    request: &Value,
    hashes: &mut SourceHashes,
) -> Result<()> {
    require_paths(session.args, &session.context.root, request, hashes)
}

/// Stop when a file `request` holds no longer reads as it did when the
/// request was built, as `args` reads it: from disk, or as Git holds a
/// file a `--staged` or `--pre-push` change touched.
fn require_paths(
    args: &CheckArgs,
    root: &std::path::Path,
    request: &Value,
    hashes: &mut SourceHashes,
) -> Result<()> {
    let max_bytes = args.max_context_bytes.max(args.max_file_bytes);
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
                        args.read(&root.join(path), max_bytes)
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
