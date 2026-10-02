//! Checks run from Git hooks, before a commit (`--staged`) or a push
//! (`--pre-push`), as Qlty's slop-one runs before a push: a run that cannot
//! finish lets the change through and says so loudly, a person watching can
//! skip the check with Enter, and a blocked commit or push tells a coding
//! agent what to do instead of `--no-verify`. Writing the hooks is in
//! `install`.
use crate::{
    options::{CheckArgs, Moment},
    schema::Report,
};
use std::{
    io::{BufRead, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub mod install;

/// Say on stderr that a run which could not finish let the change through:
/// `why` it could not, and how to make such a run stop the change instead.
/// It is the last thing printed, in bold on a terminal, since a hook's
/// output is often read only when something goes wrong.
pub fn not_checked(args: &CheckArgs, why: &str) {
    let why = why.trim_end_matches('.');
    let (first, second) = match args.moment() {
        Some(moment) => (
            format!(
                "jevgate: this {} was not checked: {why}.",
                moment.noun()
            ),
            "jevgate: it goes ahead unchecked, as on_incomplete is \"pass\"; set on_incomplete = \"fail\" in jevgate.toml to stop it instead.".to_string(),
        ),
        None => (
            format!("jevgate: the check did not finish: {why}."),
            "jevgate: it exits 0, as on_incomplete is \"pass\"; what it reported is not the whole review.".to_string(),
        ),
    };
    let bold = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    for line in [first, second] {
        if bold {
            note!("\x1b[1;33m{line}\x1b[0m");
        } else {
            note!("{line}");
        }
    }
}

/// What a person reads after a report whose gate stopped a commit or a
/// push, and a coding agent that ran `git commit` or `git push` reads as
/// its next step: fix each finding, or dismiss it with a reason, as in the
/// agent hook. Accepting findings wholesale stays with people, and the hook
/// is never bypassed.
pub fn stopped(moment: Moment) -> String {
    let noun = moment.noun();
    format!(
        "\nJevGate stopped this {noun}: fix the findings above and {noun} again. A person who judges a finding acceptable can add a `jevgate: allow(RULE) reason` comment on its line, or run `jevgate baseline --merge` and commit jevgate-baseline.json. Coding agents: fix each finding, or dismiss one that is mistaken, intended or left for later with `jevgate baseline mark wrong|intended|later PATH:LINE` and commit jevgate-baseline.json with the change, which the person audits with `jevgate baseline stats`; never bypass this check with --no-verify or an allow comment."
    )
}

/// How long a check runs before a person is offered the skip: one answered
/// from the cache ends first, and prints nothing more.
const OFFER_AFTER: Duration = Duration::from_secs(1);

/// The terminal a person types on, read for Enter.
#[cfg(windows)]
const TERMINAL: &str = "CONIN$";
#[cfg(not(windows))]
const TERMINAL: &str = "/dev/tty";

/// While it is alive, Enter skips the check; dropped when the check ends.
pub struct Skip {
    ended: Arc<AtomicBool>,
}

impl Drop for Skip {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::SeqCst);
    }
}

/// Offer a person the skip once the check has run a second: Enter then
/// exits 0, so Git goes on with the commit or push, and a line says it was
/// not checked. Only when stderr is a terminal outside CI, so a person
/// reads the offer: a coding agent's shell reads stderr through a pipe,
/// and the terminal device there belongs to whoever runs the agent, whose
/// keys must not be taken. The answers already received stay cached,
/// since the cache is written file by file.
pub fn offer_skip(moment: Moment) -> Option<Skip> {
    if !std::io::stderr().is_terminal() || std::env::var_os("CI").is_some() {
        return None;
    }
    let terminal = std::fs::File::open(TERMINAL).ok()?;
    let ended = Arc::new(AtomicBool::new(false));
    let watching = Arc::clone(&ended);
    std::thread::spawn(move || {
        std::thread::sleep(OFFER_AFTER);
        if watching.load(Ordering::SeqCst) {
            return;
        }
        let noun = moment.noun();
        note!("jevgate: press Enter to skip the check; the {noun} then goes ahead unchecked.");
        let mut lines = std::io::BufReader::new(terminal).lines();
        while let Some(Ok(line)) = lines.next() {
            if line.trim().is_empty() && !watching.load(Ordering::SeqCst) {
                note!("jevgate: skipped with Enter; this {noun} was not checked.");
                std::process::exit(0);
            }
        }
    });
    Some(Skip { ended })
}

/// The exit code of a finished check: an incomplete run passes when
/// `args` lets it, saying so, and otherwise the gate decides. `ran_out`:
/// the run used up its `--max-seconds`.
pub fn exit_code(report: &Report, args: &CheckArgs, ran_out: bool) -> u8 {
    let code = crate::gate::exit_code(report);
    // An interrupted run stops the commit, whatever lets it through.
    let interrupted = crate::cancellation::signal().is_some();
    if code == 2 && args.passes_incomplete() && !report.dry_run && !interrupted {
        let why = match args.max_seconds.filter(|_| ran_out) {
            Some(seconds) => format!(
                "it used its {} (max_seconds) before every answer came back; the answers it received are cached, so the next run asks only for the rest",
                crate::output::count(seconds as usize, "second")
            ),
            None => report.incomplete_reason(),
        };
        not_checked(args, &why);
        return 0;
    }
    code
}
