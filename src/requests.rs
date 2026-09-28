//! Cached, validated and budgeted TypeSafe requests. Each answer is cached by
//! the state it is about and its question, so a request sends only the
//! questions the cache does not answer.
use crate::{
    evaluate::Session,
    options::CheckArgs,
    response, schema,
    storage::{CacheReader, CachedAnswer},
};
use anyhow::{Result, ensure};
use serde_json::{Map, Value};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

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
    "values",
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

fn hash_of(value: &impl serde::Serialize) -> String {
    schema::hash(&serde_json::to_vec(value).unwrap())
}

/// The key of every answer about one uploaded state: the rubric, the model and
/// the state. TypeSafe answers each question of a request independently of
/// the others, so an answer is kept by its state and question alone: sent
/// whole and one question at a time, five times each, 51 questions of nine
/// JevGate requests moved 0.005 on average, within their own spread across
/// sends (0.007; a permutation test found no batching effect, p = 0.31).
fn state_key(request: &Value) -> String {
    hash_of(&(schema::RUBRIC, &request["model"], &request["state"]))
}

/// A question's key among its state's answers: its name and body.
fn question_key(name: &str, question: &Value) -> String {
    hash_of(&(name, question))
}

/// The key a whole request's answers were cached under before each question
/// was, read to carry them over.
fn request_key(request: &Value) -> String {
    hash_of(&(schema::RUBRIC, provider_request(request)))
}

/// Aliases move to new model versions, so their answers expire. A pinned
/// version answers the same request the same way; its entries never expire.
fn cache_ttl(model: &str, ttl: u64) -> Option<u64> {
    (!crate::model::pinned(model)).then_some(ttl)
}

fn questions(request: &Value) -> impl Iterator<Item = (&String, &Value)> {
    request["questions"].as_object().into_iter().flatten()
}

pub(super) fn question_count(request: &Value) -> u64 {
    questions(request).count() as u64
}

/// Each of `request`'s answers in `body`, as the cache keeps them: given at
/// `created_at`, with an even share of the body's usage, the remainder to
/// the first, so the shares add up to what the request cost.
fn cached_answers<'r>(
    request: &'r Value,
    body: &Value,
    created_at: u64,
) -> Vec<(&'r String, &'r Value, CachedAnswer)> {
    let count = question_count(request).max(1);
    let share = |total: u64, index: u64| total / count + u64::from(index < total % count);
    let input = response::input_tokens(body).unwrap_or(0);
    let output = response::output_tokens(body);
    questions(request)
        .zip(0..)
        .map(|((name, question), index)| {
            let answer = CachedAnswer {
                created_at,
                model: body["model"].as_str().unwrap_or_default().into(),
                answer: response::typed_fields(&body["answers"][name], question),
                input_tokens: share(input, index),
                output_tokens: share(output, index),
            };
            (name, question, answer)
        })
        .collect()
}

/// A cached answer usable for `question` of `request`: well formed, with a
/// real usage count, and from the requested model when a version is pinned.
fn usable(answer: &CachedAnswer, question: &Value, request: &Value) -> bool {
    response::validate_model(&answer.model, request).is_ok()
        && response::validate_answer(&answer.answer, question).is_ok()
        && answer.input_tokens.max(answer.output_tokens) <= response::MAX_REPORTED_TOKENS
}

/// The questions an invocation answered, by state key: with `--refresh` it
/// asks each of them once, and a question two of its requests ask has one
/// answer, the one the cache keeps.
pub(super) type Answered = BTreeMap<String, BTreeSet<String>>;

/// What the cache holds for one planned request.
struct Lookup {
    state: String,
    /// Cached answers by question name.
    found: BTreeMap<String, CachedAnswer>,
    /// The found answers read from the request's whole entry of an earlier
    /// version, by question key, which a run copies under the state's key.
    carried: BTreeMap<String, CachedAnswer>,
    /// Names of the questions no answer is cached for.
    missing: Vec<String>,
}

impl Lookup {
    /// The cached answers to `request`: its state's answers to its
    /// questions, and for the questions they lack, the request's whole entry
    /// of an earlier version. With `--refresh`, only the answers this
    /// invocation gave (`answered`).
    fn new(
        args: &CheckArgs,
        request: &Value,
        cache: Option<&CacheReader>,
        answered: &Answered,
    ) -> Self {
        let mut lookup = Self {
            state: state_key(request),
            found: BTreeMap::new(),
            carried: BTreeMap::new(),
            missing: Vec::new(),
        };
        let Some(cache) = cache else {
            lookup.missing = questions(request).map(|(name, _)| name.clone()).collect();
            return lookup;
        };
        let ttl = cache_ttl(args.model(), args.cache_ttl_secs());
        let stored = cache.answers(&lookup.state, ttl);
        let this_run = answered.get(&lookup.state);
        for (name, question) in questions(request) {
            let key = question_key(name, question);
            let current = !args.refresh || this_run.is_some_and(|keys| keys.contains(&key));
            match stored
                .get(&key)
                .filter(|answer| current && usable(answer, question, request))
            {
                Some(answer) => {
                    lookup.found.insert(name.clone(), answer.clone());
                }
                None => lookup.missing.push(name.clone()),
            }
        }
        if !lookup.missing.is_empty()
            && !args.refresh
            && let Some((body, created_at)) = cache
                .request(&request_key(request), ttl)
                .filter(|(body, _)| response::validate(body, request).is_ok())
        {
            lookup.carry(request, &body, created_at);
        }
        lookup
    }

    /// Answer the missing questions from a valid whole-request `body` given
    /// at `created_at`: an alias's answer does not live longer by being copied.
    fn carry(&mut self, request: &Value, body: &Value, created_at: u64) {
        for (name, question, answer) in cached_answers(request, body, created_at) {
            if self.missing.contains(name) {
                self.carried
                    .insert(question_key(name, question), answer.clone());
                self.found.insert(name.clone(), answer);
            }
        }
        self.missing.clear();
    }

    /// Take the provider's `body` answering `sent`, given at `created_at`,
    /// and return its answers to save, by question key. Where `earlier`, the
    /// answers this invocation already gave about the state, holds one to the
    /// same question, another request asked it too, and that answer is used:
    /// the traces of two security rules about one unit ask one `dev_only`
    /// Noul, and a rerun reads the answer the cache kept.
    fn answered_by(
        &mut self,
        sent: &Value,
        body: &Value,
        created_at: u64,
        earlier: &BTreeMap<String, CachedAnswer>,
    ) -> BTreeMap<String, CachedAnswer> {
        let mut fresh = BTreeMap::new();
        for (name, question, answer) in cached_answers(sent, body, created_at) {
            let key = question_key(name, question);
            match earlier
                .get(&key)
                .filter(|earlier| usable(earlier, question, sent))
            {
                Some(earlier) => {
                    self.found.insert(name.clone(), earlier.clone());
                }
                None => {
                    fresh.insert(key, answer.clone());
                    self.found.insert(name.clone(), answer);
                }
            }
        }
        self.missing.clear();
        fresh
    }

    /// Save the answers the provider's `body` gives to `sent`, except where
    /// this invocation already answered the question about the state, and
    /// return when they were given.
    fn keep(
        &mut self,
        store: &crate::storage::Store,
        answered: &mut Answered,
        sent: &Value,
        body: &Value,
    ) -> Result<u64> {
        let timestamp = schema::now();
        let this_run = answered.entry(self.state.clone()).or_default();
        let mut earlier = store.reader().answers(&self.state, None);
        earlier.retain(|key, _| this_run.contains(key));
        let fresh = self.answered_by(sent, body, timestamp, &earlier);
        let keys: Vec<String> = fresh.keys().cloned().collect();
        store.save_answers(&self.state, fresh)?;
        this_run.extend(keys);
        Ok(timestamp)
    }

    /// `request` with only the questions to ask: none when every one is
    /// answered, and the request itself, not a copy, when none is (every
    /// request of a first run).
    fn unanswered<'r>(&self, request: &'r Value) -> Option<Cow<'r, Value>> {
        if self.missing.is_empty() {
            return None;
        }
        if self.found.is_empty() {
            return Some(Cow::Borrowed(request));
        }
        let mut sent = request.clone();
        sent["questions"] = Value::Object(
            questions(request)
                .filter(|(name, _)| self.missing.contains(name))
                .map(|(name, question)| (name.clone(), question.clone()))
                .collect(),
        );
        Some(Cow::Owned(sent))
    }

    /// A response body answering the planned request from the found answers,
    /// and when its newest answer was given: the answers by question name,
    /// the model of the newest, and the sum of their usage shares.
    fn body(&self, request: &Value) -> (Value, u64) {
        let newest = self
            .found
            .values()
            .max_by(|a, b| (a.created_at, &a.model).cmp(&(b.created_at, &b.model)));
        let model = newest.map_or_else(
            || request["model"].clone(),
            |answer| Value::String(answer.model.clone()),
        );
        let answers: Map<String, Value> = self
            .found
            .iter()
            .map(|(name, answer)| (name.clone(), answer.answer.clone()))
            .collect();
        let input: u64 = self.found.values().map(|a| a.input_tokens).sum();
        let output: u64 = self.found.values().map(|a| a.output_tokens).sum();
        let body = serde_json::json!({
            "model": model,
            "answers": answers,
            "usage": {"input_tokens": input, "output_tokens": output},
        });
        (body, newest.map_or(0, |answer| answer.created_at))
    }
}

/// The part of `request` a run would send: the request with only the
/// questions the cache does not answer, or none when it answers them all.
/// Read without opening the store, for planning and dry runs.
pub(super) fn unanswered(
    root: &std::path::Path,
    args: &CheckArgs,
    request: &Value,
) -> Option<Value> {
    peek(root, args, request)
        .unanswered(request)
        .map(Cow::into_owned)
}

/// A dry run's cached answer to a whole planned request, read without
/// opening the store; none while any question is unanswered.
pub(super) fn cached(root: &std::path::Path, args: &CheckArgs, request: &Value) -> Option<Value> {
    let lookup = peek(root, args, request);
    lookup.missing.is_empty().then(|| lookup.body(request).0)
}

/// The cached answers to `request`, read without opening the store while
/// planning and in dry runs, before the invocation has answered anything.
fn peek(root: &std::path::Path, args: &CheckArgs, request: &Value) -> Lookup {
    Lookup::new(
        args,
        request,
        CacheReader::peek(root).as_ref(),
        &Answered::new(),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Value {
        json!({"model":"jev-1.13.0","state":{"source":"fn f() {}"},
            "questions":{"a":{"type":"noul","instructions":{"question":"Is it empty?"}}}})
    }

    #[test]
    fn pinned_answers_do_not_expire_and_aliases_do() {
        let project = crate::tests::Project::new();
        let store = crate::storage::Store::open(&project.0).unwrap();
        let body = json!({"model":"jev-1.13.0","answers":{}});
        store
            .save_request("old", &body, schema::now() - 7200)
            .unwrap();
        let answer = CachedAnswer {
            created_at: schema::now() - 7200,
            model: "jev-1.13.0".into(),
            answer: json!({"type":"noul","noul":0.1}),
            input_tokens: 1,
            output_tokens: 0,
        };
        store
            .save_answers("state", [("q".to_string(), answer)].into())
            .unwrap();
        let cache = store.reader();
        for (model, kept) in [
            ("jev-1.13.0", true),
            ("typesafe/jev-1.13.0", true),
            ("jev-latest", false),
            ("jev-preview", false),
            ("jev-1.13", false),
            ("typesafe/jev-1.13", false),
            ("typesafe-ai/jev", false),
        ] {
            let ttl = cache_ttl(model, 3600);
            assert_eq!(cache.request("old", ttl).is_some(), kept, "{model}");
            assert_eq!(
                cache.answers("state", ttl).len(),
                usize::from(kept),
                "{model}"
            );
        }
        let day = cache_ttl("jev-latest", 86_400);
        assert!(cache.request("old", day).is_some());
        assert_eq!(cache.answers("state", day).len(), 1);
        assert!(cache.request("other", None).is_none());
        assert!(cache.answers("other", None).is_empty());
    }

    #[test]
    fn local_metadata_is_not_uploaded_or_part_of_the_cache_keys() {
        let plain = json!({"model":"m","state":{"a":1},"questions":{}});
        let mut tagged = plain.clone();
        tagged["jevgate"] = json!({"stage":"functions","sources":[]});
        assert_eq!(*provider_request(&tagged), plain);
        assert_eq!(request_key(&tagged), request_key(&plain));
        assert_eq!(state_key(&tagged), state_key(&plain));
        assert_eq!(stage(&tagged), "functions");
    }

    #[test]
    fn earlier_whole_request_keys_are_read_unchanged() {
        // The key 0.25.0 saved this request's answers under: a change here
        // would make every existing cache entry unreadable.
        assert_eq!(
            request_key(&request()),
            "13e78f94719641fa4608e49d17958374cab141b93b4130f30ecbac6ba98017c7"
        );
    }

    #[test]
    fn a_question_is_keyed_by_its_state_name_and_body_alone() {
        let first = request();
        let mut other = first.clone();
        other["questions"]["b"] = json!({"type":"noul","instructions":{"question":"Is it long?"}});
        assert_eq!(state_key(&first), state_key(&other));
        assert_ne!(request_key(&first), request_key(&other));
        let body = &first["questions"]["a"];
        assert_ne!(question_key("a", body), question_key("b", body));
        let mut moved = first.clone();
        moved["state"]["source"] = json!("fn g() {}");
        assert_ne!(state_key(&first), state_key(&moved));
        let mut model = first.clone();
        model["model"] = json!("jev-latest");
        assert_ne!(state_key(&first), state_key(&model));
    }

    #[test]
    fn usage_is_shared_among_the_questions_it_paid_for() {
        let mut request = request();
        for name in ["b", "c"] {
            request["questions"][name] = request["questions"]["a"].clone();
        }
        let body = json!({"model":"jev-1.13.0","usage":{"input_tokens":301,"output_tokens":2},
            "answers":{"a":{"type":"noul","noul":0.1,"extra":1},"b":{"type":"noul","noul":0.2},"c":{"type":"noul","noul":0.3}}});
        let answers = cached_answers(&request, &body, 7);
        let inputs: Vec<u64> = answers.iter().map(|(_, _, a)| a.input_tokens).collect();
        let outputs: Vec<u64> = answers.iter().map(|(_, _, a)| a.output_tokens).collect();
        assert_eq!((inputs, outputs), (vec![101, 100, 100], vec![1, 1, 0]));
        assert_eq!(answers[0].2.answer, json!({"type":"noul","noul":0.1}));
        assert!(answers.iter().all(|(_, _, a)| a.created_at == 7));
    }
}
