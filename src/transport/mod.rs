use crate::provider::{Endpoint, Provider, Service, TYPESAFE};
use crate::provider_error::{
    Failure, Interrupted, ProviderError, Unsent, provider_error, retryable,
};
use crate::response_headers::{REQUEST_ID, request_id, retry_after};
use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU16, Ordering},
    },
    time::{Duration, Instant},
};

/// Sends of a request the provider answered with a status worth retrying
/// (a rate limit, overload, or a server or gateway error), the first
/// included. On 2026-09-28 TypeSafe answered 503 to about two attempts in
/// three for at least ten minutes, directly (22 of 34) and through OpenRouter
/// (141 of 224), each failed attempt taking about 10 s: with 4 sends, 1 of
/// 13 TypeSafe requests and 16 of 99 OpenRouter ones gave up, and every run
/// ended incomplete. Simulating this queue, 6 sends leave 6% of such
/// requests unanswered instead of 16%; when 1 attempt in 5 fails, a
/// 1,000-request run completes 95% of the time instead of 21%; and a
/// provider failing every attempt is outlasted for 26 s instead of 8. It
/// costs time only while attempts fail: a hard outage takes up to three
/// times as long to end a run incomplete, 8 minutes instead of 3 for 100
/// requests.
const ANSWERED_ATTEMPTS: u32 = 6;
/// Sends of a request whose connection failed before anything was sent, as
/// on a machine without a network: 4 as before, so such a run ends no later.
const UNSENT_ATTEMPTS: u32 = 4;
/// Attempts after a timeout or dropped connection: the provider may have run
/// (and billed) the first send, so it is repeated only once.
const INTERRUPTED_ATTEMPTS: u32 = 2;
/// Longest provider-requested pause that is honored before a retry.
const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);
/// How long one attempt may take, from connecting to reading the answer. A
/// request takes about 0.3 s and TypeSafe's SDKs wait 10 s per attempt; one
/// that has not answered in 20 s is sent again once, rather than holding a
/// worker for a minute.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
/// Requests start at least this far apart: 1,200 a minute, TypeSafe's limit.
const REQUEST_INTERVAL: Duration = Duration::from_millis(50);

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
    /// The provider the check planned its model for; the key must be its.
    provider: Provider,
    endpoint: Endpoint,
    key_file: std::path::PathBuf,
    key: Option<crate::auth::sources::Credential>,
    explicit_file: bool,
    access: ProviderAccess,
}

/// An HTTP client that gives up on an attempt after `timeout`, follows no
/// redirect (the key must not travel elsewhere) and returns error statuses as
/// responses, so their headers and body can be read.
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}

impl Client {
    /// A client for `provider`'s key, sending to its endpoint; says on
    /// stderr when `JEVGATE_BASE_URL` sends requests elsewhere.
    pub fn new(key_file: &Path, explicit_file: bool, provider: Provider) -> Result<Self> {
        let endpoint = Endpoint::new(provider)?;
        if endpoint.is_custom() {
            note!("Sending requests to {}", endpoint.describe());
        }
        Ok(Self {
            agent: agent(ATTEMPT_TIMEOUT),
            provider,
            access: ProviderAccess {
                service: endpoint.service,
                ..Default::default()
            },
            endpoint,
            key_file: key_file.into(),
            key: None,
            explicit_file,
        })
    }

    /// Give up on a retry, or a pause another request's failure asked for,
    /// that would end past `deadline`: the agent hook answers by then, and
    /// a provider asking for 30 s would otherwise hold every edit for its
    /// whole budget (29.8 s after an edit measured against a 503 asking
    /// for `retry-after-ms: 30000`). The failure is then the answer.
    pub fn until(mut self, deadline: Instant) -> Self {
        self.access.deadline = Some(deadline);
        self
    }

    fn credential(&mut self) -> Result<&str> {
        if self.key.is_none() {
            let credential = crate::auth::sources::resolve(&self.key_file, self.explicit_file)?;
            ensure!(
                credential.provider == self.provider,
                "The key in {} is for {}, but this check planned its requests for {}; rerun the check",
                credential.source,
                credential.provider.service().label,
                self.provider.service().label
            );
            self.key = Some(credential);
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
        self.credential()?;
        let key = self.key.as_ref().unwrap().key.expose();
        let result = send(&agent, &self.endpoint, key, request);
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
        let endpoint = &self.endpoint;
        self.access.evaluate_queue(
            requests,
            concurrency,
            before,
            |request| send(&agent, endpoint, key, request),
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

/// The first pause after an answer worth retrying; each later one doubles
/// the one before, up to [`LONGEST_BACKOFF`]: 1, 2, 4, 8 and 8 s.
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
/// The longest pause before its jitter. A pause holds every worker, so a
/// longer one would stall the whole run for one request's last send.
const LONGEST_BACKOFF: Duration = Duration::from_secs(8);

/// Reject further uploads in this review only after a typed account/access failure.
/// Completed and in-flight requests keep their individual results; caches bypass this gate.
/// Rate-limit and overload responses pause every worker through one shared cooldown.
struct ProviderAccess {
    rejected: AtomicU16,
    /// The rejection came from the provider's edge protection, not the account.
    edge: std::sync::atomic::AtomicBool,
    /// Consecutive edge blocks: a request whose content trips a firewall rule
    /// fails alone; blocks with no success between them stop further uploads.
    edge_blocks: AtomicU16,
    cooldown: Mutex<Option<Instant>>,
    backoff: Duration,
    /// The earliest time the next request may start, so that every worker's
    /// sends together stay `interval` apart.
    next_start: Mutex<Instant>,
    interval: Duration,
    /// The provider the requests go to, named in the messages.
    service: &'static Service,
    /// No retry or pause runs past it.
    deadline: Option<Instant>,
}

impl Default for ProviderAccess {
    fn default() -> Self {
        Self {
            rejected: AtomicU16::new(0),
            edge: std::sync::atomic::AtomicBool::new(false),
            edge_blocks: AtomicU16::new(0),
            cooldown: Mutex::new(None),
            backoff: FIRST_BACKOFF,
            next_start: Mutex::new(Instant::now()),
            interval: REQUEST_INTERVAL,
            service: &TYPESAFE,
            deadline: None,
        }
    }
}

impl ProviderAccess {
    fn reset(&mut self) -> bool {
        *self.cooldown.lock().unwrap() = None;
        self.edge_blocks.store(0, Ordering::Release);
        self.edge.store(false, Ordering::Release);
        self.rejected.swap(0, Ordering::AcqRel) != 0
    }

    fn check(&self) -> Result<()> {
        let status = self.rejected.load(Ordering::Acquire);
        let provider = self.service.label;
        if status != 0 && self.edge.load(Ordering::Acquire) {
            bail!(
                "{provider} request not sent after HTTP {status} from the provider's edge protection; wait before rerunning, and contact {provider} if it persists"
            );
        }
        if status == 402 {
            bail!(
                "{provider} request not sent after HTTP 402 (credits exhausted); {}, then rerun the review",
                self.service.credits
            );
        }
        if status != 0 {
            bail!(
                "{provider} request not sent after HTTP {status}; restore account access and rerun the review"
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

    /// Wait out the shared pause; an error when it ends past the deadline.
    fn wait(&self) -> Result<()> {
        let Some(until) = *self.cooldown.lock().unwrap() else {
            return Ok(());
        };
        ensure!(
            self.deadline.is_none_or(|deadline| until < deadline),
            "{} request not sent: the provider asked to wait past the time this check has",
            self.service.label
        );
        if let Some(remaining) = until.checked_duration_since(Instant::now()) {
            std::thread::sleep(remaining);
        }
        Ok(())
    }

    /// Whether a pause of `pause` from now ends past the deadline.
    fn past_deadline(&self, pause: Duration) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() + pause >= deadline)
    }

    /// Take the next start time, `interval` after the one before, and sleep until it.
    fn pace(&self) {
        let start = {
            let mut next = self.next_start.lock().unwrap();
            let start = (*next).max(Instant::now());
            *next = start + self.interval;
            start
        };
        if let Some(remaining) = start.checked_duration_since(Instant::now()) {
            std::thread::sleep(remaining);
        }
    }

    /// Exponential backoff with deterministic jitter, so reruns are reproducible
    /// while concurrent requests still spread out; `retry` counts from 1.
    fn backoff(&self, index: usize, retry: u32) -> Duration {
        let jitter = (index as u64 * JITTER_INDEX_STEP + u64::from(retry) * JITTER_RETRY_STEP)
            % JITTER_RANGE;
        let pause = self
            .backoff
            .saturating_mul(2u32.saturating_pow(retry.saturating_sub(1)))
            .min(LONGEST_BACKOFF);
        pause * (PER_MILLE + jitter as u32) / PER_MILLE
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
            if let Err(error) = self.wait() {
                return (Err(error), retry);
            }
            self.pace();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| send(request)))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "{} request worker failed",
                        self.service.label
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
            let pause = delay
                .unwrap_or_default()
                .max(self.backoff(index, retry + 1));
            if self.past_deadline(pause) {
                return (result, retry);
            }
            retry += 1;
            self.pause(pause);
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
            (
                error.retry_after.map(|p| p.min(RETRY_AFTER_CAP)),
                ANSWERED_ATTEMPTS,
            )
        });
    }
    if error.downcast_ref::<Interrupted>().is_some() {
        return Some((None, INTERRUPTED_ATTEMPTS));
    }
    error
        .downcast_ref::<Unsent>()
        .map(|_| (None, UNSENT_ATTEMPTS))
}

/// Whether `error` is a provider failure that passes with time: a rate
/// limit, an overload, a server or gateway error, a timeout, or a connection
/// that failed or dropped. Asking again at once rarely helps.
pub(crate) fn transient(error: &anyhow::Error) -> bool {
    retry_delay(error).is_some()
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
fn request_body(request: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(
        crate::requests::provider_request(request).as_ref(),
    )?)
}

/// Send one request and return the provider's answer, with the provider's
/// request id under `request_id`: TypeSafe's `x-typesafe-request-id` header,
/// else the response's own `id`, which OpenRouter sends.
fn send(agent: &ureq::Agent, endpoint: &Endpoint, key: &str, request: &Value) -> Result<Value> {
    let service = endpoint.service;
    let body = request_body(request)?;
    let response = agent
        .post(endpoint.systemone())
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
            let failure = Failure {
                status,
                ..Default::default()
            };
            return Err(provider_error(service, failure).into());
        }
        Err(ureq::Error::HostNotFound | ureq::Error::ConnectionFailed) => {
            return Err(Unsent(service).into());
        }
        Err(ureq::Error::Timeout(_) | ureq::Error::Io(_)) => {
            return Err(Interrupted(service).into());
        }
        Err(_) => bail!(
            "{} transport failure; request was not retried",
            service.label
        ),
    };
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let id = request_id(header(REQUEST_ID).as_deref());
    if !response.status().is_success() {
        let wait = retry_after(
            header("retry-after-ms").as_deref(),
            header("retry-after").as_deref(),
            std::time::SystemTime::now(),
        );
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(65_536)
            .read_to_string()
            .ok();
        let failure = Failure {
            status,
            body: body.as_deref(),
            retry_after: wait,
            request_id: id,
        };
        return Err(provider_error(service, failure).into());
    }
    // Error bodies and headers may echo credentials or source; never render them.
    let mut answer: Value = response
        .body_mut()
        .with_config()
        .limit(1_048_576)
        .read_json()
        .map_err(|error| match error {
            ureq::Error::Timeout(_) | ureq::Error::Io(_) => Interrupted(service).into(),
            _ => anyhow::anyhow!("{} returned invalid or oversized JSON", service.label),
        })?;
    // A `request_id` the body carries itself is replaced by the checked id,
    // or dropped without one, as a cached answer drops it.
    let id = id.or_else(|| request_id(answer["id"].as_str()));
    if let Some(fields) = answer.as_object_mut() {
        match id {
            Some(id) => fields.insert("request_id".into(), Value::String(id)),
            None => fields.remove("request_id"),
        };
    }
    Ok(answer)
}

#[cfg(test)]
mod tests;
