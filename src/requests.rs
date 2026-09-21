//! Individual rule judgments are reusable across batch composition and unrelated edits.
use crate::{evaluate::Session, response, schema};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) type SourceHashes = BTreeMap<String, Option<String>>;

/// Freshness checks, cache identity and reports retain the full internal request.
/// Only the provider/preview copy omits local diagnostic validation metadata.
pub(super) fn provider_request(request: &Value) -> std::borrow::Cow<'_, Value> {
    std::borrow::Cow::Borrowed(request)
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

pub(super) fn stage(request: &Value) -> &'static str {
    if request["state"]["role_version"].is_string() {
        "roles"
    } else if request["state"]["purpose_version"].is_number() {
        "file-purpose"
    } else {
        "maintainability"
    }
}
fn groups(request: &Value) -> BTreeMap<String, Value> {
    let mut groups = BTreeMap::new();
    for (name, question) in request["questions"].as_object().unwrap() {
        let group = crate::catalog::rules()
            .into_iter()
            .find(|r| {
                !name.starts_with("shared_logic_fragment_")
                    && (name == r.key || name.starts_with(&format!("{}_", r.key)))
            })
            .map_or_else(|| name.clone(), |r| r.key.into());
        let entry = groups.entry(group).or_insert_with(|| {
            let mut r = request.clone();
            r["questions"] = json!({});
            r
        });
        entry["questions"][name] = question.clone();
    }
    // Role-routing fields belong to the shared-logic cascade. Leaving them on
    // the other gates would make a later single-rule check miss an identical judgment.
    for (group, part) in &mut groups {
        if group == "shared_logic"
            || group.starts_with("role_")
            || group.starts_with("cascade_")
            || group.starts_with("shared_logic_fragment_")
        {
            continue;
        }
        let Some(state) = part["state"].as_object_mut() else {
            continue;
        };
        for key in [
            "regions",
            "region_sources",
            "fragments",
            "role_limitations",
            "cascade_role_version",
            "cascade_version",
        ] {
            state.remove(key);
        }
    }
    groups
}
pub(super) fn judgment_key(request: &Value, group: &str) -> String {
    schema::hash(
        &serde_json::to_vec(&(schema::RUBRIC, crate::catalog::rule_version(group), request))
            .unwrap(),
    )
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
        let mut assembled = BTreeMap::new();
        for (i, request) in requests.iter().enumerate() {
            let mut body = json!({"model":request["model"],"answers":{},"usage":{"input_tokens":0,"output_tokens":0}});
            let mut timestamp: Option<u64> = None;
            let mut missing = (*request).clone();
            missing["questions"] = json!({});
            for (group, part) in groups(request) {
                let key = judgment_key(&part, &group);
                let cached = if self.args.refresh {
                    None
                } else {
                    self.store.load(&key, self.args.cache_ttl_secs)
                }
                .filter(|(b, _)| response::validate(b, &part).is_ok());
                if let Some((cached, created)) = cached {
                    timestamp = Some(timestamp.map_or(created, |old| old.min(created)));
                    body["model"] = cached["model"].clone();
                    body["answers"]
                        .as_object_mut()
                        .unwrap()
                        .extend(cached["answers"].as_object().unwrap().clone());
                    receipts[i].metrics.cached_judgments += 1;
                } else {
                    for name in part["questions"].as_object().unwrap().keys() {
                        missing["questions"][name] = request["questions"][name].clone();
                    }
                }
            }
            if missing["questions"].as_object().unwrap().is_empty() {
                receipts[i].metrics.cache_hits = 1;
                receipts[i].result = Ok((body, timestamp.unwrap_or_else(schema::now), true));
            } else if self.args.cache_only {
                receipts[i].result = Err(anyhow::anyhow!(
                    "No current cached response; rerun without --cache-only to allow an API request"
                ));
            } else {
                assembled.insert(i, (body, timestamp));
                pending.push((i, missing));
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
        let batch: Vec<_> = pending[..count].iter().map(|(_, r)| r).collect();
        if batch.is_empty() {
            return receipts;
        }
        let store = self.store;
        let requests_count = &mut self.requests;
        let paid_input = &mut self.paid_input_tokens;
        let paid_output = &mut self.paid_output_tokens;
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
                    let timestamp = schema::now();
                    for (group, part) in groups(request) {
                        let cached = response::cache_value(&body, &part);
                        store.save(&judgment_key(&part, &group), &cached, timestamp)?;
                        receipt.metrics.evaluated_judgments += 1;
                    }
                    let (mut merged, oldest) = assembled.remove(i).unwrap();
                    merged["answers"]
                        .as_object_mut()
                        .unwrap()
                        .extend(body["answers"].as_object().unwrap().clone());
                    merged["model"] = body["model"].clone();
                    merged["usage"] = body["usage"].clone();
                    Ok((
                        merged,
                        oldest.map_or(timestamp, |old| timestamp.min(old)),
                        false,
                    ))
                });
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
    let state = &request["state"];
    for file in
        std::iter::once(&state["file"]).chain(state["context"].as_array().into_iter().flatten())
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
