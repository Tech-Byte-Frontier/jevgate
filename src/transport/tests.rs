use super::*;
use serde_json::json;

fn fast() -> ProviderAccess {
    ProviderAccess {
        backoff: Duration::from_millis(1),
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
            Err(provider_error(429, None, Some(1)).into()),
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
        (529, provider_error(529, None, None)),
        (502, provider_error(502, None, None)),
    ] {
        let (outcome, calls) = sends(&access, vec![Err(error.into()), Ok(json!({}))]);
        assert_eq!((calls, outcome.retries), (2, 1), "{status}");
    }
    let (outcome, calls) = sends(&access, vec![Err(Unsent.into()), Ok(json!({}))]);
    assert_eq!((calls, outcome.retries), (2, 1), "connection never opened");
    assert_eq!(
        retry_delay(&provider_error(429, None, Some(3600)).into()),
        Some((Some(RETRY_AFTER_CAP), ATTEMPTS))
    );
}

#[test]
fn server_errors_retry_and_an_interrupted_request_is_sent_twice_at_most() {
    for status in [500, 520, 522, 524] {
        let (outcome, calls) = sends(
            &fast(),
            vec![
                Err(provider_error(status, None, None).into()),
                Ok(json!({})),
            ],
        );
        assert!(outcome.result.is_ok(), "{status}");
        assert_eq!((calls, outcome.retries), (2, 1), "{status}");
    }
    let (outcome, calls) = sends(&fast(), vec![Err(Interrupted.into()), Ok(json!({}))]);
    assert!(outcome.result.is_ok());
    assert_eq!(calls, 2, "a timeout passes on its second send");
    let failures = (0..4).map(|_| Err(Interrupted.into())).collect();
    let (outcome, calls) = sends(&fast(), failures);
    assert_eq!(calls, INTERRUPTED_ATTEMPTS as usize);
    let message = outcome.result.unwrap_err().to_string();
    assert!(message.contains("timed out") && message.contains("gave up after 2 attempts"));
}

#[test]
fn validation_transport_and_account_errors_are_sent_once() {
    for error in [
        anyhow::Error::from(provider_error(422, None, None)),
        provider_error(400, None, Some(1)).into(),
        provider_error(401, None, None).into(),
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
    let failures = (0..ATTEMPTS + 2)
        .map(|_| Err(provider_error(503, None, None).into()))
        .collect();
    let (outcome, calls) = sends(&fast(), failures);
    assert_eq!(calls, ATTEMPTS as usize);
    assert_eq!(outcome.retries, ATTEMPTS - 1);
    let message = outcome.result.unwrap_err().to_string();
    assert!(message.contains("HTTP 503") && message.contains("gave up after 4 attempts"));
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
                    return Err(provider_error(402, None, None).into());
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
                    Err(anyhow::Error::new(provider_error(status, None, None))
                        .context("provider response"))
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
                        Err(provider_error(402, None, None).into())
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
    options.concurrency = 1;
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
        Some("TypeSafe HTTP 402; request was not retried"),
        "later unsent work in the same file must not hide the original provider failure"
    );
    assert!(
        rejected.files[2..]
            .iter()
            .all(|f| f.error.as_ref().unwrap().contains("not sent"))
    );
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
        provider_error(400, Some(&body.to_string()), None).to_string(),
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
            provider_error(400, Some(&body.to_string()), None).to_string(),
            "TypeSafe HTTP 400; request was not retried"
        );
    }
    assert_eq!(
        provider_error(503, None, None).to_string(),
        "TypeSafe HTTP 503"
    );
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
    let edge = provider_error(403, Some("error code: 1010\n"), None);
    assert!(edge.edge_block);
    assert_eq!(
        edge.to_string(),
        "TypeSafe HTTP 403 (blocked by the provider's edge protection); request was not retried"
    );
    assert!(provider_error(403, Some(EDGE_PAGE), None).edge_block);
    assert!(!provider_error(403, Some("{\"detail\":\"forbidden\"}"), None).edge_block);
}

#[test]
fn isolated_edge_blocks_fail_alone_and_consecutive_blocks_stop_uploads() {
    let access = ProviderAccess::default();
    let blocked: Result<Value> = Err(provider_error(403, Some(EDGE_PAGE), None).into());
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

#[test]
fn credential_parser_does_not_execute_shell() {
    let project = crate::tests::Project::new();
    project.write(
        ".env",
        "export TYPESAFE_API_KEY='literal$(do-not-execute)'\n",
    );
    assert_eq!(
        key_from_file(&project.0.join(".env")).unwrap(),
        "literal$(do-not-execute)"
    );
}
