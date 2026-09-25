/// Print a line to stdout. A closed pipe (as with `| head`) is not an error:
/// the reader has what it wanted, so the write failure is ignored.
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($arg)*);
    }};
}

/// Print a line to stderr, ignoring a closed stream like [`say!`].
macro_rules! note {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($arg)*);
    }};
}

mod analysis;
mod auth;
mod baseline;
mod boundary;
mod cancellation;
mod catalog;
mod changes;
mod components;
mod config;
mod context;
mod context_units;
mod discovery;
mod docs;
mod evaluate;
mod file_kind;
mod gate;
mod github;
mod html_report;
mod init;
mod inventory;
mod line_ranges;
mod locations;
mod options;
mod output;
mod packages;
mod policy;
mod provider_error;
mod requests;
mod response;
mod revision;
mod schema;
mod server;
mod storage;
mod syntax;
mod test_locations;
mod token_budget;
mod transport;
mod units;
mod watch;

use anyhow::Result;
use clap::Parser;
use config::ConfigContext;
use options::{CheckArgs, Format, JevCommand};

/// Code review gate that asks TypeSafe Jev small, literal questions about your code
///
/// JevGate parses the repository locally and builds small evidence units: a
/// function, a file outline, a pair of copies, a test, a documentation
/// section. It asks TypeSafe Jev short, typed questions about each one, and
/// code, not a chat model, composes the answers into findings. Each finding
/// has a location, a probability and a next step, and undecided answers are
/// reported as uncertain instead of hidden.
///
/// Rule groups: maintainability (on by default), tests (with
/// --include-tests), and the opt-in security and documentation groups.
#[derive(Parser)]
#[command(version, after_long_help = options::OVERVIEW)]
struct Cli {
    #[command(subcommand)]
    command: JevCommand,
}

fn main() -> std::process::ExitCode {
    let result = run(Cli::parse().command);
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            note!("jevgate: {error:#}");
            2
        }
    };
    std::process::ExitCode::from(cancellation::signal().map_or(code, |s| (128 + s) as u8))
}

fn run(command: JevCommand) -> Result<u8> {
    if let JevCommand::Auth { command } = command {
        return auth::run(command);
    }
    if let JevCommand::Init { force } = command {
        // Before reading configuration, so an invalid file can be replaced.
        let root = config::repository_root(&std::env::current_dir()?.canonicalize()?);
        let (path, allow) = init::run(&root, force)?;
        say!("Wrote {}", path.display());
        if allow.is_empty() {
            say!("No supported source found; set upload_allow before checking.");
        } else {
            say!("Uploads limited to: {}", allow.join(", "));
        }
        say!("Next: jevgate auth login, then jevgate check --dry-run --show-requests");
        return Ok(0);
    }
    let file = match &command {
        JevCommand::Check(args) => args.config.clone(),
        _ => None,
    };
    let context = ConfigContext::discover(file.as_deref())?;
    match command {
        JevCommand::Auth { .. } | JevCommand::Init { .. } => {
            unreachable!("handled before repository configuration")
        }
        JevCommand::Check(mut args) => {
            context.configure(&mut args)?;
            if let Some(base) = &args.base {
                args.base = Some(revision::resolve(&context.root, base)?);
            }
            check(&args, &context)
        }
        JevCommand::Baseline {
            action: Some(action),
            ..
        } => baseline_action(&context, action),
        JevCommand::Baseline {
            merge,
            reason,
            action: None,
        } => {
            let written = baseline::write(&context.root, merge, reason)?;
            let path = written.path.display();
            if merge {
                say!(
                    "Accepted {} finding(s) from the last check in {path}; kept {} earlier finding(s) for files it did not cover",
                    written.accepted,
                    written.kept
                );
            } else {
                say!("Accepted {} finding(s) in {path}", written.accepted);
            }
            Ok(0)
        }
        JevCommand::Rules { format } => {
            match format {
                options::RulesFormat::Json => {
                    say!("{}", serde_json::to_string_pretty(&catalog::describe())?)
                }
                options::RulesFormat::Table => say!("{}", catalog::table()),
            }
            Ok(0)
        }
        JevCommand::Serve { port } => {
            cancellation::install()?;
            server::run(&context.root, port)?;
            Ok(0)
        }
    }
}

/// `baseline mark` and `baseline stats`: offline edits and counts of the baseline.
fn baseline_action(context: &ConfigContext, action: options::BaselineAction) -> Result<u8> {
    match action {
        options::BaselineAction::Mark {
            reason,
            targets,
            rules,
        } => {
            let mut keys = Vec::new();
            for name in &rules {
                keys.extend(
                    catalog::select(name)
                        .ok_or_else(|| anyhow::anyhow!("Unknown rule or group: {name}"))?,
                );
            }
            let marked = baseline::mark(&context.root, reason, &targets, &keys)?;
            say!(
                "Marked {marked} accepted finding(s) as {}",
                output::label(&reason)
            );
        }
        options::BaselineAction::Stats { format } => {
            let counts = baseline::stats(&context.root)?;
            match format {
                options::RulesFormat::Json => say!("{}", serde_json::to_string_pretty(&counts)?),
                options::RulesFormat::Table => say!("{}", baseline::stats_table(&counts)),
            }
        }
    }
    Ok(0)
}

fn validate_check(args: &CheckArgs) -> Result<()> {
    anyhow::ensure!(
        !args.show_requests || args.output_format() == Format::Json,
        "--show-requests uses JSON output; omit --format or use --format json"
    );
    anyhow::ensure!(
        !(args.watch && args.dry_run),
        "--watch cannot be combined with --dry-run"
    );
    anyhow::ensure!(
        !(args.watch && matches!(args.output_format(), Format::Json | Format::Github)),
        "Use --format jsonl for watch snapshots"
    );
    Ok(())
}

/// The credential file: `--env-file` from the invocation directory, else the root `.env`.
fn credential_path(args: &CheckArgs, context: &ConfigContext) -> std::path::PathBuf {
    args.env_file
        .as_ref()
        .map(|p| context.input_path(p))
        .unwrap_or_else(|| context.root.join(".env"))
}

/// Record a failed evaluation in the snapshot (and report) before returning the error.
fn publish_failure(
    session: &evaluate::Session<'_>,
    report: &mut schema::Report,
    error: anyhow::Error,
) -> Result<u8> {
    report.watcher_pid = None;
    report.errors.push(error.to_string());
    report.update_status();
    session.publish(report)?;
    if session.args.report {
        html_report::open(&session.context.root);
    }
    Err(error)
}

fn check(args: &CheckArgs, context: &ConfigContext) -> Result<u8> {
    validate_check(args)?;
    cancellation::install()?;
    let scope = inventory::scope(args, context)?;
    let inputs = inventory::collect(args, context, &scope)?;
    let store = if args.dry_run {
        None
    } else {
        Some(storage::Store::open(&context.root)?)
    };
    let baseline = storage::read_latest(&context.root).ok();
    let previous = evaluate::previous_judgments(baseline.as_ref(), args.refresh);
    let mut report = evaluate::snapshot(
        &inputs,
        &previous,
        args,
        evaluate::SnapshotContext {
            root: &context.root,
            generation: baseline.as_ref().map_or(1, |r| r.generation + 1),
            requests: 0,
        },
    );
    if args.dry_run {
        output::emit(&report, args)?;
        return Ok(0);
    }
    let store = store.unwrap();
    let mut client = transport::Client::new(
        &credential_path(args, context),
        args.env_file.is_some(),
        args.provider(),
    );
    let mut session = evaluate::Session {
        args,
        context,
        store: &store,
        evaluator: &mut client,
        requests: 0,
        paid_input_tokens: 0,
        paid_output_tokens: 0,
        budget: token_budget::TokenBudget::load(&context.root),
        observed: (0, 0),
    };
    if let Err(error) = session.evaluate(&inputs, &mut report) {
        return publish_failure(&session, &mut report, error);
    }
    changes::compare(baseline.as_ref(), &mut report);
    gate::settle(&context.root, &mut report, args)?;
    report.settled = true;
    session.publish(&report)?;
    if args.report {
        html_report::open(&context.root);
    }
    if args.output_format() != Format::Jsonl {
        output::emit(&report, args)?;
    }
    if args.watch {
        watch::run(&mut session, scope, inputs, report)?;
        return Ok(0);
    }
    Ok(gate::exit_code(&report))
}

#[cfg(test)]
mod tests;
