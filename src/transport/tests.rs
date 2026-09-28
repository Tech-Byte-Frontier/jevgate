//! The request queue, retries and access failures, and whole exchanges with a
//! mock provider over HTTP.
use super::*;
use crate::tests::mock_provider::{MockProvider, Received, Reply};
use serde_json::json;

/// A TypeSafe failure with `status`, `body` and a pause in seconds.
fn failed(status: u16, body: Option<&str>, pause: Option<u64>) -> ProviderError {
    let failure = Failure {
        status,
        body,
        retry_after: pause.map(Duration::from_secs),
        request_id: None,
    };
    provider_error(&TYPESAFE, failure)
}

/// Retries and starts without the production pauses.
fn fast() -> ProviderAccess {
    ProviderAccess {
        backoff: Duration::from_millis(1),
        interval: Duration::ZERO,
        ..Default::default()
    }
}

fn sends(access: &ProviderAccess, results: Vec<Result<Value>>) -> (Outcome, usize) {
    let request = json!({"index":0});
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let results = Mutex::new(results.into_iter());
    let mut last = None;
    access.evaluate_queue(
        &[&request],
        1,
        &|_| Ok(()),
        |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            results.lock().unwrap().next().unwrap()
        },
        &mut |_, outcome| last = Some(outcome),
    );
    (last.unwrap(), calls.load(Ordering::Relaxed))
}

#[test]
fn rate_limits_retry_after_the_requested_pause_and_count_retries() {
    let access = fast();
    let start = Instant::now();
    let (outcome, calls) = sends(
        &access,
        vec![
            Err(failed(429, None, Some(1)).into()),
            Ok(json!({"answers":{}})),
        ],
    );
    assert!(outcome.result.is_ok());
    assert_eq!((calls, outcome.retries), (2, 1));
    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "retry-after is honored"
    );
    for (status, error) in [
        (529, failed(529, None, None)),
        (502, failed(502, None, None)),
    ] {
        let (outcome, calls) = sends(&access, vec![Err(error.into()), Ok(json!({}))]);
        assert_eq!((calls, outcome.retries), (2, 1), "{status}");
    }
    let (outcome, calls) = sends(&access, vec![Err(Unsent(&TYPESAFE).into()), Ok(json!({}))]);
    assert_eq!((calls, outcome.retries), (2, 1), "connection never opened");
    assert_eq!(
        retry_delay(&failed(429, None, Some(3600)).into()),
        Some((Some(RETRY_AFTER_CAP), ANSWERED_ATTEMPTS))
    );
    assert_eq!(
        retry_delay(&failed(503, None, Some(20)).into()),
        Some((Some(Duration::from_secs(20)), ANSWERED_ATTEMPTS)),
        "a pause longer than the longest backoff is still the provider's"
    );
}

#[test]
fn server_errors_retry_and_an_interrupted_request_is_sent_twice_at_most() {
    for status in [500, 520, 522, 524] {
        let (outcome, calls) = sends(
            &fast(),
            vec![Err(failed(status, None, None).into()), Ok(json!({}))],
        );
        assert!(outcome.result.is_ok(), "{status}");
        assert_eq!((calls, outcome.retries), (2, 1), "{status}");
    }
    let (outcome, calls) = sends(
        &fast(),
        vec![Err(Interrupted(&TYPESAFE).into()), Ok(json!({}))],
    );
    assert!(outcome.result.is_ok());
    assert_eq!(calls, 2, "a timeout passes on its second send");
    let failures = (0..4).map(|_| Err(Interrupted(&TYPESAFE).into())).collect();
    let (outcome, calls) = sends(&fast(), failures);
    assert_eq!(calls, INTERRUPTED_ATTEMPTS as usize);
    let message = outcome.result.unwrap_err().to_string();
    assert!(message.contains("timed out") && message.contains("gave up after 2 attempts"));
}

#[test]
fn a_retry_or_a_pause_past_the_deadline_is_not_waited() {
    let access = ProviderAccess {
        deadline: Some(Instant::now() + Duration::from_secs(2)),
        ..fast()
    };
    let start = Instant::now();
    let (outcome, calls) = sends(
        &access,
        vec![Err(failed(503, None, Some(30)).into()), Ok(json!({}))],
    );
    assert_eq!((calls, outcome.retries), (1, 0));
    assert!(start.elapsed() < Duration::from_secs(2));
    let error = outcome.result.unwrap_err();
    assert!(
        error.to_string().starts_with("TypeSafe HTTP 503") && transient(&error),
        "the provider's own failure: {error}"
    );
    access.pause(Duration::from_secs(30));
    let (outcome, calls) = sends(&access, vec![Ok(json!({}))]);
    assert_eq!(calls, 0, "a request behind another's pause is not sent");
    let message = outcome.result.unwrap_err().to_string();
    assert!(
        message.contains("past the time this check has"),
        "{message}"
    );
}

#[test]
fn validation_transport_and_account_errors_are_sent_once() {
    for error in [
        anyhow::Error::from(failed(422, None, None)),
        failed(400, None, Some(1)).into(),
        failed(401, None, None).into(),
        anyhow::anyhow!("TypeSafe transport failure; request was not retried"),
    ] {
        let text = error.to_string();
        let (outcome, calls) = sends(&fast(), vec![Err(error), Ok(json!({}))]);
        assert_eq!((calls, outcome.retries), (1, 0), "{text}");
        assert!(outcome.result.is_err());
    }
}

#[test]
fn persistent_overload_stops_after_the_attempt_limit() {
    let failures = (0..ANSWERED_ATTEMPTS + 2)
        .map(|_| Err(failed(503, None, None).into()))
        .collect();
    let (outcome, calls) = sends(&fast(), failures);
    assert_eq!(calls, ANSWERED_ATTEMPTS as usize);
    assert_eq!(outcome.retries, ANSWERED_ATTEMPTS - 1);
    let message = outcome.result.unwrap_err().to_string();
    assert!(message.contains("HTTP 503") && message.contains("gave up after 6 attempts"));
    let unsent = (0..ANSWERED_ATTEMPTS)
        .map(|_| Err(Unsent(&TYPESAFE).into()))
        .collect();
    let (outcome, calls) = sends(&fast(), unsent);
    assert_eq!(
        calls, UNSENT_ATTEMPTS as usize,
        "an offline run ends as soon as before"
    );
    let message = outcome.result.unwrap_err().to_string();
    assert!(message.contains("was not sent; gave up after 4 attempts"));
}

#[test]
fn pauses_double_from_one_second_to_at_most_eight() {
    let access = ProviderAccess::default();
    let pauses: Vec<_> = (1..ANSWERED_ATTEMPTS)
        .map(|retry| access.backoff(0, retry).as_millis())
        .collect();
    // 1, 2, 4, 8 and 8 s, each lengthened by its jitter; the first three are
    // the pauses 0.25 made.
    assert_eq!(pauses, [1101, 2404, 4212, 9232, 8040]);
    for (retry, seconds) in (1..ANSWERED_ATTEMPTS).zip([1, 2, 4, 8, 8]) {
        let base = Duration::from_secs(seconds);
        for index in 0..MAX_WORKERS * 4 {
            let pause = access.backoff(index, retry);
            assert!(pause >= base && pause < base * 5 / 4, "{retry} {index}");
        }
    }
}

#[test]
fn account_rejections_stop_pending_uploads_but_keep_in_flight_successes() {
    use std::sync::{Barrier, Condvar, Mutex, atomic::AtomicUsize};
    let access = fast();
    let requests: Vec<_> = (0..24).map(|i| json!({"index":i})).collect();
    let batch: Vec<_> = requests.iter().collect();
    let first_four = Barrier::new(4);
    let released = (Mutex::new(false), Condvar::new());
    let calls = AtomicUsize::new(0);
    let mut outcomes = Vec::new();
    access.evaluate_queue(
        &batch,
        4,
        &|_| Ok(()),
        |request| {
            calls.fetch_add(1, Ordering::Relaxed);
            let index = request["index"].as_u64().unwrap();
            if index < 4 {
                first_four.wait();
                if index == 0 {
                    return Err(failed(402, None, None).into());
                }
                let (released, timeout) = released
                    .1
                    .wait_timeout_while(
                        released.0.lock().unwrap(),
                        Duration::from_secs(5),
                        |done| !*done,
                    )
                    .unwrap();
                assert!(*released && !timeout.timed_out());
            }
            Ok(request.clone())
        },
        &mut |index, outcome| {
            if index == 0 {
                *released.0.lock().unwrap() = true;
                released.1.notify_all();
            }
            outcomes.push((index, outcome));
        },
    );
    outcomes.sort_by_key(|(index, _)| *index);
    assert_eq!(calls.load(Ordering::Relaxed), 4);
    assert_eq!(outcomes.len(), 24);
    assert!(outcomes[0].1.attempted && outcomes[0].1.result.is_err());
    for (_, outcome) in &outcomes[1..4] {
        assert!(outcome.attempted && outcome.result.is_ok());
    }
    for (_, outcome) in &outcomes[4..] {
        assert!(!outcome.attempted);
        assert_eq!(outcome.elapsed_ms, 0);
        assert!(
            outcome
                .result
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("not sent after HTTP 402")
        );
    }
    access.evaluate_queue(
        &batch,
        4,
        &|_| panic!("stopped before freshness work"),
        |_| panic!("stopped across later stages"),
        &mut |_, outcome| assert!(!outcome.attempted),
    );
}

#[test]
fn only_typed_account_errors_stop_siblings_and_a_new_review_can_retry() {
    let requests = [json!({"index":0}), json!({"index":1})];
    let batch: Vec<_> = requests.iter().collect();
    for status in [400, 401, 402, 403, 422, 429, 503, 529] {
        let mut access = fast();
        let mut attempts = 0;
        access.evaluate_queue(
            &batch,
            1,
            &|_| Ok(()),
            |request| {
                if request["index"] == 0 {
                    Err(anyhow::Error::new(failed(status, None, None)).context("provider response"))
                } else {
                    Ok(request.clone())
                }
            },
            &mut |_, outcome| attempts += usize::from(outcome.attempted),
        );
        let rejected = matches!(status, 401..=403);
        assert_eq!(attempts, if rejected { 1 } else { 2 }, "{status}");
        assert_eq!(access.reset(), rejected);
        access.evaluate_queue(
            &batch,
            1,
            &|_| Ok(()),
            |request| Ok(request.clone()),
            &mut |_, outcome| assert!(outcome.attempted && outcome.result.is_ok()),
        );
    }
    let access = fast();
    let mut attempts = 0;
    access.evaluate_queue(
        &batch,
        1,
        &|_| Ok(()),
        |_| Err(anyhow::anyhow!("source text says TypeSafe HTTP 402")),
        &mut |_, outcome| attempts += usize::from(outcome.attempted),
    );
    assert_eq!(attempts, 2, "arbitrary error text cannot close the queue");
    access.evaluate_queue(
        &batch,
        1,
        &|request| {
            if request["index"] == 0 {
                bail!("stale source")
            }
            Ok(())
        },
        |request| Ok(request.clone()),
        &mut |index, outcome| {
            assert_eq!(outcome.attempted, index == 1);
        },
    );
}

#[test]
fn rejected_review_keeps_cached_judgments_and_recovers_only_unfinished_work() {
    use crate::tests::{Project, answer, args, run};
    struct Provider {
        access: ProviderAccess,
        reject: bool,
    }
    impl Evaluator for Provider {
        fn begin_review(&mut self) {
            self.access.reset();
        }
        fn evaluate(&mut self, _: &Value) -> Result<Value> {
            unreachable!("queue path")
        }
        fn evaluate_queue(
            &mut self,
            requests: &[&Value],
            concurrency: usize,
            before: &(dyn Fn(&Value) -> Result<()> + Sync),
            completed: &mut dyn FnMut(usize, Outcome),
        ) {
            self.access.evaluate_queue(
                requests,
                concurrency,
                before,
                |request| {
                    if self.reject {
                        Err(failed(402, None, None).into())
                    } else {
                        Ok(answer(request, 0))
                    }
                },
                completed,
            );
        }
    }
    let project = Project::new();
    for name in ["a", "b", "c", "d"] {
        project.write(&format!("{name}.py"), &format!("def {name}(fn):\n    try:\n        return fn()\n    except OSError:\n        log(fn)\n        raise\n"));
    }
    project.write("b.py", "def b(fn):\n    try:\n        return fn()\n    except OSError:\n        log(fn)\n        raise\n\ndef second(fn):\n    try:\n        return fn()\n    except ValueError:\n        log(fn)\n        raise\n");
    let mut options = args();
    options.quick = true;
    options.rules = vec!["function_simplification".into()];
    options.concurrency = Some(1);
    options.paths = vec!["a.py".into()];
    let mut provider = Provider {
        access: fast(),
        reject: false,
    };
    let warm = run(&project, &options, &mut provider);
    assert!(warm.complete);
    assert_eq!(warm.api_requests, 1);
    options.paths.clear();
    provider.reject = true;
    let rejected = run(&project, &options, &mut provider);
    assert!(!rejected.complete);
    assert_eq!(rejected.api_requests, 1);
    assert_eq!(rejected.files[0].status, crate::schema::Status::Clear);
    assert!(rejected.files[0].cached);
    assert!(
        rejected.files[1..]
            .iter()
            .all(|f| f.status == crate::schema::Status::Error)
    );
    assert_eq!(rejected.stages["functions"].failed_attempts, 1);
    assert_eq!(rejected.stages["functions"].cache_hits, 1);
    assert_eq!(
        rejected.files[1].error.as_deref(),
        Some(
            "TypeSafe HTTP 402 (credits exhausted; add credits or turn on auto-refill at https://console.typesafe.ai); request was not retried"
        ),
        "later unsent work in the same file must not hide the original provider failure"
    );
    assert!(rejected.files[2..].iter().all(|f| {
        f.error
            .as_ref()
            .unwrap()
            .contains("not sent after HTTP 402 (credits exhausted); add credits")
    }));
    let saved = crate::storage::read_latest(&project.0).unwrap();
    assert!(!saved.complete);
    assert_eq!(saved.api_requests, 1);
    provider.reject = false;
    let recovered = run(&project, &options, &mut provider);
    assert!(recovered.complete);
    assert_eq!(
        recovered.api_requests, 3,
        "failed and unsent requests were not cached"
    );
    options.cache_only = true;
    let replay = run(&project, &options, &mut provider);
    assert!(replay.complete);
    assert_eq!(replay.api_requests, 0);
    for (expected, actual) in recovered.files.iter().zip(replay.files) {
        assert_eq!(json!(expected.dimensions), json!(actual.dimensions));
    }
}

#[test]
fn a_context_limit_error_is_named_without_echoing_private_text() {
    let body = json!({"detail":{"error_type":"max_tokens_exceeded","message":"private source and credentials"}});
    assert_eq!(
        failed(400, Some(&body.to_string()), None).to_string(),
        "TypeSafe HTTP 400 (model context limit exceeded); request was not retried"
    );
}

#[test]
fn unknown_error_details_are_not_echoed() {
    for body in [
        json!({"detail":"private source"}),
        json!({"detail":{"error_type":"private credentials"}}),
    ] {
        assert_eq!(
            failed(400, Some(&body.to_string()), None).to_string(),
            "TypeSafe HTTP 400; request was not retried"
        );
    }
    assert_eq!(failed(503, None, None).to_string(), "TypeSafe HTTP 503");
}

const EDGE_PAGE: &str =
    "<!DOCTYPE html><html><head><title>Attention Required! | Cloudflare</title></head></html>";

#[test]
fn request_bodies_are_compact_without_local_metadata() {
    let request = json!({"model": "m", "state": {"source": "<a b={c} />"}, "jevgate": {}});
    let body = String::from_utf8(request_body(&request).unwrap()).unwrap();
    assert_eq!(body, r#"{"model":"m","state":{"source":"<a b={c} />"}}"#);
}

#[test]
fn edge_firewall_blocks_are_told_apart_from_account_rejections() {
    let edge = failed(403, Some("error code: 1010\n"), None);
    assert!(edge.edge_block);
    assert_eq!(
        edge.to_string(),
        "TypeSafe HTTP 403 (blocked by the provider's edge protection); request was not retried"
    );
    assert!(failed(403, Some(EDGE_PAGE), None).edge_block);
    assert!(!failed(403, Some("{\"detail\":\"forbidden\"}"), None).edge_block);
}

#[test]
fn isolated_edge_blocks_fail_alone_and_consecutive_blocks_stop_uploads() {
    let access = ProviderAccess::default();
    let blocked: Result<Value> = Err(failed(403, Some(EDGE_PAGE), None).into());
    access.observe(&blocked);
    access.observe(&blocked);
    access.observe(&Ok(json!({})));
    access.observe(&blocked);
    assert!(access.check().is_ok(), "a success resets the count");
    access.observe(&blocked);
    access.observe(&blocked);
    assert!(
        access
            .check()
            .unwrap_err()
            .to_string()
            .contains("edge protection")
    );
}

/// A mock provider answering with `respond`, and a TypeSafe endpoint at it.
fn mock(respond: impl Fn(&Received) -> Reply + Send + Sync + 'static) -> (MockProvider, Endpoint) {
    let provider = MockProvider::start(respond);
    let endpoint = Endpoint::custom(&TYPESAFE, &provider.url).unwrap();
    (provider, endpoint)
}

/// A one-question request, with local metadata that must not be uploaded.
fn question() -> Value {
    json!({"model": "jev-1.13.0", "state": "x", "jevgate": {"stage": "functions"},
        "questions": {"q": {"type": "noul", "instructions": "?"}}})
}

/// A valid answer to the request the provider received.
fn answered(received: &Received) -> Reply {
    Reply::json(200, &crate::tests::answer(&received.json(), 0))
}

#[test]
fn an_exchange_sends_the_bearer_key_and_keeps_only_a_checked_request_id() {
    let (provider, endpoint) =
        mock(|received| answered(received).header(REQUEST_ID, "req_01J9-abc"));
    let answer = send(&agent(ATTEMPT_TIMEOUT), &endpoint, "test-key", &question()).unwrap();
    assert_eq!(answer["request_id"], "req_01J9-abc");
    assert!(crate::response::validate(&answer, &question()).is_ok());
    let received = &provider.received()[0];
    assert_eq!(
        (received.method.as_str(), received.path.as_str()),
        ("POST", "/v1/systemone")
    );
    assert_eq!(received.header("Authorization"), Some("Bearer test-key"));
    assert!(received.json().get("jevgate").is_none());
    let (_, openrouter) = mock(|received| {
        let mut body = crate::tests::answer(&received.json(), 0);
        body["id"] = json!("gen-dec-1789738314-X5e5");
        Reply::json(200, &body)
    });
    let answer = send(&agent(ATTEMPT_TIMEOUT), &openrouter, "k", &question()).unwrap();
    assert_eq!(
        answer["request_id"], "gen-dec-1789738314-X5e5",
        "a response's own id stands in"
    );
    let (_, forged) = mock(|received| {
        let mut body = crate::tests::answer(&received.json(), 0);
        body["request_id"] = json!("not an id \u{1b}[31m<script>\nline2");
        Reply::json(200, &body)
    });
    let answer = send(&agent(ATTEMPT_TIMEOUT), &forged, "k", &question()).unwrap();
    assert!(
        answer.get("request_id").is_none(),
        "a request_id the body sends itself is not an id that was checked"
    );
}

/// A mock provider that replies to the first request with what `first` makes
/// of its answer, and answers every later one.
fn mock_first(first: impl Fn(Reply) -> Reply + Send + Sync + 'static) -> (MockProvider, Endpoint) {
    let calls = std::sync::atomic::AtomicUsize::new(0);
    mock(move |received| {
        let reply = answered(received);
        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
            first(reply)
        } else {
            reply
        }
    })
}

/// The outcome of one request sent to `endpoint` through a one-worker queue.
fn queued(agent: &ureq::Agent, endpoint: &Endpoint) -> Outcome {
    let request = question();
    let mut last = None;
    fast().evaluate_queue(
        &[&request],
        1,
        &|_| Ok(()),
        |request| send(agent, endpoint, "k", request),
        &mut |_, outcome| last = Some(outcome),
    );
    last.unwrap()
}

#[test]
fn a_rate_limit_or_overload_waits_the_milliseconds_the_provider_asks_for() {
    for status in [429, 503] {
        let (provider, endpoint) = mock_first(move |_| {
            Reply::json(status, &json!({}))
                .header("retry-after-ms", "400")
                .header("retry-after", "30")
        });
        let start = Instant::now();
        let outcome = queued(&agent(ATTEMPT_TIMEOUT), &endpoint);
        assert!(outcome.result.is_ok(), "{status}");
        assert_eq!((outcome.retries, provider.received().len()), (1, 2));
        let waited = start.elapsed();
        assert!(
            waited >= Duration::from_millis(400) && waited < Duration::from_secs(30),
            "{status}: {waited:?}"
        );
    }
}

/// Sends 0.25 made of a request answered with an overload; after as many,
/// the canaries through OpenRouter gave up on 2026-09-28.
const SENDS_IN_0_25: usize = 4;

#[test]
fn a_gateway_answering_503_to_the_first_sends_of_every_request_completes() {
    let sends = Mutex::new(std::collections::HashMap::<String, usize>::new());
    let provider = MockProvider::start(move |received| {
        let request = received.json();
        let mut sends = sends.lock().unwrap();
        let sent = sends.entry(request["state"].to_string()).or_default();
        *sent += 1;
        if *sent <= SENDS_IN_0_25 {
            let overloaded = json!({"error": {"code": 503,
                "metadata": {"error_type": "provider_overloaded"}}});
            return Reply::json(503, &overloaded);
        }
        answered(received)
    });
    let endpoint = Endpoint::custom(&crate::provider::OPENROUTER, &provider.url).unwrap();
    let requests: Vec<Value> = (0..3)
        .map(|i| {
            let mut request = question();
            request["state"] = json!(format!("unit {i}"));
            request
        })
        .collect();
    let batch: Vec<&Value> = requests.iter().collect();
    let agent = agent(ATTEMPT_TIMEOUT);
    let mut outcomes = Vec::new();
    fast().evaluate_queue(
        &batch,
        crate::provider::GATEWAY_CONCURRENCY as usize,
        &|_| Ok(()),
        |request| send(&agent, &endpoint, "sk-or-v1-test", request),
        &mut |_, outcome| outcomes.push(outcome),
    );
    assert_eq!(outcomes.len(), requests.len());
    for outcome in outcomes {
        assert!(outcome.result.is_ok(), "{:?}", outcome.result.err());
        assert_eq!(
            outcome.retries as usize, SENDS_IN_0_25,
            "answered on the send after the last one 0.25 made"
        );
    }
    assert_eq!(
        provider.received().len(),
        requests.len() * (SENDS_IN_0_25 + 1)
    );
}

#[test]
fn a_failure_names_its_request_id_and_invalid_fields_but_never_the_provider_text() {
    let (_provider, endpoint) = mock(|_| {
        let detail = json!({"detail": [{"loc": ["body", "questions", "q", "criteria"],
            "msg": "private text", "type": "missing", "input": "private input"}]});
        Reply::json(422, &detail).header(REQUEST_ID, "req_9")
    });
    let error = send(&agent(ATTEMPT_TIMEOUT), &endpoint, "k", &question()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "TypeSafe HTTP 422 (invalid request: body.questions.q.criteria missing); request was not retried; request id req_9"
    );
}

#[test]
fn requests_start_an_interval_apart_across_workers() {
    assert_eq!(REQUEST_INTERVAL * 1200, Duration::from_secs(60));
    let access = ProviderAccess {
        interval: Duration::from_millis(20),
        ..fast()
    };
    let requests: Vec<Value> = (0..6).map(|i| json!({"index": i})).collect();
    let batch: Vec<&Value> = requests.iter().collect();
    let starts = Mutex::new(Vec::new());
    let before = Instant::now();
    access.evaluate_queue(
        &batch,
        MAX_WORKERS,
        &|_| Ok(()),
        |request| {
            starts.lock().unwrap().push(Instant::now());
            Ok(request.clone())
        },
        &mut |_, outcome| assert!(outcome.result.is_ok()),
    );
    let mut starts = starts.into_inner().unwrap();
    starts.sort();
    for (i, start) in starts.iter().enumerate() {
        assert!(
            start.duration_since(before) >= access.interval * i as u32,
            "{i}"
        );
    }
}

/// Workers a queue may run at once.
const MAX_WORKERS: usize = crate::options::MAX_CONCURRENCY as usize;

#[test]
fn a_slow_answer_times_out_and_is_sent_once_more() {
    let (provider, endpoint) = mock_first(|reply| Reply {
        delay: Duration::from_secs(2),
        ..reply
    });
    let start = Instant::now();
    let outcome = queued(&agent(Duration::from_millis(300)), &endpoint);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result.err());
    assert_eq!((outcome.retries, provider.received().len()), (1, 2));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "the attempt gave up at its timeout"
    );
}
