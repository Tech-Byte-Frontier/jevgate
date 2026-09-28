//! The answer cache: each question is kept apart, so a request sends only the
//! questions it lacks, and the whole-request entries of earlier versions keep
//! answering.
use super::*;

/// A request about one state asking three Nouls, as a planner builds it.
fn three_questions() -> Value {
    let noul = |text: &str| json!({"type":"noul","instructions":{"question":text}});
    json!({
        "model": "jev-1.13.0",
        "state": {"source": "fn total(values: &[i32]) -> i32 { values.iter().sum() }"},
        "questions": {
            "long": noul("Is `source` long?"),
            "nested": noul("Does `source` nest loops?"),
            "named": noul("Does `source` name its function?"),
        },
        "jevgate": {"stage": "functions", "sources": []},
    })
}

/// The questions the provider was sent, request by request.
fn asked(mock: &Mock) -> Vec<Vec<String>> {
    mock.requests
        .iter()
        .map(|r| {
            r["questions"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        })
        .collect()
}

/// Ask `requests` once in a session over `project`'s cache.
fn ask(
    project: &Project,
    options: &CheckArgs,
    mock: &mut Mock,
    requests: &[&Value],
) -> Vec<requests::Receipt> {
    let context = project.context();
    let store = storage::Store::open(&project.0).unwrap();
    session(options, &context, &store, mock).queries(requests)
}

/// A project whose cache answers `three_questions`, which the returned mock
/// was asked, with the default options.
fn answered_once() -> (Project, CheckArgs, Mock, Value) {
    let project = Project::new();
    let options = args();
    let mut mock = Mock::default();
    let request = three_questions();
    ask(&project, &options, &mut mock, &[&request]);
    (project, options, mock, request)
}

/// Save `request`'s answer as versions before per-question caching did: one
/// entry for the whole request, under the hash of the rubric and the request.
fn save_whole(project: &Project, request: &Value, created_at: u64) {
    let key = schema::hash(
        &serde_json::to_vec(&(schema::RUBRIC, requests::provider_request(request))).unwrap(),
    );
    let mut body = answer(request, 2);
    body["usage"] = json!({"input_tokens": 300, "output_tokens": 3});
    storage::Store::open(&project.0)
        .unwrap()
        .save_request(&key, &body, created_at)
        .unwrap();
}

fn reworded(request: &Value, name: &str) -> Value {
    let mut changed = request.clone();
    changed["questions"][name]["instructions"]["question"] = json!("Is `source` hard to read?");
    changed
}

#[test]
fn a_reworded_or_added_question_is_asked_alone() {
    let (project, options, mut mock, first) = answered_once();
    assert_eq!(asked(&mock), [["long", "named", "nested"]]);
    let second = reworded(&first, "nested");
    let receipts = ask(&project, &options, &mut mock, &[&second]);
    assert_eq!(
        asked(&mock)[1],
        ["nested"],
        "only the reworded question is sent"
    );
    let (body, _, cached) = receipts[0].result.as_ref().unwrap();
    assert!(!cached);
    assert_eq!(body["answers"].as_object().unwrap().len(), 3);
    assert!(response::validate(body, &second).is_ok());
    let metrics = &receipts[0].metrics;
    assert_eq!((metrics.asked_questions, metrics.cached_questions), (1, 2));
    let mut added = second.clone();
    added["questions"]["short"] = json!({"type":"noul","instructions":{"question":"Is it short?"}});
    ask(&project, &options, &mut mock, &[&added]);
    assert_eq!(asked(&mock)[2], ["short"]);
    let mut moved = added.clone();
    moved["state"]["source"] = json!("fn total() -> i32 { 0 }");
    ask(&project, &options, &mut mock, &[&moved]);
    assert_eq!(asked(&mock)[3].len(), 4, "a new state asks everything");
    let again = ask(&project, &options, &mut mock, &[&added, &moved]);
    assert_eq!(mock.calls, 4);
    assert!(again.iter().all(|r| r.result.as_ref().unwrap().2));
    assert_eq!(again[0].metrics.cached_questions, 4);
}

#[test]
fn the_token_calibration_divides_the_bytes_sent_by_their_tokens() {
    let (project, options, mut mock, first) = answered_once();
    let context = project.context();
    let store = storage::Store::open(&project.0).unwrap();
    let observed = {
        let mut session = session(&options, &context, &store, &mut mock);
        session.queries(&[&reworded(&first, "long")]);
        session.observed
    };
    let sent = serde_json::to_vec(&mock.requests[1]).unwrap().len() as u64;
    assert_eq!(
        observed,
        (sent, 10),
        "one question and its state, not three"
    );
}

#[test]
fn whole_request_answers_of_an_earlier_version_keep_answering() {
    let project = Project::new();
    let options = args();
    let request = three_questions();
    let created_at = schema::now() - 60;
    save_whole(&project, &request, created_at);
    let mut mock = Mock::default();
    let receipts = ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(mock.calls, 0);
    let (body, timestamp, cached) = receipts[0].result.as_ref().unwrap();
    assert!(*cached);
    assert_eq!(*timestamp, created_at);
    assert_eq!(
        body["usage"],
        json!({"input_tokens": 300, "output_tokens": 3})
    );
    assert_eq!(
        body["answers"]["long"],
        answer(&request, 2)["answers"]["long"]
    );
    // The answers were copied, so the old entry is no longer needed...
    for entry in std::fs::read_dir(project.0.join(".jevgate/cache")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            std::fs::remove_file(path).unwrap();
        }
    }
    let receipts = ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(mock.calls, 0);
    assert_eq!(receipts[0].result.as_ref().unwrap().1, created_at);
    // ...and a question reworded after the upgrade is asked alone.
    ask(
        &project,
        &options,
        &mut mock,
        &[&reworded(&request, "long")],
    );
    assert_eq!(asked(&mock), [["long"]]);
}

/// Answers its first request at the bottom of every scale and the others at the top.
#[derive(Default)]
struct Shifting {
    calls: usize,
}

impl transport::Evaluator for Shifting {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        self.calls += 1;
        Ok(answer(request, if self.calls == 1 { 0 } else { 2 }))
    }
}

/// `request` asking its `long` question beside a question of its own.
fn sharing_long(request: &Value) -> Value {
    let mut other = request.clone();
    other["questions"] = json!({
        "long": request["questions"]["long"],
        "short": {"type":"noul","instructions":{"question":"Is `source` short?"}},
    });
    other
}

fn long(receipt: &requests::Receipt) -> Value {
    receipt.result.as_ref().unwrap().0["answers"]["long"].clone()
}

#[test]
fn a_question_two_requests_ask_about_one_state_has_one_answer_in_a_run() {
    let project = Project::new();
    let options = args();
    let first = three_questions();
    let second = sharing_long(&first);
    let mut shifting = Shifting::default();
    let receipts = {
        let context = project.context();
        let store = storage::Store::open(&project.0).unwrap();
        session(&options, &context, &store, &mut shifting).queries(&[&first, &second])
    };
    assert_eq!(shifting.calls, 2);
    assert_eq!(
        long(&receipts[0]),
        long(&receipts[1]),
        "one answer in the run"
    );
    let rerun = ask(&project, &options, &mut Mock::default(), &[&first, &second]);
    assert_eq!(long(&rerun[0]), long(&receipts[0]), "the rerun reads it");
    assert_eq!(long(&rerun[1]), long(&receipts[1]));
}

#[test]
fn refresh_asks_each_question_once_in_a_run() {
    let project = Project::new();
    let mut options = args();
    let first = three_questions();
    ask(&project, &options, &mut Mock::default(), &[&first]);
    options.refresh = true;
    let context = project.context();
    let store = storage::Store::open(&project.0).unwrap();
    let mut mock = Mock::default();
    let mut refreshed = session(&options, &context, &store, &mut mock);
    refreshed.queries(&[&first]);
    refreshed.queries(&[&sharing_long(&first)]);
    drop(refreshed);
    assert_eq!(
        asked(&mock),
        [vec!["long", "named", "nested"], vec!["short"]]
    );
}

#[test]
fn refresh_asks_every_question_and_cache_only_needs_every_answer() {
    let (project, mut options, mut mock, request) = answered_once();
    options.cache_only = true;
    let second = reworded(&request, "named");
    let receipts = ask(&project, &options, &mut mock, &[&second]);
    let error = receipts[0].result.as_ref().unwrap_err().to_string();
    assert!(error.contains("--cache-only"), "{error}");
    assert!(
        ask(&project, &options, &mut mock, &[&request])[0]
            .result
            .is_ok()
    );
    assert_eq!(mock.calls, 1);
    options.cache_only = false;
    options.refresh = true;
    ask(&project, &options, &mut mock, &[&second]);
    assert_eq!(asked(&mock)[1].len(), 3);
}

#[test]
fn alias_answers_expire_one_by_one_and_keep_their_age_when_carried() {
    let project = Project::new();
    let mut options = args();
    options.model = Some("jev-latest".into());
    options.cache_ttl_secs = Some(3600);
    let mut request = three_questions();
    request["model"] = json!("jev-latest");
    save_whole(&project, &request, schema::now() - 7200);
    let mut mock = Mock::default();
    ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(mock.calls, 1, "an expired whole entry answers nothing");
    let project = Project::new();
    save_whole(&project, &request, schema::now() - 1800);
    let mut mock = Mock::default();
    ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(mock.calls, 0);
    options.cache_ttl_secs = Some(900);
    ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(
        mock.calls, 1,
        "a copied answer keeps the age of the entry it came from"
    );
}

#[test]
fn an_alias_answer_that_expires_in_a_session_is_asked_once_and_replaced() {
    let project = Project::new();
    let mut options = args();
    options.model = Some("jev-latest".into());
    options.cache_ttl_secs = Some(3600);
    let mut request = three_questions();
    request["model"] = json!("jev-latest");
    let context = project.context();
    let store = storage::Store::open(&project.0).unwrap();
    let mut shifting = Shifting::default();
    let mut watch = session(&options, &context, &store, &mut shifting);
    let first = watch.queries(&[&request]);
    edit_answers(&project, |answers| {
        for answer in answers.values_mut() {
            answer["created_at"] = json!(schema::now() - 7200);
        }
    });
    let renewed = watch.queries(&[&request]);
    let kept = watch.queries(&[&request]);
    drop(watch);
    assert_eq!(shifting.calls, 2, "asked again once, when it expired");
    assert_ne!(long(&renewed[0]), long(&first[0]), "the new answer is used");
    assert!(kept[0].result.as_ref().unwrap().2, "and kept");
    assert_eq!(long(&kept[0]), long(&renewed[0]));
}

#[test]
fn a_batch_uses_an_earlier_versions_answer_wherever_it_asks_the_question() {
    let project = Project::new();
    let options = args();
    let first = three_questions();
    let second = sharing_long(&first);
    // Kept at the top of each scale; the mock answers at the bottom.
    save_whole(&project, &first, schema::now() - 60);
    let mut mock = Mock::default();
    // `second`, which no earlier version asked, is looked up first.
    let receipts = ask(&project, &options, &mut mock, &[&second, &first]);
    assert_eq!(
        asked(&mock),
        [["short"]],
        "the kept answer is not bought again"
    );
    assert_eq!(
        long(&receipts[0]),
        long(&receipts[1]),
        "one answer in the batch"
    );
    let rerun = ask(&project, &options, &mut mock, &[&second, &first]);
    assert_eq!(mock.calls, 1);
    assert_eq!(long(&rerun[0]), long(&receipts[0]), "the rerun reads it");
}

#[test]
fn paid_tokens_are_what_was_sent_and_a_file_counts_its_answers_shares() {
    let project = Project::new();
    project.write("lib.rs", &format!("{}{}", function("a"), function("b")));
    let mut options = args();
    options.rules = vec!["function_simplification".into()];
    let mut mock = Mock::default();
    let first = run(&project, &options, &mut mock);
    assert_eq!((first.api_requests, first.paid_input_tokens), (1, 10));
    let stage = &first.stages["functions"];
    assert_eq!((stage.asked_questions, stage.cached_questions), (2, 0));
    let cached = run(&project, &options, &mut mock);
    assert_eq!((cached.api_requests, cached.paid_input_tokens), (0, 0));
    assert_eq!(
        cached.files[0].input_tokens, 10,
        "the shares add up to what was paid"
    );
    assert_eq!(cached.stages["functions"].cached_questions, 2);
}

/// The one state file of a project whose run asked about one state.
fn state_file(project: &Project) -> std::path::PathBuf {
    let files: Vec<_> = std::fs::read_dir(project.0.join(".jevgate/cache/answers"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    files.into_iter().next().unwrap()
}

/// Change the answers in the one state file of `project`.
fn edit_answers(project: &Project, edit: impl FnOnce(&mut serde_json::Map<String, Value>)) {
    let path = state_file(project);
    let mut entry: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(entry["answers"].as_object_mut().unwrap());
    std::fs::write(&path, entry.to_string()).unwrap();
}

#[test]
fn a_dry_run_counts_the_questions_the_cache_answers_and_prices_the_rest() {
    let project = Project::new();
    project.write("lib.rs", &format!("{}{}", function("a"), function("b")));
    let mut options = args();
    options.rules = vec!["function_simplification".into()];
    let preview = |options: &CheckArgs| snapshot(&project, options).1.stages["functions"].clone();
    options.dry_run = true;
    let cold = preview(&options);
    assert_eq!(
        (cold.planned_questions, cold.planned_cached_questions),
        (2, 0)
    );
    options.dry_run = false;
    run(&project, &options, &mut Mock::default());
    edit_answers(&project, |answers| {
        let first = answers.keys().next().unwrap().clone();
        answers.remove(&first);
    });
    options.dry_run = true;
    let partial = preview(&options);
    assert_eq!(
        (partial.planned_requests, partial.planned_cached),
        (1, 0),
        "a request with an unanswered question is sent"
    );
    assert_eq!(
        (partial.planned_questions, partial.planned_cached_questions),
        (2, 1)
    );
    assert!(
        (1..cold.planned_tokens).contains(&partial.planned_tokens),
        "only the unanswered question is priced: {} of {}",
        partial.planned_tokens,
        cold.planned_tokens
    );
    let headline = output::headline(&snapshot(&project, &options).1);
    assert!(
        headline.contains("2 questions, 1 answered by the cache"),
        "{headline}"
    );
}

#[test]
fn a_deleted_corrupt_or_linked_state_file_is_asked_again() {
    let (project, options, mut mock, request) = answered_once();
    let path = state_file(&project);
    std::fs::write(&path, b"{\"state_hash\":").unwrap();
    ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(mock.calls, 2, "a corrupt file answers nothing");
    std::fs::remove_file(&path).unwrap();
    ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(mock.calls, 3, "a deleted file answers nothing");
    #[cfg(unix)]
    {
        let outside = project.0.join("elsewhere.json");
        std::fs::rename(&path, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        let linked = std::fs::read(&outside).unwrap();
        ask(&project, &options, &mut mock, &[&request]);
        assert_eq!(mock.calls, 4, "a linked file answers nothing");
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            linked,
            "nothing is written through it"
        );
        assert!(!path.is_symlink());
    }
}

#[test]
fn only_tampered_answers_are_asked_again() {
    let (project, options, mut mock, request) = answered_once();
    edit_answers(&project, |answers| {
        let mut answers = answers.values_mut();
        answers.next().unwrap()["answer"]["noul"] = json!(1.5);
        answers.next().unwrap()["input_tokens"] = json!(u64::MAX);
    });
    let receipts = ask(&project, &options, &mut mock, &[&request]);
    assert_eq!(
        asked(&mock)[1].len(),
        2,
        "only the invalid answers are asked again"
    );
    let (body, _, _) = receipts[0].result.as_ref().unwrap();
    assert!(body["usage"]["input_tokens"].as_u64().unwrap() < 1_000);
}
