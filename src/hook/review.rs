//! One check inside the hook: the repository's configuration, the files the
//! turn changed, and the gate's levels, run on a worker thread so the hook
//! answers the agent by its deadline whatever the provider does. Within a
//! turn, the check reads jevgate.toml, the custom questions, the baseline
//! and `jevgate: allow` comments as they were when the turn began: accepting
//! a finding or loosening the gate is the person's decision, so the agent's
//! edits to them count from the next turn, and the person is told of each.
//! The exception is a finding the agent dismissed with a reason through
//! `jevgate baseline mark`, which counts at once and which the person
//! audits with `baseline stats`.
use super::outage::{Waiting, Watch, Watched};
use crate::{
    check,
    config::{Config, ConfigContext},
    guards::{self, Guard},
    init::CONFIG_FILE,
    inventory,
    options::{CheckArgs, Disposition, Format},
    revision,
    schema::{Finding, Gating, Report, Status, Strength},
    storage,
    transport::Evaluator,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

/// How long a check waits for another JevGate process in the repository (a
/// check, `--watch` or a hook of a parallel edit) to release the session lock.
const LOCK_WAIT: Duration = Duration::from_secs(10);
/// jevgate.toml is read up to this size, as the baseline is.
const CONFIG_BYTES: u64 = crate::baseline::BASELINE_BYTES;
const LOCK_POLL: Duration = Duration::from_millis(100);
/// The stack of the thread a check runs on: 8 MiB, a main thread's on
/// Linux and macOS, where `jevgate check` runs.
const WORKER_STACK: usize = 8 * 1024 * 1024;
/// The `command` of the reports the hook's checks publish: a few files a
/// turn changed, which `baseline` must not take for the repository's.
pub(crate) const REPORT_COMMAND: &str = "hook";

/// Builds the evaluator a check asks, which gives up on what would run past
/// the deadline: the provider in production, a script in tests.
pub(crate) type Evaluators =
    dyn Fn(&CheckArgs, &ConfigContext, Instant) -> Result<Box<dyn Evaluator + Send>> + Send + Sync;

/// How a hook check asks: the evaluator built for its deadline, or, while
/// the hook waits out a provider failure, none, with why.
pub(super) struct Asking {
    pub evaluators: Arc<Evaluators>,
    pub deadline: Instant,
    pub waiting: Option<String>,
}

impl Asking {
    /// The evaluator for one check, watched by `watch`.
    fn evaluator<'w>(
        &self,
        args: &CheckArgs,
        context: &ConfigContext,
        watch: &'w Watch,
    ) -> Result<Watched<'w>> {
        let inner: Box<dyn Evaluator + Send> = match &self.waiting {
            Some(why) => Box::new(Waiting(why.clone())),
            None => (self.evaluators)(args, context, self.deadline)?,
        };
        Ok(Watched { inner, watch })
    }
}

/// Why a hook check could not finish, and the provider's failure when it
/// is one the hook waits out.
#[derive(Debug)]
pub(super) struct Unfinished {
    pub reason: String,
    pub outage: Option<String>,
    /// The configuration of the turn's start did not load. Every check of
    /// the turn reads it, so checking these changes later from the same
    /// start would fail the same way.
    pub start_unreadable: bool,
}

impl Unfinished {
    fn new(reason: String) -> Self {
        Self {
            reason,
            outage: None,
            start_unreadable: false,
        }
    }
}

/// What one hook check covers.
pub(super) struct Scope {
    /// The turn's baseline and the working tree now, as Git trees; without
    /// them the files in `paths` are checked whole.
    pub trees: Option<(String, String)>,
    /// Absolute paths of the files to check; empty checks every changed file.
    pub paths: Vec<PathBuf>,
}

/// A finding the agent may act on.
#[derive(Clone, Debug)]
pub(super) struct Flagged {
    pub path: PathBuf,
    pub finding: Finding,
    /// Accepted by a baseline entry or `jevgate: allow` comment the turn
    /// added, which counts from the next turn.
    pub accepted_this_turn: bool,
}

impl Flagged {
    /// Whether it fails the gate: what the check's gate recorded on it.
    pub fn fails(&self) -> bool {
        self.finding.fails_gate()
    }

    /// Whether it keeps the agent working until it is fixed or dismissed:
    /// every finding the gate fails on or still measures, so a look-here
    /// review is verified once. A rule the person set to `none` and a custom
    /// question tried as a note stay optional.
    pub fn blocks(&self) -> bool {
        matches!(self.finding.gate, Some(Gating::Fails | Gating::Measuring))
    }
}

/// A finding the agent dismissed with a reason during the turn.
#[derive(Clone, Debug)]
pub(super) struct Dismissed {
    pub path: PathBuf,
    pub line: usize,
    pub rule: String,
    pub reason: Disposition,
    /// The note given with the reason, such as the issue that will fix it.
    pub note: Option<String>,
}

/// A changed file of code the check did not judge, and why: it reads as
/// generated code or a copied library, it is larger than max_file_bytes, or
/// it does not parse. Silence about it would read as a pass.
#[derive(Clone, Debug)]
pub(super) struct Unreviewed {
    pub path: PathBuf,
    pub why: String,
}

impl Unreviewed {
    /// A file the agent edited inside a Git repository of its own, which
    /// the turn's snapshots do not see.
    pub fn unseen(path: PathBuf) -> Self {
        Self {
            path,
            why: "it is inside a Git repository of its own (a submodule or nested clone), whose files this repository's snapshots do not record; check it in that repository".into(),
        }
    }

    /// The same file, not judged for the same reason, has the same id.
    pub fn id(&self) -> String {
        crate::schema::hash(format!("unreviewed {} {}", self.path.display(), self.why).as_bytes())
    }
}

/// A unit the check left undecided where undecided results fail the gate:
/// the person put `uncertain` among a rule's levels, so it fails the check
/// as a finding does, and the hook blocks on it too.
#[derive(Clone, Debug)]
pub(super) struct Undecided {
    pub path: PathBuf,
    pub line: usize,
    /// The rule's ID.
    pub rule: String,
    /// The unit and its open questions, or why the file was not decided.
    pub what: String,
}

impl Undecided {
    /// The same unit, undecided on the same questions, has the same id.
    pub fn id(&self) -> String {
        crate::schema::hash(
            format!(
                "undecided {} {} {} {}",
                self.path.display(),
                self.line,
                self.rule,
                self.what
            )
            .as_bytes(),
        )
    }
}

/// What one hook check found: the findings the agent may act on, those it
/// dismissed this turn, the units left undecided that fail the gate, what
/// the change does to the checks around the code, and the files it did not
/// judge.
#[derive(Debug, Default)]
pub(super) struct Checked {
    pub flagged: Vec<Flagged>,
    pub dismissed: Vec<Dismissed>,
    pub undecided: Vec<Undecided>,
    pub guards: Vec<Guard>,
    pub unreviewed: Vec<Unreviewed>,
}

/// Where a hook check runs: the session's directory and the repository
/// around it.
pub(super) struct Place {
    pub cwd: PathBuf,
    pub root: PathBuf,
}

/// Check `scope` in `place` and wait for it until the deadline. The error
/// says why the check could not finish, for the person and the agent, and
/// what it met of the provider.
pub(super) fn check(
    place: Place,
    scope: Scope,
    asking: Asking,
) -> std::result::Result<Checked, Unfinished> {
    let started = Instant::now();
    let deadline = asking.deadline;
    // The wait for the lock ends first, so its reason reaches the reply.
    let lock_until =
        (started + LOCK_WAIT).min(deadline.checked_sub(LOCK_POLL * 2).unwrap_or(started));
    let watch = Arc::new(Watch::default());
    let watching = Arc::clone(&watch);
    let (sender, receiver) = mpsc::channel();
    // The thread is left behind at the deadline; the process exits after the
    // reply, and the answers it received are already in the cache. Its
    // stack is a main thread's: a parse of deeply nested code recurses, and
    // the 2 MiB of a spawned thread aborted on a 1,500-branch `else if`
    // chain that `jevgate check` judges.
    let worker = std::thread::Builder::new()
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let _ = sender.send(work(&place, scope, (&asking, &watching), lock_until));
        });
    if let Err(error) = worker {
        return Err(Unfinished::new(format!(
            "the check could not start ({error})"
        )));
    }
    let waited = (deadline.saturating_duration_since(started).as_millis() + 500) / 1000;
    let received = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    finished(received, &watch, waited)
}

/// What the worker ran: the check, or why it failed and whether it failed
/// on the configuration of the turn's start. Within a turn the check reads
/// that configuration, so when it does not load no later check from that
/// start can run.
fn work(
    place: &Place,
    scope: Scope,
    (asking, watch): (&Asking, &Watch),
    lock_until: Instant,
) -> std::result::Result<Checked, (String, bool)> {
    let within_turn = scope.trees.is_some();
    let configured = context(place, &scope)
        .and_then(|context| arguments(&context, scope).map(|args| (context, args)));
    match configured {
        Ok((context, args)) => {
            run(context, args, (asking, watch), lock_until).map_err(|e| (format!("{e:#}"), false))
        }
        Err(e) => Err((format!("{e:#}"), within_turn)),
    }
}

/// The check's result as the hook reports it: what the worker sent, or why
/// it sent nothing within `waited` seconds, from what `watch` saw of the
/// provider.
fn finished(
    received: std::result::Result<
        std::result::Result<Checked, (String, bool)>,
        mpsc::RecvTimeoutError,
    >,
    watch: &Watch,
    waited: u128,
) -> std::result::Result<Checked, Unfinished> {
    match received {
        Ok(Ok(checked)) => Ok(checked),
        Ok(Err((reason, start_unreadable))) => Err(Unfinished {
            reason,
            outage: watch.failure(),
            start_unreadable,
        }),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(match watch.failure() {
            Some(failure) => Unfinished {
                outage: Some(failure.clone()),
                ..Unfinished::new(format!(
                    "{failure}; the check did not finish within {waited} s"
                ))
            },
            None if watch.silent() => Unfinished {
                outage: Some(format!("no answer within {waited} s")),
                ..Unfinished::new(format!("the provider did not answer within {waited} s"))
            },
            None => Unfinished::new(format!(
                "the check did not finish within {waited} s (answers received so far are cached)"
            )),
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(Unfinished::new("the check stopped unexpectedly".into()))
        }
    }
}

/// The repository's configuration for a check of `scope`: within a turn,
/// jevgate.toml and the custom questions as the turn began, so the agent's
/// edits to them, even one that deletes, lowers or breaks a question, count
/// from the next turn; else those there now.
fn context(place: &Place, scope: &Scope) -> Result<ConfigContext> {
    let Some((start, _)) = &scope.trees else {
        return ConfigContext::discover_in(&place.cwd, None, None);
    };
    let config = configuration_at(&place.root, start)?;
    let questions = crate::custom::load_at(
        &place.root,
        start,
        (Path::new(CONFIG_FILE), &config.question),
    )
    .context("Invalid custom questions as the turn began")?;
    Ok(ConfigContext {
        invocation_dir: place.cwd.clone(),
        root: place.root.clone(),
        config,
        questions: Box::leak(questions.into_boxed_slice()),
    })
}

/// The check itself: what `jevgate check` does with the repository's
/// configuration and `args`, less the output.
fn run(
    context: ConfigContext,
    args: CheckArgs,
    (asking, watch): (&Asking, &Watch),
    lock_until: Instant,
) -> Result<Checked> {
    let paths = inventory::scope(&args, &context)?;
    let inputs = inventory::collect(&args, &context, &paths)?;
    if inputs.is_empty() {
        // Nothing the rules judge changed: no report replaces the last one,
        // but an edit to jevgate.toml or the baseline is still a guard.
        let guards = guards::scan(&context.root, &args, &context.config, &paths).guards;
        return Ok(Checked {
            guards,
            ..Checked::default()
        });
    }
    let store = open_store(&context.root, lock_until)?;
    let (previous, mut report) = check::first_snapshot(&args, &context, &inputs);
    report.command = REPORT_COMMAND.into();
    let mut evaluator = asking.evaluator(&args, &context, watch)?;
    let mut session = check::session(&args, &context, &store, &mut evaluator);
    check::judge(&mut session, &inputs, previous.as_ref(), &mut report)?;
    if !report.complete {
        bail!(report.incomplete_reason());
    }
    let (accepted_now, dismissed) = match args.turn_start() {
        Some(start) => (
            accepted_now(&context.root, &report),
            dismissed(&context.root, &report, start),
        ),
        None => (BTreeSet::new(), Vec::new()),
    };
    Ok(Checked {
        flagged: flag(&report, &accepted_now),
        dismissed,
        undecided: undecided(&report, &args),
        guards: std::mem::take(&mut report.guards),
        unreviewed: unreviewed(&report, &args),
    })
}

/// The findings of `report` the agent dismissed with a reason since the
/// turn began at `start`, which the report's gate already accepts.
fn dismissed(root: &Path, report: &Report, start: &str) -> Vec<Dismissed> {
    let dismissals = crate::baseline::dismissed_since(root, start).unwrap_or_default();
    report
        .files
        .iter()
        .flat_map(|file| file.findings.iter().map(move |f| (file, f)))
        .filter(|(_, f)| f.baselined)
        .filter_map(|(file, f)| {
            let dismissal = dismissals.get(&f.fingerprint)?;
            Some(Dismissed {
                path: file.path.clone(),
                line: f.line,
                rule: f.rule.clone(),
                reason: dismissal.reason,
                note: dismissal.note.clone(),
            })
        })
        .collect()
}

/// jevgate.toml as it was in Git tree `start`, the turn's start; the
/// defaults when it had none.
fn configuration_at(root: &Path, start: &str) -> Result<Config> {
    let path = Path::new(CONFIG_FILE);
    match revision::blobs(root, start, &[path], CONFIG_BYTES)?.remove(path) {
        Some(text) => toml::from_str(&text).context("Invalid jevgate.toml as the turn began"),
        None => Ok(Config::default()),
    }
}

/// The fingerprints of the findings the baseline and allow comments accept
/// now, though not when the turn began nor by a dismissal with a reason:
/// the turn's own edits accepted them, which count from the next turn. A
/// baseline the turn left unreadable accepts nothing; its guard tells the
/// person.
fn accepted_now(root: &Path, report: &Report) -> BTreeSet<String> {
    let mut now = report.clone();
    crate::suppress::apply(root, &mut now, &BTreeSet::new(), None);
    let _ = crate::baseline::apply(root, &mut now, None);
    let then = report.files.iter().flat_map(|f| &f.findings);
    now.files
        .iter()
        .flat_map(|f| &f.findings)
        .zip(then)
        .filter(|(now, then)| now.accepted() && !then.accepted())
        .map(|(now, _)| now.fingerprint.clone())
        .collect()
}

/// `check`'s arguments with the repository's configuration, for `scope`.
fn arguments(context: &ConfigContext, scope: Scope) -> Result<CheckArgs> {
    let mut args = CheckArgs::defaults();
    check::configure(&mut args, context)?;
    // Never printed: the hook answers with its own JSON.
    args.format = Some(Format::Json);
    args.paths = scope.paths;
    if let Some((base, now)) = scope.trees {
        args.base = Some(base);
        args.now = crate::revision::Now::Snapshot(now);
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

/// The files of code in `report` that the check skipped or could not send
/// whole, with the reason the report gives.
fn unreviewed(report: &Report, args: &CheckArgs) -> Vec<Unreviewed> {
    let files = report
        .files
        .iter()
        .filter(|f| {
            matches!(f.status, Status::Skipped | Status::NeedsContext) || unjudged_tests(f, args)
        })
        .filter(|f| {
            matches!(
                f.role.as_str(),
                "source" | "test" | "generated" | "vendored"
            )
        })
        .map(|f| Unreviewed {
            path: f.path.clone(),
            why: f
                .error
                .clone()
                .or_else(|| f.classification.as_ref().map(|c| c.reason.clone()))
                .filter(|why| !why.is_empty())
                .unwrap_or_else(|| "it was not judged".into()),
        });
    // Code the parser could not read in the files it judged: the check
    // names only what the turn touched, so each is the agent's to know of.
    let units = crate::output::left_out(report)
        .into_iter()
        .map(|(path, entry)| Unreviewed {
            path: path.to_path_buf(),
            why: format!(
                "{} at line {} was left out, the rest of the file judged: {}",
                crate::output::left_out_unit(entry),
                entry.start_line,
                entry.reason
            ),
        });
    files.chain(units).collect()
}

/// Whether `file` is a test file the check did not judge although tests
/// were asked for: a preview language's, whose tests JevGate does not
/// judge yet.
fn unjudged_tests(file: &crate::schema::FileResult, args: &CheckArgs) -> bool {
    args.include_tests
        && file.status == Status::NotApplicable
        && file
            .classification
            .as_ref()
            .is_some_and(|c| c.kind == crate::file_kind::TESTS && c.gate == "excluded")
}

/// The units whose undecided results fail the gate, as `gate` counts their
/// files: each unit a rule whose level includes `uncertain` left undecided,
/// with its open questions, or the file when none is named, as for a file
/// that needs more context.
fn undecided(report: &Report, args: &CheckArgs) -> Vec<Undecided> {
    crate::gate::undecided_files(report, args)
        .flat_map(|file| {
            let units: Vec<Undecided> = file
                .dimensions
                .iter()
                .filter(|(rule, _)| crate::gate::fails_undecided(args, rule, &file.path))
                .flat_map(|(rule, dimension)| {
                    dimension.undecided.iter().map(|unit| Undecided {
                        path: file.path.clone(),
                        line: unit.line,
                        rule: crate::catalog::id(rule).to_string(),
                        what: format!("`{}`: {}", unit.unit, unit.questions.join("; ")),
                    })
                })
                .collect();
            if units.is_empty() {
                let why = file
                    .error
                    .clone()
                    .unwrap_or_else(|| "JevGate needs more context to judge this file".to_string());
                vec![Undecided {
                    path: file.path.clone(),
                    line: 1,
                    rule: "file".into(),
                    what: why,
                }]
            } else {
                units
            }
        })
        .collect()
}

/// The findings the agent may act on: not notes and not accepted (as the
/// turn began or dismissed since, within a turn), those that fail the gate
/// first, then by rank; `accepted_now` are those the turn's own edits
/// accepted.
fn flag(report: &Report, accepted_now: &BTreeSet<String>) -> Vec<Flagged> {
    let mut flagged: Vec<Flagged> = report
        .files
        .iter()
        .flat_map(|file| {
            file.findings
                .iter()
                .filter(|f| f.strength != Strength::Note && !f.accepted())
                .map(|finding| Flagged {
                    path: file.path.clone(),
                    accepted_this_turn: accepted_now.contains(&finding.fingerprint),
                    finding: finding.clone(),
                })
        })
        .collect();
    flagged.sort_by(|a, b| {
        b.fails()
            .cmp(&a.fails())
            .then(b.finding.rank.total_cmp(&a.finding.rank))
    });
    flagged
}
