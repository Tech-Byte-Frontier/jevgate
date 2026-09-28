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
pub(crate) fn credential_path(args: &CheckArgs, context: &ConfigContext) -> std::path::PathBuf {
    args.env_file
        .as_ref()
        .map(|p| context.input_path(p))
        .unwrap_or_else(|| context.root.join(".env"))
}

/// `args` as every check runs them: the repository's configuration, and the
/// provider of the key the check will use, which its default model follows.
pub(crate) fn configure(args: &mut CheckArgs, context: &ConfigContext) -> Result<()> {
    context.configure(args)?;
    args.provider = crate::auth::sources::planned_provider(
        &credential_path(args, context),
        args.env_file.is_some(),
    );
    Ok(())
}

/// Record a failed evaluation in the snapshot (and report) before returning the error.
fn publish_failure(
    session: &evaluate::Session<'_>,
    report: &mut schema::Report,
    error: anyhow::Error,
) -> Result<()> {
    report.watcher_pid = None;
    report.errors.push(error.to_string());
    report.update_status();
    session.publish(report)?;
    if session.args.report {
        html_report::open(&session.context.root);
    }
    Err(error)
}

/// The last published report and the first snapshot of this run, numbered
/// after it.
pub fn first_snapshot(
    args: &CheckArgs,
    context: &ConfigContext,
    inputs: &[inventory::Input],
) -> (Option<schema::Report>, schema::Report) {
    let previous = storage::read_latest(&context.root).ok();
    let report = evaluate::snapshot(
        inputs,
        &evaluate::previous_judgments(previous.as_ref(), args.refresh),
        args,
        evaluate::SnapshotContext {
            root: &context.root,
            generation: previous.as_ref().map_or(1, |r| r.generation + 1),
            requests: 0,
        },
    );
    (previous, report)
}

/// A session that asks `evaluator` and records answers in `store`.
pub fn session<'a>(
    args: &'a CheckArgs,
    context: &'a ConfigContext,
    store: &'a storage::Store,
    evaluator: &'a mut dyn transport::Evaluator,
) -> evaluate::Session<'a> {
    evaluate::Session {
        args,
        context,
        store,
        evaluator,
        requests: 0,
        paid: Default::default(),
        budget: token_budget::TokenBudget::load(&context.root),
        observed: (0, 0),
    }
}

/// Evaluate the snapshot, compare it with the previous report, apply the
/// gate and publish it: what every check does, and the agent hook's checks.
pub fn judge(
    session: &mut evaluate::Session<'_>,
    inputs: &[inventory::Input],
    previous: Option<&schema::Report>,
    report: &mut schema::Report,
) -> Result<()> {
    if let Err(error) = session.evaluate(inputs, report) {
        return publish_failure(session, report, error);
    }
    changes::compare(previous, report);
    gate::settle(&session.context.root, report, session.args)?;
    report.settled = true;
    session.publish(report)
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
    let (previous, mut report) = first_snapshot(args, context, &inputs);
    if args.dry_run {
        evaluate::preview_guards(&mut report, args, context, &scope);
        output::emit(&report, args)?;
        return Ok(0);
    }
    let store = store.unwrap();
    let mut client = transport::Client::new(
        &credential_path(args, context),
        args.env_file.is_some(),
        args.provider,
    )?;
    let mut session = session(args, context, &store, &mut client);
    judge(&mut session, &inputs, previous.as_ref(), &mut report)?;
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
