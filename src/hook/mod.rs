//! `jevgate hook`: one coding-agent hook event read on stdin, answered with
//! one JSON reply on stdout. A turn's start records a snapshot of the working
//! tree; an edit is checked and its findings reach the agent as context; the
//! end of a turn is checked against its start and blocked while findings
//! fail the gate, at most three times. The hook always exits 0: agents read
//! exit 2 as a block and exit 1 as silence, the opposite of `check`, so an
//! outage, an HTTP 402 or a missing key never blocks the agent, and the reply
//! always says so. What each event does is in `events`.
mod agents;
mod events;
mod review;
#[cfg(test)]
mod tests;
mod text;
mod turn;

pub use agents::Agent;

use crate::transport;
use agents::{Event, Kind, Reply};
use anyhow::{Result, bail};
use events::{Hook, failed};
use review::Evaluators;
use serde_json::{Value, json};
use std::{
    io::{IsTerminal, Read},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// Hook input larger than this is refused: a `Write` event carries the whole
/// file twice, and 64 MiB leaves room for any source JevGate reads.
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
/// Kept from the budget to write the reply.
const REPLY_MARGIN: Duration = Duration::from_millis(250);

/// `jevgate hook`'s arguments: the agent and the time the hook may take.
#[derive(clap::Args, Debug)]
pub struct HookArgs {
    /// The agent that runs the hook [default: detected from the event]
    #[arg(long, value_enum)]
    pub agent: Option<Agent>,
    /// Seconds before the hook gives up and lets the agent go on [default: 10 at a session or turn start, 30 after an edit, 50 at the end of a turn]
    ///
    /// Keep it below the agent's own hook timeout: an agent that stops the
    /// hook first discards its reply, so the person is not told why.
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout: Option<u64>,
}

/// How the hook runs: the agent when not detected, and its time budget.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Options {
    pub agent: Option<Agent>,
    pub timeout: Option<Duration>,
}

/// What the hook needs beyond the event, so tests can script it.
pub(crate) struct Host {
    /// Builds the evaluator a check asks.
    pub evaluators: Arc<Evaluators>,
    /// The directory of an event that names none.
    pub cwd: PathBuf,
}

/// A reply, and the person's message when the agent shows none from a hook
/// (Cursor shows stderr in its Hooks output channel).
pub(crate) struct Answer {
    pub json: Value,
    pub stderr: Option<String>,
}

/// `jevgate hook`: answer the event on stdin. Only a person running it on a
/// terminal, with no event to read, gets an error.
pub fn run(args: &HookArgs) -> Result<u8> {
    if std::io::stdin().is_terminal() {
        bail!(
            "jevgate hook reads one hook event as JSON on stdin; coding agents run it (see jevgate hook --help)"
        );
    }
    let options = Options {
        agent: args.agent,
        timeout: args.timeout.map(Duration::from_secs),
    };
    let host = Host {
        evaluators: Arc::new(|args, context| {
            let client = transport::Client::new(
                &crate::check::credential_path(args, context),
                args.env_file.is_some(),
                args.provider,
            )?;
            Ok(Box::new(client))
        }),
        cwd: std::env::current_dir().unwrap_or_default(),
    };
    let answer = match read_event(std::io::stdin().lock()) {
        Ok(input) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            respond(&input, options, &host)
        }))
        .unwrap_or_else(|panic| {
            unreadable(&format!(
                "JevGate's hook failed unexpectedly ({})",
                panic_message(panic.as_ref())
            ))
        }),
        Err(error) => unreadable(&format!("JevGate could not read the hook event: {error:#}")),
    };
    say!("{}", answer.json);
    if let Some(message) = answer.stderr {
        note!("{message}");
    }
    Ok(0)
}

/// Whether this process runs `jevgate hook`, so a usage error is answered
/// as a hook reply and not with clap's exit 2, which agents read as a block.
pub fn invoked() -> bool {
    std::env::args_os()
        .nth(1)
        .is_some_and(|name| name == "hook")
}

/// The reply to invalid `jevgate hook` arguments: exit 0, and the error for the person.
pub fn usage_error(error: &clap::Error) -> std::process::ExitCode {
    let first = error.to_string();
    let first = first.lines().next().unwrap_or_default();
    let answer = unreadable(&format!(
        "JevGate's hook command is invalid ({})",
        first.trim_start_matches("error: ")
    ));
    say!("{}", answer.json);
    std::process::ExitCode::SUCCESS
}

/// A reply for any agent that nothing was checked, and why.
fn unreadable(reason: &str) -> Answer {
    let message = format!("{reason}. Nothing was checked or blocked.");
    Answer {
        json: json!({ "systemMessage": message }),
        stderr: Some(message),
    }
}

/// What a panic said, when it said it in text.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic")
}

/// One JSON object from `reader`, read to its closing brace, so a host that
/// keeps stdin open does not stall the hook.
fn read_event(reader: impl Read) -> Result<Value> {
    let mut values =
        serde_json::Deserializer::from_reader(reader.take(MAX_INPUT_BYTES)).into_iter::<Value>();
    match values.next() {
        Some(Ok(value)) if value.is_object() => Ok(value),
        Some(Ok(_)) => bail!("it is not a JSON object"),
        Some(Err(error)) => bail!("it is not valid JSON ({error})"),
        None => bail!("stdin was empty"),
    }
}

/// The reply to one hook event.
pub(crate) fn respond(input: &Value, options: Options, host: &Host) -> Answer {
    let started = Instant::now();
    let agent = options.agent.unwrap_or_else(|| agents::detect(input));
    let event = agents::event(agent, input);
    let budget = options.timeout.unwrap_or_else(|| budget(event.kind));
    let deadline = started + budget.saturating_sub(REPLY_MARGIN);
    let reply = match event.kind {
        Kind::Other => Reply::default(),
        _ => match Hook::open(&event, host, deadline) {
            Ok(hook) => hook.handle(),
            Err(error) => failed(&event, &what(&event), &format!("{error:#}")),
        },
    };
    Answer {
        json: agents::render(&event, &reply),
        stderr: reply.user.filter(|_| agent == Agent::Cursor),
    }
}

/// The default budget of an event: a snapshot is quick, a check of an edit
/// is meant to take seconds, and the end of a turn checks every changed file.
/// All stay under Gemini CLI's 60 s and Claude Code's 30 s for prompts.
fn budget(kind: Kind) -> Duration {
    Duration::from_secs(match kind {
        Kind::SessionStart | Kind::TurnStart | Kind::Other => 10,
        Kind::AfterEdit => 30,
        Kind::Stop => 50,
    })
}

/// What an event checks, for a failure's wording.
fn what(event: &Event) -> String {
    match event.kind {
        Kind::AfterEdit => text::named(&event.files),
        _ => "this turn".into(),
    }
}
