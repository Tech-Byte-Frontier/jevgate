//! `check`: collect the selected files, evaluate them, apply the gate, and
//! report once or keep watching.
use crate::{
    cancellation, changes,
    config::ConfigContext,
    evaluate, gate, git_hooks,
    hook::outage,
    html_report, inventory,
    options::{CheckArgs, Format},
    output, progress,
    revision::{self, Now, Recorded, push},
    schema, storage, token_budget, transport, watch,
};
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    io::{IsTerminal, Read},
    path::Path,
};

/// Flags that cannot run together: a usage error, which exits 2 even when
/// a run that cannot finish would pass.
pub(crate) fn validate(args: &CheckArgs) -> Result<()> {
    anyhow::ensure!(
        !args.show_requests || args.output_format() == Format::Json,
        "--show-requests uses JSON output; omit --format or use --format json"
    );
    anyhow::ensure!(
        !(args.watch && args.dry_run),
        "--watch cannot be combined with --dry-run"
    );
    anyhow::ensure!(
        !(args.watch && args.moment().is_some()),
        "--watch judges the working tree as it changes; it cannot be combined with --staged or --pre-push"
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
    args.provider = planned_provider(args, context);
    Ok(())
}

/// The provider of the key a check will use, found before planning so the
/// default model follows the key.
pub(crate) fn planned_provider(
    args: &CheckArgs,
    context: &ConfigContext,
) -> crate::provider::Provider {
    crate::auth::sources::planned_provider(&credential_path(args, context), args.env_file.is_some())
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
        answered: Default::default(),
        spend: args.max_cost.map(crate::requests::Spend::new),
        halted: None,
        budget_noted: false,
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

/// After a Git hook's check that asked the provider: a failure that passes
/// with time is waited out by the next hooks' checks for a few minutes, and
/// a check that finished clears the one waited out.
fn remember_outage(root: &Path, report: &schema::Report, watch: &outage::Watch) {
    match watch.failure() {
        Some(failure) if !report.complete => outage::record(root, &failure),
        _ if report.complete => outage::clear(root),
        _ => {}
    }
}

/// Say which selected custom questions this run cannot ask, and what they
/// need: a question about changed hunks needs `--base`, and one about tests
/// needs them judged.
fn unasked(args: &CheckArgs) {
    for question in args.custom() {
        let needs = match question.unit {
            crate::custom::Kind::Hunk if args.base.is_none() => "--base",
            crate::custom::Kind::Test if !args.include_tests => "--include-tests",
            _ => continue,
        };
        note!("jevgate: {} was not asked: it needs {needs}", question.rule);
    }
}

/// Resolve the change a check judges: `--base` to its fork point with HEAD,
/// the working tree's change from it; `--staged` to HEAD, the index's
/// change from it, with the files it touched read from Git.
pub(crate) fn resolve_change(args: &mut CheckArgs, root: &Path) -> Result<()> {
    if args.staged {
        let base = revision::staged_base(root)?;
        args.recorded = Some(Recorded::load(root, &base, &Now::Index)?);
        args.now = Now::Index;
        args.base = Some(base);
    } else if let Some(base) = &args.base {
        args.base = Some(revision::resolve(root, base)?);
    }
    Ok(())
}

/// Bytes of pre-push input read: a line per pushed ref, about 200 bytes.
const PUSH_INPUT_BYTES: u64 = 4 * 1024 * 1024;

/// `check --pre-push`: judge what each pushed ref sends, as committed, with
/// one check per distinct change; the highest exit code of them.
pub(crate) fn pre_push(args: &mut CheckArgs, context: &ConfigContext) -> Result<u8> {
    let root = &context.root;
    let mut judged = BTreeSet::new();
    let mut code = 0;
    for update in pushed_refs()? {
        match push::sent(root, &update)? {
            push::Sent::Commits { base, commit } => {
                if !judged.insert((base.clone(), commit.clone())) {
                    continue;
                }
                let now = Now::Commit(commit);
                args.recorded = Some(Recorded::load(root, &base, &now)?);
                args.now = now;
                args.base = Some(base);
                code = code.max(run(args, context)?);
            }
            push::Sent::Nothing => {}
            push::Sent::Unrooted => {
                let why = format!(
                    "none of the commits of {} is on a remote yet, so there is nothing to compare them with; `jevgate check` judges the whole repository",
                    update.name
                );
                if args.passes_incomplete() {
                    git_hooks::not_checked(args, &why);
                } else {
                    note!("jevgate: this push cannot be checked: {why}.");
                    code = code.max(2);
                }
            }
        }
    }
    if judged.is_empty() {
        note!("jevgate: nothing to check: this push sends no commit a remote lacks.");
    }
    Ok(code)
}

/// The refs a pre-push check judges: the one pre-commit or prek names, else
/// those Git passes the hook on stdin, else, on a terminal, the current
/// branch.
fn pushed_refs() -> Result<Vec<push::Update>> {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    if let Some(update) = push::from_pre_commit(var) {
        return Ok(vec![update]);
    }
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Ok(vec![push::Update::head()]);
    }
    let mut input = String::new();
    stdin
        .lock()
        .take(PUSH_INPUT_BYTES)
        .read_to_string(&mut input)
        .context("Cannot read the refs Git passed the pre-push hook")?;
    push::updates(&input)
}

/// `check`: judge the selected files, apply the gate and report.
pub fn run(args: &CheckArgs, context: &ConfigContext) -> Result<u8> {
    let started = std::time::Instant::now();
    validate(args)?;
    cancellation::install()?;
    unasked(args);
    let progress =
        progress::start(progress::wanted() && !args.watch && args.output_format() != Format::Jsonl);
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
        drop(progress);
        output::emit(&report, args)?;
        return Ok(0);
    }
    let store = store.unwrap();
    let mut client = transport::Client::new(
        &credential_path(args, context),
        args.env_file.is_some(),
        args.provider,
    )?;
    if let Some(seconds) = args.max_seconds {
        client = client.until(started + std::time::Duration::from_secs(seconds));
    }
    // A Git hook's check waits out a provider failure as the agent hook does.
    let waiting = args.moment().and_then(|_| outage::current(&context.root));
    let watch = outage::Watch::default();
    let mut evaluator = outage::Watched {
        inner: match &waiting {
            Some(outage) => Box::new(outage::Waiting(crate::hook::text::waiting(outage))),
            None => Box::new(client),
        },
        watch: &watch,
    };
    let mut session = session(args, context, &store, &mut evaluator);
    judge(&mut session, &inputs, previous.as_ref(), &mut report)?;
    drop(progress);
    if let Some(error) = session.halted.take().filter(|_| !args.watch) {
        // No request could be sent, as without a key: say why once, not
        // once per file. The report keeps each file's error.
        return Err(error);
    }
    if args.moment().is_some() && waiting.is_none() {
        remember_outage(&context.root, &report, &watch);
    }
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
    let ran_out = args
        .max_seconds
        .is_some_and(|seconds| started.elapsed().as_secs() >= seconds);
    let code = git_hooks::exit_code(&report, args, ran_out);
    if let Some(moment) = args.moment().filter(|_| code == 1)
        && args.output_format() == Format::Agent
    {
        say!("{}", git_hooks::stopped(moment));
    }
    Ok(code)
}
