//! A provider that stops answering must not hold the agent at every event,
//! nor every commit or push. A hook's check, and a Git hook's (`check
//! --staged` or `--pre-push`), watches what the provider does, and when it
//! fails in a way that passes with time (a timeout, a refused or dropped
//! connection, a rate limit or a server error), the checks of the next few
//! minutes use only the answers already cached: measured against a provider
//! that stopped answering, every edit otherwise waited 29.8 s and every stop
//! 41 to 50 s, the hook's whole budget, for as long as the outage lasted.
use super::turn;
use crate::{
    schema,
    transport::{self, Evaluator, Outcome},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

/// How long after a provider failure the hook's checks use only cached
/// answers: an agent edits every few seconds to minutes, so one wait at most
/// every five minutes of an outage, and a provider back within them is asked
/// again at the first event after.
const WAIT_SECS: u64 = 5 * 60;
const FILE: &str = "outage.json";

/// What the provider did during one check: requests sent and answered, and
/// the last failure worth waiting out.
#[derive(Default)]
pub(crate) struct Watch {
    sent: AtomicUsize,
    answered: AtomicUsize,
    failure: Mutex<Option<String>>,
}

impl Watch {
    fn saw(&self, result: &Result<Value>, retries: u32) {
        match result {
            Ok(_) => {
                self.answered.fetch_add(1, Ordering::Relaxed);
            }
            // Only failures that pass with time are retried.
            Err(error) if retries > 0 || transport::transient(error) => {
                *self.failure.lock().unwrap() = Some(error.to_string());
            }
            Err(_) => {}
        }
    }

    /// The last failure worth waiting out that the check met.
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().unwrap().clone()
    }

    /// Whether the provider was asked and nothing came back.
    pub fn silent(&self) -> bool {
        self.sent.load(Ordering::Relaxed) > 0 && self.answered.load(Ordering::Relaxed) == 0
    }
}

/// An evaluator whose answers `watch` sees.
pub(crate) struct Watched<'a> {
    pub inner: Box<dyn Evaluator + Send>,
    pub watch: &'a Watch,
}

impl Evaluator for Watched<'_> {
    fn begin_review(&mut self) {
        self.inner.begin_review();
    }

    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        self.watch.sent.fetch_add(1, Ordering::Relaxed);
        let result = self.inner.evaluate(request);
        self.watch.saw(&result, 0);
        result
    }

    fn evaluate_queue(
        &mut self,
        requests: &[&Value],
        concurrency: usize,
        before: &(dyn Fn(&Value) -> Result<()> + Sync),
        completed: &mut dyn FnMut(usize, Outcome),
    ) {
        let watch = self.watch;
        let counted = |request: &Value| {
            watch.sent.fetch_add(1, Ordering::Relaxed);
            before(request)
        };
        self.inner
            .evaluate_queue(requests, concurrency, &counted, &mut |index, outcome| {
                watch.saw(&outcome.result, outcome.retries);
                completed(index, outcome);
            });
    }
}

/// While the hook waits a failure out: no request is sent, so only cached
/// answers count, and each other request fails at once with `0`.
pub(crate) struct Waiting(pub String);

impl Evaluator for Waiting {
    fn evaluate(&mut self, _: &Value) -> Result<Value> {
        anyhow::bail!("{}", self.0)
    }
}

/// A provider failure the hook's checks wait out, in `.jevgate/turns/`.
#[derive(Serialize, Deserialize)]
pub(crate) struct Outage {
    /// When it was met, in seconds since the Unix epoch.
    pub at: u64,
    pub reason: String,
}

impl Outage {
    /// The whole minutes, at least one, before the provider is asked again.
    pub fn minutes_left(&self) -> u64 {
        (self.at + WAIT_SECS)
            .saturating_sub(schema::now())
            .div_ceil(60)
            .max(1)
    }
}

fn file(root: &Path) -> PathBuf {
    root.join(".jevgate/turns").join(FILE)
}

/// The failure the hook still waits out in the repository at `root`.
pub(crate) fn current(root: &Path) -> Option<Outage> {
    let text = crate::inventory::read_source(&file(root), turn::MAX_BYTES).ok()?;
    let outage: Outage = serde_json::from_str(&text).ok()?;
    (schema::now() < outage.at + WAIT_SECS).then_some(outage)
}

/// Wait out `reason`, a failure met now.
pub(crate) fn record(root: &Path, reason: &str) {
    let outage = Outage {
        at: schema::now(),
        reason: reason.to_string(),
    };
    if let (Ok(directory), Ok(bytes)) = (turn::directory(root), serde_json::to_vec(&outage)) {
        let _ = crate::storage::atomic(
            &directory.join(FILE),
            &bytes,
            crate::storage::Durability::Synced,
        );
    }
}

/// The provider answered: nothing is waited out.
pub(crate) fn clear(root: &Path) {
    let _ = std::fs::remove_file(file(root));
}
