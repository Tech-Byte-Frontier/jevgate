//! `check`: collect the selected files, evaluate them, apply the gate, and
//! report once or keep watching.
use crate::{
    cancellation, changes,
    config::ConfigContext,
    evaluate, gate, html_report, inventory,
    options::{CheckArgs, Format},
    output, schema, storage, token_budget, transport, watch,
};
use anyhow::Result;

fn validate(args: &CheckArgs) -> Result<()> {
    anyhow::ensure!(
        !args.show_requests || args.output_format() == Format::Json,
        "--show-requests uses JSON output; omit --format or use --format json"
    );
    anyhow::ensure!(
        !(args.watch && args.dry_run),
        "--watch cannot be combined with --dry-run"
    );
    anyhow::ensure!(
        !(args.watch
            && matches!(
                args.output_format(),
                Format::Json | Format::Github | Format::Sarif | Format::Gitlab
            )),
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

/// `check`: judge the selected files, apply the gate and report.
pub fn run(args: &CheckArgs, context: &ConfigContext) -> Result<u8> {
    validate(args)?;
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
    let mut client =
        transport::Client::new(&credential_path(args, context), args.env_file.is_some());
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
