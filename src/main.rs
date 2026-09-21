mod auth;
mod cancellation;
mod cascade;
mod catalog;
mod changes;
mod config;
mod context;
mod context_units;
mod discovery;
mod evaluate;
mod file_kind;
mod html_report;
mod inventory;
mod locations;
mod maintainability;
mod options;
mod output;
mod questions;
mod repetition;
mod requests;
mod response;
mod revision;
mod roles;
mod schema;
mod server;
mod storage;
mod transport;
mod watch;

use anyhow::Result;
use clap::Parser;
use config::ConfigContext;
use options::{CheckArgs, Format, JevCommand};

#[derive(Parser)]
#[command(version, about = "Incremental advisory code review with TypeSafe Jev")]
struct Cli {
    #[command(subcommand)]
    command: JevCommand,
}

fn main() -> std::process::ExitCode {
    let result = run(Cli::parse().command);
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("jevgate: {error:#}");
            2
        }
    };
    std::process::ExitCode::from(cancellation::signal().map_or(code, |s| (128 + s) as u8))
}

fn run(command: JevCommand) -> Result<u8> {
    if let JevCommand::Auth { command } = command {
        return auth::run(command);
    }
    let context = ConfigContext::discover()?;
    match command {
        JevCommand::Auth { .. } => unreachable!("auth handled before repository configuration"),
        JevCommand::Check(mut args) => {
            context.configure(&mut args)?;
            if let Some(base) = &args.base {
                args.base = Some(revision::resolve(&context.root, base)?);
            }
            check(&args, &context)
        }
        JevCommand::Rules => {
            println!("{}", serde_json::to_string_pretty(&catalog::describe())?);
            Ok(0)
        }
        JevCommand::Serve { port } => {
            cancellation::install()?;
            server::run(&context.root, port)?;
            Ok(0)
        }
    }
}

fn check(args: &CheckArgs, context: &ConfigContext) -> Result<u8> {
    anyhow::ensure!(
        !args.show_requests || args.output_format() == Format::Json,
        "--show-requests uses JSON output; omit --format or use --format json"
    );
    anyhow::ensure!(
        !(args.watch && args.dry_run),
        "--watch cannot be combined with --dry-run"
    );
    anyhow::ensure!(
        !(args.watch && args.output_format() == Format::Json),
        "Use --format jsonl for watch snapshots"
    );
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
        output::emit(&report, args.output_format())?;
        return Ok(0);
    }
    let store = store.unwrap();
    let credential = args
        .env_file
        .as_ref()
        .map(|p| context.input_path(p))
        .unwrap_or_else(|| context.root.join(".env"));
    let mut client = transport::Client::new(&credential, args.env_file.is_some());
    let mut session = evaluate::Session {
        args,
        context,
        store: &store,
        evaluator: &mut client,
        requests: 0,
        paid_input_tokens: 0,
        paid_output_tokens: 0,
    };
    if let Err(error) = session.evaluate(&inputs, &mut report) {
        report.watcher_pid = None;
        report.errors.push(error.to_string());
        report.update_status();
        session.publish(&report)?;
        if args.report {
            html_report::open(&context.root);
        }
        return Err(error);
    }
    changes::compare(baseline.as_ref(), &mut report);
    report.settled = true;
    session.publish(&report)?;
    if args.report {
        html_report::open(&context.root);
    }
    if args.output_format() != Format::Jsonl {
        output::emit(&report, args.output_format())?;
    }
    if args.watch {
        watch::run(&mut session, scope, inputs, report)?;
        return Ok(0);
    }
    Ok(outcome(&report))
}

fn outcome(report: &schema::Report) -> u8 {
    if report.complete { 0 } else { 2 }
}

#[cfg(test)]
mod tests;
