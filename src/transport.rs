use anyhow::{Result, bail};
use serde_json::Value;
use std::{
    path::Path,
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};

pub trait Evaluator {
    /// A new snapshot may retry after account access has been restored.
    fn begin_review(&mut self) {}

    fn evaluate(&mut self, request: &Value) -> Result<Value>;

    fn evaluate_batch(&mut self, requests: &[&Value]) -> Vec<Result<Value>> {
        requests
            .iter()
            .map(|request| self.evaluate(request))
            .collect()
    }

    fn evaluate_queue(
        &mut self,
        requests: &[&Value],
        concurrency: usize,
        before: &(dyn Fn(&Value) -> Result<()> + Sync),
        completed: &mut dyn FnMut(usize, Outcome),
    ) {
        let queue_start = std::time::Instant::now();
        let mut index = 0;
        for chunk in requests.chunks(concurrency) {
            let checks: Vec<_> = chunk.iter().map(|r| before(r)).collect();
            let valid: Vec<_> = chunk
                .iter()
                .zip(&checks)
                .filter_map(|(r, c)| c.is_ok().then_some(*r))
                .collect();
            let started_ms = queue_start.elapsed().as_millis() as u64;
            let start = std::time::Instant::now();
            let mut bodies = self.evaluate_batch(&valid).into_iter();
            for check in checks {
                completed(
                    index,
                    match check {
                        Ok(()) => Outcome {
                            result: bodies.next().unwrap_or_else(|| {
                                Err(anyhow::anyhow!("Missing provider receipt"))
                            }),
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            started_ms,
                            attempted: true,
                        },
                        Err(e) => Outcome {
                            result: Err(e),
                            elapsed_ms: 0,
                            started_ms: 0,
                            attempted: false,
                        },
                    },
                );
                index += 1;
            }
        }
    }
}

pub struct Outcome {
    pub result: Result<Value>,
    pub elapsed_ms: u64,
    pub started_ms: u64,
    pub attempted: bool,
}

/// Workers claim the next item immediately after finishing, independent of the
/// slowest sibling. Completions carry their input index and are persisted immediately;
/// cancellation/freshness run at send.
pub(crate) fn work_queue<T: Sync, R: Send>(
    items: &[T],
    concurrency: usize,
    work: impl Fn(&T) -> R + Sync,
    mut completed: impl FnMut(usize, R),
) {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    let next = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..concurrency.min(items.len()) {
            let (next, work, sender) = (&next, &work, sender.clone());
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else {
                        break;
                    };
                    if sender.send((i, work(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for (index, result) in receiver {
            completed(index, result);
        }
    });
}

pub struct Client {
    agent: ureq::Agent,
    key_file: std::path::PathBuf,
    key: Option<crate::auth::sources::Credential>,
    explicit_file: bool,
    access: ProviderAccess,
}

impl Client {
    pub fn new(key_file: &Path, explicit_file: bool) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(30)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
            key_file: key_file.into(),
            key: None,
            explicit_file,
            access: ProviderAccess::default(),
        }
    }

    fn credential(&mut self) -> Result<&str> {
        if self.key.is_none() {
            self.key = Some(crate::auth::sources::resolve(
                &self.key_file,
                self.explicit_file,
            )?);
        }
        Ok(self.key.as_ref().unwrap().key.expose())
    }
}

impl Evaluator for Client {
    fn begin_review(&mut self) {
        if self.access.reset() {
            // A rejected credential may have been replaced between snapshots.
            self.key = None;
        }
    }

    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        self.access.check()?;
        let agent = self.agent.clone();
        let result = send(&agent, self.credential()?, request);
        self.access.observe(&result);
        result
    }

    fn evaluate_queue(
        &mut self,
        requests: &[&Value],
        concurrency: usize,
        before: &(dyn Fn(&Value) -> Result<()> + Sync),
        completed: &mut dyn FnMut(usize, Outcome),
    ) {
        let agent = self.agent.clone();
        match self.credential() {
            Ok(_) => {}
            Err(error) => {
                for i in 0..requests.len() {
                    completed(
                        i,
                        Outcome {
                            result: Err(anyhow::anyhow!(error.to_string())),
                            elapsed_ms: 0,
                            started_ms: 0,
                            attempted: false,
                        },
                    );
                }
                return;
            }
        }
        let key = self.key.as_ref().unwrap().key.expose();
        self.access.evaluate_queue(
            requests,
            concurrency,
            before,
            |request| send(&agent, key, request),
            completed,
        );
    }
}

/// Reject further uploads in this review only after a typed account/access failure.
/// Completed and in-flight requests keep their individual results; caches bypass this gate.
#[derive(Default)]
struct ProviderAccess(AtomicU16);

impl ProviderAccess {
    fn reset(&mut self) -> bool {
        self.0.swap(0, Ordering::AcqRel) != 0
    }

    fn check(&self) -> Result<()> {
        let status = self.0.load(Ordering::Acquire);
        if status != 0 {
            bail!(
                "TypeSafe request not sent after HTTP {status}; restore account access and rerun the review"
            );
        }
        Ok(())
    }

    fn observe(&self, result: &Result<Value>) {
        if let Err(error) = result
            && let Some(error) = error.downcast_ref::<ProviderError>()
            && matches!(error.status, 401..=403)
        {
            let _ = self
                .0
                .compare_exchange(0, error.status, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    fn evaluate_queue(
        &self,
        requests: &[&Value],
        concurrency: usize,
        before: &(dyn Fn(&Value) -> Result<()> + Sync),
        send: impl Fn(&Value) -> Result<Value> + Sync,
        completed: &mut dyn FnMut(usize, Outcome),
    ) {
        let queue_start = std::time::Instant::now();
        work_queue(
            requests,
            concurrency.clamp(1, 16),
            |request| {
                // Recheck after freshness work in case a sibling has since been rejected.
                if let Err(error) = self
                    .check()
                    .and_then(|()| before(request))
                    .and_then(|()| self.check())
                {
                    return Outcome {
                        result: Err(error),
                        elapsed_ms: 0,
                        started_ms: 0,
                        attempted: false,
                    };
                }
                let started_ms = queue_start.elapsed().as_millis() as u64;
                let start = std::time::Instant::now();
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| send(request)))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("TypeSafe request worker failed")));
                self.observe(&result);
                Outcome {
                    result,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    started_ms,
                    attempted: true,
                }
            },
            completed,
        );
    }
}

fn send(agent: &ureq::Agent, key: &str, request: &Value) -> Result<Value> {
    let response = agent
        .post("https://api.typesafe.ai/v1/systemone")
        .header("Authorization", format!("Bearer {key}"))
        .send_json(crate::requests::provider_request(request).as_ref());
    let mut response = match response {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(status)) => {
            return Err(provider_error(status, None).into());
        }
        Err(_) => bail!("TypeSafe transport failure or timeout; request was not retried"),
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(65_536)
            .read_json::<Value>()
            .ok();
        return Err(provider_error(status, body.as_ref()).into());
    }
    // Error bodies and headers may echo credentials or source; never render them.
    response
        .body_mut()
        .with_config()
        .limit(1_048_576)
        .read_json()
        .map_err(|_| anyhow::anyhow!("TypeSafe returned invalid or oversized JSON"))
}

#[derive(Debug)]
struct ProviderError {
    status: u16,
    context_limit: bool,
}

impl std::error::Error for ProviderError {}
impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail = if self.context_limit {
            " (model context limit exceeded)"
        } else {
            ""
        };
        write!(
            f,
            "TypeSafe HTTP {}{detail}; request was not retried",
            self.status
        )
    }
}

fn provider_error(status: u16, body: Option<&Value>) -> ProviderError {
    // Recognize only a verified machine code; do not echo arbitrary provider text.
    ProviderError {
        status,
        context_limit: status == 400
            && body.is_some_and(|body| body["detail"]["error_type"] == "max_tokens_exceeded"),
    }
}

#[cfg(test)]
pub(super) fn key_from_file(path: &Path) -> Result<String> {
    crate::auth::sources::key_from_file(path)?
        .map(|key| key.expose().to_owned())
        .ok_or_else(|| anyhow::anyhow!("Credential file has no TYPESAFE_API_KEY"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn account_rejections_stop_pending_uploads_but_keep_in_flight_successes() {
        use std::sync::{Barrier, Condvar, Mutex, atomic::AtomicUsize};
        let access = ProviderAccess::default();
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
                        return Err(provider_error(402, None).into());
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
            let mut access = ProviderAccess::default();
            let mut attempts = 0;
            access.evaluate_queue(
                &batch,
                1,
                &|_| Ok(()),
                |request| {
                    if request["index"] == 0 {
                        Err(anyhow::Error::new(provider_error(status, None))
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
        let access = ProviderAccess::default();
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
                            Err(provider_error(402, None).into())
                        } else {
                            Ok(answer(request, 0, 0.0))
                        }
                    },
                    completed,
                );
            }
        }
        let project = Project::new();
        for name in ["a", "b", "c", "d"] {
            project.write(&format!("{name}.py"), &format!("def {name}(fn):\n    try:\n        return fn()\n    except OSError:\n        raise\n"));
        }
        project.write("b.py", "def b(fn):\n    try:\n        return fn()\n    except OSError:\n        raise\n\ndef second(fn):\n    try:\n        return fn()\n    except ValueError:\n        raise\n");
        let mut options = args();
        options.quick = true;
        options.rules = vec!["function_simplification".into()];
        options.concurrency = 1;
        options.paths = vec!["a.py".into()];
        let mut provider = Provider {
            access: ProviderAccess::default(),
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
        assert_eq!(rejected.stages["maintainability"].failed_attempts, 1);
        assert_eq!(rejected.stages["maintainability"].cache_hits, 1);
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
    fn provider_errors_explain_known_limits_without_echoing_private_text() {
        let body = json!({"detail":{"error_type":"max_tokens_exceeded","message":"private source and credentials"}});
        assert_eq!(
            provider_error(400, Some(&body)).to_string(),
            "TypeSafe HTTP 400 (model context limit exceeded); request was not retried"
        );
        for body in [
            json!({"detail":"private source"}),
            json!({"detail":{"error_type":"private credentials"}}),
        ] {
            assert_eq!(
                provider_error(400, Some(&body)).to_string(),
                "TypeSafe HTTP 400; request was not retried"
            );
        }
        assert_eq!(
            provider_error(503, None).to_string(),
            "TypeSafe HTTP 503; request was not retried"
        );
    }
}
