use crate::{
    options::Provider,
    provider_error::{
        Interrupted, ProviderError, Unsent, provider_error, provider_error_for, retryable,
    },
};
use anyhow::{Result, bail};
use serde_json::Value;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};

/// Attempts per request, including the first send.
const ATTEMPTS: u32 = 4;
/// Attempts after a timeout or dropped connection: the provider may have run
/// (and billed) the first send, so it is repeated only once.
const INTERRUPTED_ATTEMPTS: u32 = 2;
/// Longest provider-requested pause that is honored before a retry.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);

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
                        Ok(()) => Outcome::attempted(
                            bodies.next().unwrap_or_else(|| {
                                Err(anyhow::anyhow!("Missing provider receipt"))
                            }),
                            start,
                            started_ms,
                        ),
                        Err(e) => Outcome::skipped(e),
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
    /// Sends after the first one; zero when the request was not retried.
    pub retries: u32,
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
    provider: Provider,
    access: ProviderAccess,
}

impl Client {
    pub fn new(key_file: &Path, explicit_file: bool, provider: Provider) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(60)))
                .max_redirects(0)
                .http_status_as_error(false)
                .build()
                .into(),
            key_file: key_file.into(),
            key: None,
            explicit_file,
            provider,
            access: ProviderAccess::for_provider(provider),
        }
    }

    fn credential(&mut self) -> Result<&str> {
        if self.key.is_none() {
            self.key = Some(crate::auth::sources::resolve_for(
                self.provider,
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
        let provider = self.provider;
        let credential = self.credential()?;
        let result = send(&agent, credential, request, provider);
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
                            retries: 0,
                        },
                    );
                }
                return;
            }
        }
        let key = self.key.as_ref().unwrap().key.expose();
        let provider = self.provider;
        self.access.evaluate_queue(
            requests,
            concurrency,
            before,
            |request| send(&agent, key, request, provider),
            completed,
        );
    }
}

/// Consecutive edge blocks that stop further uploads.
const EDGE_BLOCKS: u16 = 3;

/// Backoff jitter in thousandths of the delay: coprime steps spread requests
/// and retries over up to a quarter of it.
const JITTER_INDEX_STEP: u64 = 37;
const JITTER_RETRY_STEP: u64 = 101;
const JITTER_RANGE: u64 = 250;
const PER_MILLE: u32 = 1000;

/// Reject further uploads in this review only after a typed account/access failure.
/// Completed and in-flight requests keep their individual results; caches bypass this gate.
/// Rate-limit and overload responses pause every worker through one shared cooldown.
struct ProviderAccess {
    provider_name: &'static str,
    rejected: AtomicU16,
    /// The rejection came from the provider's edge protection, not the account.
    edge: std::sync::atomic::AtomicBool,
    /// Consecutive edge blocks: a request whose content trips a firewall rule
    /// fails alone; blocks with no success between them stop further uploads.
    edge_blocks: AtomicU16,
    cooldown: Mutex<Option<Instant>>,
    backoff: Duration,
}

impl Default for ProviderAccess {
    fn default() -> Self {
        Self {
            provider_name: "TypeSafe",
            rejected: AtomicU16::new(0),
            edge: std::sync::atomic::AtomicBool::new(false),
            edge_blocks: AtomicU16::new(0),
            cooldown: Mutex::new(None),
            backoff: Duration::from_millis(500),
        }
    }
}

impl ProviderAccess {
    fn for_provider(provider: Provider) -> Self {
        Self {
            provider_name: provider.name(),
            ..Default::default()
        }
    }

    fn reset(&mut self) -> bool {
        *self.cooldown.lock().unwrap() = None;
        self.edge_blocks.store(0, Ordering::Release);
        self.edge.store(false, Ordering::Release);
        self.rejected.swap(0, Ordering::AcqRel) != 0
    }

    fn check(&self) -> Result<()> {
        let status = self.rejected.load(Ordering::Acquire);
        if status != 0 && self.edge.load(Ordering::Acquire) {
            bail!(
                "{} request not sent after HTTP {status} from the provider's edge protection; wait before rerunning, and contact {} if it persists",
                self.provider_name,
                self.provider_name
            );
        }
        if status != 0 {
            bail!(
                "{} request not sent after HTTP {status}; restore account access and rerun the review",
                self.provider_name
            );
        }
        Ok(())
    }

    fn observe(&self, result: &Result<Value>) {
        if result.is_ok() {
            self.edge_blocks.store(0, Ordering::Release);
        }
        if let Err(error) = result
            && let Some(error) = error.downcast_ref::<ProviderError>()
            && matches!(error.status, 401..=403)
        {
            if error.edge_block {
                if self.edge_blocks.fetch_add(1, Ordering::AcqRel) + 1 < EDGE_BLOCKS {
                    return;
                }
                self.edge.store(true, Ordering::Release);
            }
            let _ = self.rejected.compare_exchange(
                0,
                error.status,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }

    /// Extend the shared pause; a shorter request never shortens another worker's wait.
    fn pause(&self, delay: Duration) {
        let until = Instant::now() + delay;
        let mut cooldown = self.cooldown.lock().unwrap();
        if cooldown.is_none_or(|current| current < until) {
            *cooldown = Some(until);
        }
    }

    fn wait(&self) {
        let until = *self.cooldown.lock().unwrap();
        if let Some(remaining) =
            until.and_then(|until| until.checked_duration_since(Instant::now()))
        {
            std::thread::sleep(remaining);
        }
    }

    /// Exponential backoff with deterministic jitter, so reruns are reproducible
    /// while concurrent requests still spread out.
    fn backoff(&self, index: usize, retry: u32) -> Duration {
        let jitter = (index as u64 * JITTER_INDEX_STEP + u64::from(retry) * JITTER_RETRY_STEP)
            % JITTER_RANGE;
        self.backoff * 2u32.pow(retry) * (PER_MILLE + jitter as u32) / PER_MILLE
    }

    fn send_with_retries(
        &self,
        index: usize,
        request: &Value,
        before: &(dyn Fn(&Value) -> Result<()> + Sync),
        send: &(impl Fn(&Value) -> Result<Value> + Sync),
    ) -> (Result<Value>, u32) {
        let mut retry = 0;
        loop {
            self.wait();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| send(request)))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "{} request worker failed",
                        self.provider_name
                    ))
                });
            self.observe(&result);
            let Some((delay, attempts)) = result.as_ref().err().and_then(retry_delay) else {
                return (result, retry);
            };
            if retry + 1 >= attempts {
                return (
                    result.map_err(|error| {
                        anyhow::anyhow!("{error}; gave up after {attempts} attempts")
                    }),
                    retry,
                );
            }
            retry += 1;
            self.pause(delay.unwrap_or_default().max(self.backoff(index, retry)));
            // A rejected sibling or an edited source stops the retry like a first send.
            if let Err(error) = self
                .check()
                .and_then(|()| before(request))
                .and_then(|()| self.check())
            {
                return (Err(error), retry);
            }
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
        let indexed: Vec<_> = requests.iter().enumerate().collect();
        work_queue(
            &indexed,
            concurrency.clamp(1, crate::options::MAX_CONCURRENCY as usize),
            |(index, request)| {
                // Recheck after freshness work in case a sibling has since been rejected.
                if let Err(error) = self
                    .check()
                    .and_then(|()| before(request))
                    .and_then(|()| self.check())
                {
                    return Outcome::skipped(error);
                }
                let started_ms = queue_start.elapsed().as_millis() as u64;
                let start = std::time::Instant::now();
                let (result, retries) = self.send_with_retries(*index, request, before, &send);
                let mut outcome = Outcome::attempted(result, start, started_ms);
                outcome.retries = retries;
                outcome
            },
            completed,
        );
    }
}

/// The pause and the attempt limit for a failure worth retrying: rate limits,
/// overload, server and gateway errors, and connections that failed before
/// the request was sent. A timeout or dropped connection is retried once,
/// since the request may have run. Validation and account errors are never
/// retried.
fn retry_delay(error: &anyhow::Error) -> Option<(Option<Duration>, u32)> {
    if let Some(error) = error.downcast_ref::<ProviderError>() {
        return retryable(error.status).then(|| {
            let pause = error
                .retry_after
                .map(|s| Duration::from_secs(s).min(RETRY_AFTER_CAP));
            (pause, ATTEMPTS)
        });
    }
    if error.downcast_ref::<Interrupted>().is_some() {
        return Some((None, INTERRUPTED_ATTEMPTS));
    }
    error.downcast_ref::<Unsent>().map(|_| (None, ATTEMPTS))
}

impl Outcome {
    fn attempted(result: Result<Value>, start: std::time::Instant, started_ms: u64) -> Self {
        Self {
            result,
            elapsed_ms: start.elapsed().as_millis() as u64,
            started_ms,
            attempted: true,
            retries: 0,
        }
    }

    fn skipped(error: anyhow::Error) -> Self {
        Self {
            result: Err(error),
            elapsed_ms: 0,
            started_ms: 0,
            attempted: false,
            retries: 0,
        }
    }
}

/// Compact JSON: ureq's `send_json` pretty-prints, and the provider's edge
/// blocks indented bodies carrying JSX that it accepts when compact.
fn request_body(request: &Value, provider: Provider) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(
        crate::requests::provider_request_for(request, provider).as_ref(),
    )?)
}

fn endpoint(provider: Provider) -> &'static str {
    match provider {
        Provider::TypeSafe => "https://api.typesafe.ai/v1/systemone",
        Provider::OpenRouter => "https://openrouter.ai/api/v1/systemone",
    }
}

fn send(agent: &ureq::Agent, key: &str, request: &Value, provider: Provider) -> Result<Value> {
    let body = request_body(request, provider)?;
    let response = agent
        .post(endpoint(provider))
        .header("Authorization", format!("Bearer {key}"))
        .header(
            "User-Agent",
            concat!(
                "jevgate/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/Tech-Byte-Frontier/jevgate)"
            ),
        )
        .content_type("application/json")
        .send(&body[..]);
    let mut response = match response {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(status)) => {
            let error = match provider {
                Provider::TypeSafe => provider_error(status, None, None),
                Provider::OpenRouter => provider_error_for(provider.name(), status, None, None),
            };
            return Err(error.into());
        }
        Err(ureq::Error::HostNotFound | ureq::Error::ConnectionFailed) => {
            return Err(Unsent.into());
        }
        Err(ureq::Error::Timeout(_) | ureq::Error::Io(_)) => return Err(Interrupted.into()),
        Err(_) => bail!(
            "{} transport failure; request was not retried",
            provider.name()
        ),
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let body = response
            .body_mut()
            .with_config()
            .limit(65_536)
            .read_to_string()
            .ok();
        let error = match provider {
            Provider::TypeSafe => provider_error(status, body.as_deref(), retry_after),
            Provider::OpenRouter => {
                provider_error_for(provider.name(), status, body.as_deref(), retry_after)
            }
        };
        return Err(error.into());
    }
    // Error bodies and headers may echo credentials or source; never render them.
    response
        .body_mut()
        .with_config()
        .limit(1_048_576)
        .read_json()
        .map_err(|error| match error {
            ureq::Error::Timeout(_) | ureq::Error::Io(_) => Interrupted.into(),
            _ => anyhow::anyhow!("{} returned invalid or oversized JSON", provider.name()),
        })
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
    use crate::provider_error::provider_error;
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
        let body = String::from_utf8(request_body(&request, Provider::TypeSafe).unwrap()).unwrap();
        assert_eq!(body, r#"{"model":"m","state":{"source":"<a b={c} />"}}"#);
        assert_eq!(
            endpoint(Provider::TypeSafe),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            endpoint(Provider::OpenRouter),
            "https://openrouter.ai/api/v1/systemone"
        );
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
}
