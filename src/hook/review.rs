//! One check inside the hook: the repository's configuration, the files the
//! turn changed, and the gate's levels, run on a worker thread so the hook
//! answers the agent by its deadline whatever the provider does.
use crate::{
    check,
    config::ConfigContext,
    inventory,
    options::{CheckArgs, Format},
    schema::{Finding, Report, Status, Strength},
    storage,
    transport::Evaluator,
};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// How long a check waits for another JevGate process in the repository (a
/// check, `--watch` or a hook of a parallel edit) to release the session lock.
const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(100);
/// The `command` of the reports the hook's checks publish: a few files a
/// turn changed, which `baseline` must not take for the repository's.
pub(crate) const REPORT_COMMAND: &str = "hook";

/// Builds the evaluator a check asks: the provider in production, a script
/// in tests.
pub(crate) type Evaluators =
    dyn Fn(&CheckArgs, &ConfigContext) -> Result<Box<dyn Evaluator + Send>> + Send + Sync;

/// What one hook check covers.
pub(super) struct Scope {
    /// The turn's baseline and the working tree now, as Git trees; without
    /// them the files in `paths` are checked whole.
    pub trees: Option<(String, String)>,
    /// Absolute paths of the files to check; empty checks every changed file.
    pub paths: Vec<PathBuf>,
}

/// A finding the agent may act on, and whether it fails the gate.
#[derive(Clone, Debug)]
pub(super) struct Flagged {
    pub path: PathBuf,
    pub finding: Finding,
    pub fails: bool,
}

/// Check `scope` and wait for it until `deadline`. The error says why the
/// check could not finish, for the person and the agent.
pub(super) fn check(
    context: ConfigContext,
    scope: Scope,
    evaluators: Arc<Evaluators>,
    deadline: Instant,
) -> std::result::Result<Vec<Flagged>, String> {
    let started = Instant::now();
    // The wait for the lock ends first, so its reason reaches the reply.
    let lock_until =
        (started + LOCK_WAIT).min(deadline.checked_sub(LOCK_POLL * 2).unwrap_or(started));
    let (sender, receiver) = mpsc::channel();
    // The thread is left behind at the deadline; the process exits after the
    // reply, and the answers it received are already in the cache.
    std::thread::spawn(move || {
        let checked = run(&context, scope, evaluators.as_ref(), lock_until);
        let _ = sender.send(checked.map_err(|e| format!("{e:#}")));
    });
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "the check did not finish within {} s (answers received so far are cached)",
            (deadline.saturating_duration_since(started).as_millis() + 500) / 1000
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("the check stopped unexpectedly".into()),
    }
}

/// The check itself: what `jevgate check` does with the repository's
/// configuration, less the output.
fn run(
    context: &ConfigContext,
    scope: Scope,
    evaluators: &Evaluators,
    lock_until: Instant,
) -> Result<Vec<Flagged>> {
    let args = arguments(context, scope)?;
    let paths = inventory::scope(&args, context)?;
    let inputs = inventory::collect(&args, context, &paths)?;
    if inputs.is_empty() {
        // Nothing the rules judge changed: no report replaces the last one.
        return Ok(Vec::new());
    }
    let store = open_store(&context.root, lock_until)?;
    let (previous, mut report) = check::first_snapshot(&args, context, &inputs);
    report.command = REPORT_COMMAND.into();
    let mut evaluator = evaluators(&args, context)?;
    let mut session = check::session(&args, context, &store, evaluator.as_mut());
    check::judge(&mut session, &inputs, previous.as_ref(), &mut report)?;
    if !report.complete {
        bail!(incomplete(&report));
    }
    Ok(flag(&report))
}

/// `check`'s arguments with the repository's configuration, for `scope`.
fn arguments(context: &ConfigContext, scope: Scope) -> Result<CheckArgs> {
    #[derive(clap::Parser)]
    struct Defaults {
        #[command(flatten)]
        args: CheckArgs,
    }
    let mut args = <Defaults as clap::Parser>::parse_from(["jevgate"]).args;
    check::configure(&mut args, context)?;
    // Never printed: the hook answers with its own JSON.
    args.format = Some(Format::Json);
    args.paths = scope.paths;
    if let Some((base, now)) = scope.trees {
        args.base = Some(base);
        args.worktree_snapshot = Some(now);
    }
    Ok(args)
}

/// The session's store, once no other JevGate process holds its lock.
fn open_store(root: &Path, until: Instant) -> Result<storage::Store> {
    loop {
        match storage::Store::open(root) {
            Ok(store) => return Ok(store),
            Err(_) if storage::writer_active(root) && Instant::now() < until => {
                std::thread::sleep(LOCK_POLL);
            }
            Err(_) if storage::writer_active(root) => bail!(
                "another JevGate process in this repository (a check, --watch or another hook) held its session lock"
            ),
            Err(error) => return Err(error),
        }
    }
}

/// Why an incomplete check could not judge everything: the run's first
/// error, else the first failed file's and how many failed alike.
fn incomplete(report: &Report) -> String {
    if let Some(error) = report.errors.first() {
        return error.clone();
    }
    let failed: Vec<&str> = report
        .files
        .iter()
        .filter(|f| f.status == Status::Error)
        .filter_map(|f| f.error.as_deref())
        .collect();
    match failed.first() {
        Some(first) if failed.len() > 1 => format!("{first} ({} files)", failed.len()),
        Some(first) => (*first).to_string(),
        None => "the check did not finish".into(),
    }
}

/// The findings the agent may act on: not notes and not accepted, those
/// that fail the gate first, then reviews, then by rank. Whether one fails is
/// what the check's own gate recorded on it.
fn flag(report: &Report) -> Vec<Flagged> {
    let mut flagged: Vec<Flagged> = report
        .files
        .iter()
        .flat_map(|file| {
            file.findings
                .iter()
                .filter(|f| f.strength != Strength::Note && !f.accepted())
                .map(|finding| Flagged {
                    path: file.path.clone(),
                    fails: finding.fails_gate(),
                    finding: finding.clone(),
                })
        })
        .collect();
    flagged.sort_by(|a, b| {
        b.fails
            .cmp(&a.fails)
            .then(b.finding.strength.cmp(&a.finding.strength))
            .then(b.finding.rank.total_cmp(&a.finding.rank))
    });
    flagged
}
