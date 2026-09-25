//! Cached, validated and budgeted TypeSafe requests. One cache entry per uploaded request.
use crate::{evaluate::Session, options::Provider, response, schema};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::collections::BTreeMap;

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

/// Map local requests to the selected provider's accepted request shape.
pub(super) fn provider_request_for(
    request: &Value,
    provider: Provider,
) -> std::borrow::Cow<'_, Value> {
    let clean = provider_request(request);
    // OpenRouter's System One API accepts bare aliases and maps them to the
    // `typesafe/` namespace; the route configuration keeps the explicit slug.
    let alias = clean["model"]
        .as_str()
        .and_then(|model| model.strip_prefix("typesafe/"))
        .filter(|model| matches!(*model, "jev-latest" | "jev-preview"))
        .map(str::to_owned);
    if provider == Provider::OpenRouter
        && let Some(alias) = alias
    {
        let mut copy = clean.into_owned();
        copy["model"] = Value::String(alias);
        std::borrow::Cow::Owned(copy)
    } else {
        clean
    }
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
pub(crate) const STAGES: [&str; 18] = [
    "file-purpose",
    "functions",
    "outline",
    "duplicate-pair",
    "tests",
    "test-pair",
    "recheck",
    "locate",
    "values",
    "constants",
    "security",
    "trace",
    "settle",
    "instructions",
    "docs",
    "doc-checks",
    "access",
    "workflows",
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

/// One cache entry per request: the model, state and questions it uploads.
pub(super) fn judgment_key(request: &Value, provider: Provider) -> String {
    let clean = provider_request(request);
    if provider == Provider::OpenRouter {
        schema::hash(&serde_json::to_vec(&(schema::RUBRIC, "openrouter", clean)).unwrap())
    } else {
        schema::hash(&serde_json::to_vec(&(schema::RUBRIC, clean)).unwrap())
    }
}

/// Aliases move to new model versions, so their answers expire. A pinned
/// version answers the same request the same way; its entries never expire.
fn cache_ttl(model: &str, ttl: u64) -> Option<u64> {
    matches!(
        model,
        "jev-latest" | "jev-preview" | "typesafe/jev-latest" | "typesafe/jev-preview"
    )
    .then_some(ttl)
}

/// A valid, unexpired cached answer to `request`, read through `load`; none
/// with `--refresh`.
fn cached_answer(
    args: &crate::options::CheckArgs,
    request: &Value,
    load: impl Fn(&str, Option<u64>) -> Option<(Value, u64)>,
) -> Option<(Value, u64)> {
    if args.refresh {
        return None;
    }
    load(
        &judgment_key(request, args.provider()),
        cache_ttl(args.model(), args.cache_ttl_secs()),
    )
    .filter(|(b, _)| response::validate(b, request).is_ok())
}

/// Whether a dry run's planned request already has a cached answer, read
/// without opening the store.
pub(super) fn answered(
    root: &std::path::Path,
    args: &crate::options::CheckArgs,
    request: &Value,
) -> bool {
    cached_answer(args, request, |key, ttl| {
        crate::storage::peek(root, key, ttl)
    })
    .is_some()
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
        for (i, _) in pending.drain(allowed..) {
            receipts[i].result = Err(anyhow::anyhow!(
                "Session API request budget exhausted; restart with an explicit larger --max-requests"
            ));
        }
        if !pending.is_empty() {
            self.send(&pending, &mut receipts);
        }
        receipts
    }

    /// Fill receipts from valid cached answers; return the requests still to send.
    fn answer_from_cache<'r>(
        &self,
        requests: &[&'r Value],
        receipts: &mut [Receipt],
    ) -> Vec<(usize, &'r Value)> {
        let mut pending = Vec::new();
        for (i, request) in requests.iter().enumerate() {
            let cached = cached_answer(self.args, request, |key, ttl| self.store.load(key, ttl));
            if let Some((cached, created)) = cached {
                receipts[i].metrics.cache_hits = 1;
                receipts[i].metrics.cached_judgments = 1;
                receipts[i].result = Ok((cached, created, true));
            } else if self.args.cache_only {
                receipts[i].result = Err(anyhow::anyhow!(
                    "No current cached response; rerun without --cache-only to allow an API request"
                ));
            } else {
                pending.push((i, *request));
            }
        }
        pending
    }

    /// Upload `pending` through the evaluator, rechecking each source first,
    /// and record every outcome in its receipt.
    fn send(&mut self, pending: &[(usize, &Value)], receipts: &mut [Receipt]) {
        let root = &self.context.root;
        let max_bytes = self.args.max_context_bytes.max(self.args.max_file_bytes);
        let before = |request: &Value| {
            crate::cancellation::check()?;
            // A queued upload must recheck the current bytes even when its
            // source was already verified while preparing the batch.
            require_paths(root, max_bytes, request, &mut SourceHashes::new())
        };
        let batch: Vec<&Value> = pending.iter().map(|(_, r)| *r).collect();
        let store = self.store;
        let requests_count = &mut self.requests;
        let paid = (&mut self.paid_input_tokens, &mut self.paid_output_tokens);
        let observed = &mut self.observed;
        self.evaluator.evaluate_queue(
            &batch,
            self.args.concurrency as usize,
            &before,
            &mut |index, outcome| {
                let (i, request) = pending[index];
                *requests_count += u32::from(outcome.attempted);
                let receipt = &mut receipts[i];
                record(store, request, self.args.provider(), outcome, receipt);
                *paid.0 += receipt.metrics.input_tokens;
                *paid.1 += receipt.metrics.output_tokens;
                if receipt.metrics.evaluated_judgments > 0 {
                    observed.0 += serde_json::to_vec(&provider_request(request))
                        .map_or(0, |v| v.len() as u64);
                    observed.1 += receipt.metrics.input_tokens;
                }
            },
        );
    }
}

/// Record one outcome: timing, token usage, and a validated answer saved to the cache.
fn record(
    store: &crate::storage::Store,
    request: &Value,
    provider: Provider,
    outcome: crate::transport::Outcome,
    receipt: &mut Receipt,
) {
    receipt.metrics.service_ms = outcome.elapsed_ms;
    receipt.metrics.queue_wait_ms = outcome.started_ms;
    receipt.metrics.evidence_bytes = if outcome.attempted {
        evidence_bytes(request)
    } else {
        0
    };
    receipt.result = outcome.result.and_then(|body| {
        receipt.metrics.input_tokens += usage(&body, "input_tokens");
        receipt.metrics.output_tokens += usage(&body, "output_tokens");
        response::validate(&body, request)?;
        let timestamp = schema::now();
        store.save(
            &judgment_key(request, provider),
            &response::cache_value(&body, request),
            timestamp,
        )?;
        receipt.metrics.evaluated_judgments += 1;
        Ok((body, timestamp, false))
    });
    receipt.metrics.retries = u64::from(outcome.retries);
    if outcome.attempted {
        if receipt.result.is_ok() {
            receipt.metrics.successful_requests = 1;
        } else {
            receipt.metrics.failed_attempts = 1;
        }
    }
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

/// A usage count above this is corrupt, not a real count, and is ignored.
const MAX_REPORTED_TOKENS: u64 = 1_000_000_000;

fn usage(body: &Value, field: &str) -> u64 {
    body["usage"][field]
        .as_u64()
        .filter(|n| *n <= MAX_REPORTED_TOKENS)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_answers_do_not_expire_and_aliases_do() {
        let project = crate::tests::Project::new();
        let store = crate::storage::Store::open(&project.0).unwrap();
        let body = serde_json::json!({"model":"jev-1.13.0","answers":{}});
        store.save("old", &body, schema::now() - 7200).unwrap();
        for (model, kept) in [
            ("jev-1.13.0", true),
            ("jev-latest", false),
            ("jev-preview", false),
        ] {
            let ttl = cache_ttl(model, 3600);
            assert_eq!(store.load("old", ttl).is_some(), kept, "{model}");
        }
        assert!(store.load("old", cache_ttl("jev-latest", 86_400)).is_some());
        assert!(store.load("other", None).is_none());
        assert_eq!(cache_ttl("typesafe/jev-latest", 3600), Some(3600));
    }

    #[test]
    fn local_metadata_is_not_uploaded_or_part_of_the_cache_key() {
        let plain = serde_json::json!({"model":"m","state":{"a":1},"questions":{}});
        let mut tagged = plain.clone();
        tagged["jevgate"] = serde_json::json!({"stage":"functions","sources":[]});
        assert_eq!(*provider_request(&tagged), plain);
        let default_openrouter = serde_json::json!({
            "model":"typesafe/jev-latest",
            "state":{"a":1},
            "questions":{
                "kind":{"type":"choice","criteria":{"ball":"Ball","book":"Book"}},
                "size":{"type":"score","criteria":["Small","Medium","Large"]}
            },
            "jevgate":{"stage":"functions","sources":[]}
        });
        let mapped = provider_request_for(&default_openrouter, Provider::OpenRouter);
        assert_eq!(mapped["model"], "jev-latest");
        let preview = serde_json::json!({"model":"typesafe/jev-preview"});
        assert_eq!(
            provider_request_for(&preview, Provider::OpenRouter)["model"],
            "jev-preview"
        );
        assert_eq!(mapped["state"], default_openrouter["state"]);
        assert_eq!(mapped["questions"], default_openrouter["questions"]);
        assert!(mapped.get("jevgate").is_none());
        assert_eq!(
            provider_request_for(&default_openrouter, Provider::TypeSafe)["model"],
            "typesafe/jev-latest"
        );
        assert_eq!(
            judgment_key(&tagged, Provider::TypeSafe),
            judgment_key(&plain, Provider::TypeSafe)
        );
        assert_ne!(
            judgment_key(&plain, Provider::TypeSafe),
            judgment_key(&plain, Provider::OpenRouter)
        );
        assert_eq!(stage(&tagged), "functions");
    }
}
