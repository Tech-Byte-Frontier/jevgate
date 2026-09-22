//! Cached, validated and budgeted TypeSafe requests. One cache entry per uploaded request.
use crate::{evaluate::Session, response, schema};
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
pub(crate) const STAGES: [&str; 7] = [
    "file-purpose",
    "functions",
    "outline",
    "duplicate-pair",
    "tests",
    "test-pair",
    "recheck",
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

/// Provider context limits: all questions plus state, and state plus the longest question.
const TOTAL_TOKENS: f64 = 64_000.0;
const STATE_TOKENS: f64 = 32_000.0;
/// Headroom for estimation error.
const MARGIN: f64 = 0.9;
const BUDGET_FILE: &str = "token-budget.json";

/// Token estimates from request bytes. The ratio is calibrated from observed
/// `usage.input_tokens` and saved in `.jevgate/`; it only decides packing and
/// whether a unit fits, never a verdict.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TokenBudget {
    pub bytes_per_token: f64,
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            bytes_per_token: 3.0,
        }
    }
}

impl TokenBudget {
    pub fn load(root: &std::path::Path) -> Self {
        crate::inventory::read_source(&root.join(".jevgate").join(BUDGET_FILE), 4096)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .map(|b| Self::calibrated(b.bytes_per_token))
            .unwrap_or_default()
    }

    fn calibrated(bytes_per_token: f64) -> Self {
        Self {
            bytes_per_token: if bytes_per_token.is_finite() {
                bytes_per_token.clamp(2.0, 6.0)
            } else {
                Self::default().bytes_per_token
            },
        }
    }

    /// Replace the ratio with one observed over a batch of fresh requests.
    pub fn observe(&mut self, bytes: u64, tokens: u64) {
        if tokens > 0 {
            *self = Self::calibrated(bytes as f64 / tokens as f64);
        }
    }

    pub fn save(&self, store: &crate::storage::Store) -> Result<()> {
        store.write(BUDGET_FILE, &serde_json::to_vec(self)?)
    }

    pub fn tokens(&self, bytes: usize) -> usize {
        (bytes as f64 / self.bytes_per_token).ceil() as usize
    }

    pub fn tokens_of(&self, value: &Value) -> usize {
        self.tokens(serde_json::to_vec(value).map_or(0, |v| v.len()))
    }

    /// Estimated uploaded tokens of a request.
    pub fn request_tokens(&self, request: &Value) -> usize {
        self.tokens_of(&provider_request(request))
    }

    pub fn fits(&self, request: &Value) -> bool {
        let provider = provider_request(request);
        let state = self.tokens_of(&provider["state"]) as f64;
        let longest = provider["questions"]
            .as_object()
            .into_iter()
            .flat_map(|q| q.values())
            .map(|q| self.tokens_of(q))
            .max()
            .unwrap_or(0) as f64;
        (self.tokens_of(&provider) as f64) <= TOTAL_TOKENS * MARGIN
            && state + longest <= STATE_TOKENS * MARGIN
    }
}

/// One cache entry per request: the model, state and questions it uploads.
pub(super) fn judgment_key(request: &Value) -> String {
    schema::hash(&serde_json::to_vec(&(schema::RUBRIC, provider_request(request))).unwrap())
}

/// Aliases move to new model versions, so their answers expire. A pinned
/// version answers the same request the same way; its entries never expire.
fn cache_ttl(model: &str, ttl: u64) -> Option<u64> {
    matches!(model, "jev-latest" | "jev-preview").then_some(ttl)
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
        let mut pending = Vec::new();
        let ttl = cache_ttl(&self.args.model, self.args.cache_ttl_secs);
        for (i, request) in requests.iter().enumerate() {
            let cached = if self.args.refresh {
                None
            } else {
                self.store.load(&judgment_key(request), ttl)
            }
            .filter(|(b, _)| response::validate(b, request).is_ok());
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
        let count = pending.len().min(
            self.args
                .max_requests
                .map_or(pending.len(), |n| n.saturating_sub(self.requests) as usize),
        );
        for (i, _) in &pending[count..] {
            receipts[*i].result = Err(anyhow::anyhow!(
                "Session API request budget exhausted; restart with an explicit larger --max-requests"
            ));
        }
        let root = &self.context.root;
        let max_bytes = self.args.max_context_bytes.max(self.args.max_file_bytes);
        let before = |request: &Value| {
            crate::cancellation::check()?;
            // A queued upload must recheck the current bytes even when its
            // source was already verified while preparing the batch.
            require_paths(root, max_bytes, request, &mut SourceHashes::new())
        };
        let batch: Vec<&Value> = pending[..count].iter().map(|(_, r)| *r).collect();
        if batch.is_empty() {
            return receipts;
        }
        let store = self.store;
        let requests_count = &mut self.requests;
        let paid_input = &mut self.paid_input_tokens;
        let paid_output = &mut self.paid_output_tokens;
        let observed = &mut self.observed;
        self.evaluator.evaluate_queue(
            &batch,
            self.args.concurrency as usize,
            &before,
            &mut |index, outcome| {
                let (i, request) = &pending[index];
                *requests_count += u32::from(outcome.attempted);
                let receipt = &mut receipts[*i];
                receipt.metrics.service_ms = outcome.elapsed_ms;
                receipt.metrics.queue_wait_ms = outcome.started_ms;
                receipt.metrics.evidence_bytes = if outcome.attempted {
                    evidence_bytes(request)
                } else {
                    0
                };
                receipt.result = outcome.result.and_then(|body| {
                    let input = usage(&body, "input_tokens");
                    let output = usage(&body, "output_tokens");
                    *paid_input += input;
                    *paid_output += output;
                    receipt.metrics.input_tokens += input;
                    receipt.metrics.output_tokens += output;
                    response::validate(&body, request)?;
                    observed.0 += serde_json::to_vec(&provider_request(request))
                        .map_or(0, |v| v.len() as u64);
                    observed.1 += input;
                    let timestamp = schema::now();
                    store.save(
                        &judgment_key(request),
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
            },
        );
        receipts
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

fn usage(body: &Value, field: &str) -> u64 {
    body["usage"][field]
        .as_u64()
        .filter(|n| *n <= 1_000_000_000)
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
    }

    #[test]
    fn local_metadata_is_not_uploaded_or_part_of_the_cache_key() {
        let plain = serde_json::json!({"model":"m","state":{"a":1},"questions":{}});
        let mut tagged = plain.clone();
        tagged["jevgate"] = serde_json::json!({"stage":"functions","sources":[]});
        assert_eq!(*provider_request(&tagged), plain);
        assert_eq!(judgment_key(&tagged), judgment_key(&plain));
        assert_eq!(stage(&tagged), "functions");
    }
}
