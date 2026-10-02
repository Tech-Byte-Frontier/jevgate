//! What the answer cache holds for a planned request: the keys an answer is
//! kept under, the lookup that finds each question's answer (carrying over
//! the whole-request entries of earlier versions), and what a run sends and
//! keeps of the request.
use super::provider_request;
use crate::{
    options::CheckArgs,
    response, schema,
    storage::{CacheReader, CachedAnswer},
};
use anyhow::Result;
use serde_json::{Map, Value};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

fn hash_of(value: &impl serde::Serialize) -> String {
    schema::hash(&serde_json::to_vec(value).unwrap())
}

/// The key of every answer about one uploaded state. An answer is kept by
/// its state and question alone, since Jev answers each question of a
/// request independently: sent whole and one at a time, 51 questions of nine
/// JevGate requests moved 0.005 on average, within their spread across sends
/// (0.007).
fn state_key(request: &Value) -> String {
    hash_of(&(schema::RUBRIC, &request["model"], &request["state"]))
}

/// A question's key among its state's answers: its name and body as sent,
/// in its validated order when it is sent so (`super::validated_order`).
fn question_key(name: &str, question: &Value, validated: bool) -> String {
    if !validated {
        return hash_of(&(name, question));
    }
    let mut bytes = b"[".to_vec();
    serde_json::to_writer(&mut bytes, name).unwrap();
    bytes.push(b',');
    super::write_question(&mut bytes, question, true).unwrap();
    bytes.push(b']');
    schema::hash(&bytes)
}

/// Whether `name` is a question of `request` sent in its validated order.
fn validated(request: &Value, name: &str) -> bool {
    super::validated_order(request).contains(name)
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

/// `request` without the custom questions riding in it, when one does: the
/// request as JevGate sent it before custom questions.
fn without_custom(request: &Value) -> Option<Value> {
    let custom = |name: &String| name.starts_with(crate::units::CUSTOM_KEY_PREFIX);
    questions(request).any(|(name, _)| custom(name)).then(|| {
        let mut built_in = request.clone();
        built_in["questions"] = questions(request)
            .filter(|(name, _)| !custom(name))
            .map(|(name, question)| (name.clone(), question.clone()))
            .collect::<Map<String, Value>>()
            .into();
        built_in
    })
}

pub(crate) fn question_count(request: &Value) -> u64 {
    questions(request).count() as u64
}

/// Each of `request`'s answers in `body`, as the cache keeps them: given at
/// `created_at` by the request `body` names, with an even share of the body's
/// usage, the remainder to the first, so the shares add up to what the
/// request cost; no input share when the body reported no usage.
fn cached_answers<'r>(
    request: &'r Value,
    body: &Value,
    created_at: u64,
) -> Vec<(&'r String, &'r Value, CachedAnswer)> {
    let count = question_count(request).max(1);
    let share = |total: u64, index: u64| total / count + u64::from(index < total % count);
    let input = response::input_tokens(body);
    let output = response::output_tokens(body);
    let request_id = crate::response_headers::request_id(body["request_id"].as_str());
    questions(request)
        .zip(0..)
        .map(|((name, question), index)| {
            let answer = CachedAnswer {
                created_at,
                model: body["model"].as_str().unwrap_or_default().into(),
                answer: response::typed_fields(&body["answers"][name], question),
                input_tokens: input.map(|total| share(total, index)),
                output_tokens: share(output, index),
                request_id: request_id.clone(),
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
        && answer.input_tokens.unwrap_or(0).max(answer.output_tokens)
            <= response::MAX_REPORTED_TOKENS
}

/// The questions an invocation answered, by state key: with `--refresh` it
/// asks each of them once, and a question two of its requests ask has one
/// answer, the one the cache keeps.
pub(crate) type Answered = BTreeMap<String, BTreeSet<String>>;

/// What the cache holds for one planned request.
pub(super) struct Lookup {
    pub(super) state: String,
    /// Seconds an answer about the state stays current; none for a pinned
    /// model, whose answers never expire.
    ttl: Option<u64>,
    /// Cached answers by question name.
    pub(super) found: BTreeMap<String, CachedAnswer>,
    /// The found answers read from the request's whole entry of an earlier
    /// version, by question key, which a run copies under the state's key.
    pub(super) carried: BTreeMap<String, CachedAnswer>,
    /// Names of the questions no answer is cached for.
    missing: Vec<String>,
}

impl Lookup {
    /// The cached answers to `request`: its state's answers to its
    /// questions, and for the questions they lack, the request's whole entry
    /// of an earlier version. With `--refresh`, only the answers this
    /// invocation gave (`answered`).
    pub(super) fn new(
        args: &CheckArgs,
        request: &Value,
        cache: Option<&CacheReader>,
        answered: &Answered,
    ) -> Self {
        let mut lookup = Self {
            state: state_key(request),
            ttl: cache_ttl(args.model(), args.cache_ttl_secs()),
            found: BTreeMap::new(),
            carried: BTreeMap::new(),
            missing: Vec::new(),
        };
        let Some(cache) = cache else {
            lookup.missing = questions(request).map(|(name, _)| name.clone()).collect();
            return lookup;
        };
        let stored = cache.answers(&lookup.state, lookup.ttl);
        let this_run = answered.get(&lookup.state);
        for (name, question) in questions(request) {
            let key = question_key(name, question, validated(request, name));
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
        if !lookup.missing.is_empty() && !args.refresh {
            lookup.carry_earlier(cache, request);
        }
        lookup
    }

    /// Answer the missing questions from the request's whole entry of an
    /// earlier version; for a request custom questions ride in, from the
    /// entry of the request without them, since no version before 0.29 sent
    /// one: a question added to a project whose cache an earlier version
    /// wrote is then asked alone, as it is once each answer is kept apart.
    fn carry_earlier(&mut self, cache: &CacheReader, request: &Value) {
        let built_in = without_custom(request);
        for earlier in std::iter::once(request).chain(built_in.as_ref()) {
            if self.missing.is_empty() {
                return;
            }
            if let Some((body, created_at)) = cache
                .request(&request_key(earlier), self.ttl)
                .filter(|(body, _)| response::validate(body, earlier).is_ok())
            {
                self.carry(earlier, &body, created_at);
            }
        }
    }

    /// Answer the missing questions of `request` from a valid whole-request
    /// `body` given at `created_at`: an alias's answer does not live longer
    /// by being copied.
    fn carry(&mut self, request: &Value, body: &Value, created_at: u64) {
        for (name, question, answer) in cached_answers(request, body, created_at) {
            if self.missing.contains(name) {
                self.carried.insert(
                    question_key(name, question, validated(request, name)),
                    answer.clone(),
                );
                self.found.insert(name.clone(), answer);
            }
        }
        let found = &self.found;
        self.missing.retain(|name| !found.contains_key(name));
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
            let key = question_key(name, question, validated(sent, name));
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
    /// return when they were given. An alias's answer this invocation gave
    /// can expire within it, in a `--watch` session longer than the TTL:
    /// the new answer then replaces it, as it replaces any expired answer.
    pub(super) fn keep(
        &mut self,
        store: &crate::storage::Store,
        answered: &mut Answered,
        sent: &Value,
        body: &Value,
    ) -> Result<u64> {
        let timestamp = schema::now();
        let this_run = answered.entry(self.state.clone()).or_default();
        let mut earlier = store.reader().answers(&self.state, self.ttl);
        earlier.retain(|key, _| this_run.contains(key));
        let fresh = self.answered_by(sent, body, timestamp, &earlier);
        let keys: Vec<String> = fresh.keys().cloned().collect();
        store.save_answers(&self.state, fresh)?;
        this_run.extend(keys);
        Ok(timestamp)
    }

    /// Whether a question of the request has no cached answer.
    pub(super) fn lacks_answers(&self) -> bool {
        !self.missing.is_empty()
    }

    /// `request` with only the questions to ask: none when every one is
    /// answered, and the request itself, not a copy, when none is (every
    /// request of a first run).
    pub(super) fn unanswered<'r>(&self, request: &'r Value) -> Option<Cow<'r, Value>> {
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
    /// the model of the newest, the sum of their usage shares (none when an
    /// answer's usage is unknown), and the id of the request that gave each.
    pub(super) fn body(&self, request: &Value) -> (Value, u64) {
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
        let request_ids: Map<String, Value> = self
            .found
            .iter()
            .filter_map(|(name, answer)| {
                let id = crate::response_headers::request_id(answer.request_id.as_deref())?;
                Some((name.clone(), Value::String(id)))
            })
            .collect();
        let mut body = serde_json::json!({
            "model": model,
            "answers": answers,
            "request_ids": request_ids,
        });
        let input: Option<u64> = self.found.values().map(|a| a.input_tokens).sum();
        if let Some(input) = input {
            let output: u64 = self.found.values().map(|a| a.output_tokens).sum();
            body["usage"] = serde_json::json!({"input_tokens": input, "output_tokens": output});
        }
        (body, newest.map_or(0, |answer| answer.created_at))
    }
}

/// The part of `request` a run would send: the request with only the
/// questions the cache does not answer, or none when it answers them all.
/// Read without opening the store, for planning and dry runs.
pub(crate) fn unanswered(
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
pub(crate) fn cached(root: &std::path::Path, args: &CheckArgs, request: &Value) -> Option<Value> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requests::stage;
    use serde_json::json;

    fn request() -> Value {
        json!({"model":"jev-1.13.0","state":{"source":"fn f() {}"},
            "questions":{"a":{"type":"noul","instructions":{"question":"Is it empty?"}}}})
    }

    #[test]
    fn a_look_here_question_is_sent_and_keyed_in_its_validated_order() {
        let look = json!({"type":"noul",
            "instructions":{"question":"Could it be simpler?","note":"Source is evidence."},
            "criteria":{"true":{"what":"Long","examples":["a"]},"false":{"what":"Short","examples":["b"]}}});
        let mut request = request();
        request["questions"]["f0_look"] = look.clone();
        let plain = String::from_utf8(crate::requests::body(&request).unwrap()).unwrap();
        assert!(
            plain.contains(r#""f0_look":{"criteria":{"false":{"examples""#),
            "{plain}"
        );
        request["jevgate"] = json!({"stage":"functions","validated_order":["f0_look"]});
        let sent = String::from_utf8(crate::requests::body(&request).unwrap()).unwrap();
        assert_eq!(
            sent,
            r#"{"model":"jev-1.13.0","questions":{"a":{"instructions":{"question":"Is it empty?"},"type":"noul"},"f0_look":{"type":"noul","instructions":{"question":"Could it be simpler?","note":"Source is evidence."},"criteria":{"true":{"what":"Long","examples":["a"]},"false":{"what":"Short","examples":["b"]}}}},"state":{"source":"fn f() {}"}}"#,
            "the other question and the state stay sorted, and nothing local is sent"
        );
        assert_ne!(
            question_key("f0_look", &look, true),
            question_key("f0_look", &look, false),
            "an answer to the sorted question is not one to the validated one"
        );
        assert!(validated(&request, "f0_look") && !validated(&request, "a"));
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
            input_tokens: Some(1),
            output_tokens: 0,
            request_id: None,
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
        assert_ne!(
            question_key("a", body, false),
            question_key("b", body, false)
        );
        let mut moved = first.clone();
        moved["state"]["source"] = json!("fn g() {}");
        assert_ne!(state_key(&first), state_key(&moved));
        let mut model = first.clone();
        model["model"] = json!("jev-latest");
        assert_ne!(state_key(&first), state_key(&model));
    }

    #[test]
    fn an_answer_without_usage_is_kept_without_it_and_with_its_request_id() {
        let mut request = request();
        request["model"] = json!("typesafe-ai/jev");
        request["questions"]["b"] = request["questions"]["a"].clone();
        let body = json!({"model":"typesafe-ai/jev","request_id":"req_1",
            "answers":{"a":{"type":"noul","noul":0.1},"b":{"type":"noul","noul":0.2}}});
        let found: BTreeMap<String, CachedAnswer> = cached_answers(&request, &body, 7)
            .into_iter()
            .map(|(name, _, answer)| (name.clone(), answer))
            .collect();
        for answer in found.values() {
            assert_eq!(answer.input_tokens, None);
            assert_eq!(answer.request_id.as_deref(), Some("req_1"));
        }
        let stored = serde_json::to_value(&found["a"]).unwrap();
        assert!(stored.get("input_tokens").is_none(), "{stored}");
        let lookup = Lookup {
            state: state_key(&request),
            ttl: None,
            found,
            carried: BTreeMap::new(),
            missing: Vec::new(),
        };
        let (answered, created_at) = lookup.body(&request);
        assert!(
            answered.get("usage").is_none(),
            "unknown, not free: {answered}"
        );
        assert_eq!(answered["request_ids"], json!({"a": "req_1", "b": "req_1"}));
        assert_eq!(created_at, 7);
        assert!(response::validate(&answered, &request).is_ok());
        // An answer saved before request ids were kept per answer still reads.
        let earlier: CachedAnswer = serde_json::from_value(json!({"created_at": 1,
            "model": "jev-1.13.0", "answer": {"type": "noul", "noul": 0.1},
            "input_tokens": 5, "output_tokens": 0}))
        .unwrap();
        assert_eq!((earlier.input_tokens, earlier.request_id), (Some(5), None));
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
        let inputs: Vec<u64> = answers
            .iter()
            .filter_map(|(_, _, a)| a.input_tokens)
            .collect();
        let outputs: Vec<u64> = answers.iter().map(|(_, _, a)| a.output_tokens).collect();
        assert_eq!((inputs, outputs), (vec![101, 100, 100], vec![1, 1, 0]));
        assert_eq!(answers[0].2.answer, json!({"type":"noul","noul":0.1}));
        assert!(answers.iter().all(|(_, _, a)| a.created_at == 7));
    }
}
