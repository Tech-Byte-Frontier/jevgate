//! Running each command: offline commands and the agent hook first (the hook
//! finds its repository from the event, and agent setup writes the agents'
//! files), then the ones that read the repository's configuration; `check`
//! runs in its own module.
use crate::{
    auth, baseline, cancellation, catalog, config,
    config::ConfigContext,
    git_hooks, hook, init, manual, mcp,
    options::{self, CheckArgs, JevCommand},
    output, server, setup,
};
use anyhow::Result;

pub fn run(command: JevCommand) -> Result<u8> {
    match command {
        JevCommand::Auth { command } => auth::run(command),
        JevCommand::Completions { shell } => manual::completions(shell).map(|()| 0),
        JevCommand::Man { command } => manual::man(command.as_deref()).map(|()| 0),
        JevCommand::Init { force, setup } => match setup.git_hook {
            Some(hook) => git_hooks::install::run(hook, setup.remove, setup.dry_run),
            None if !setup.agents.is_empty() => setup::run(&setup),
            None => init(force),
        },
        JevCommand::Mcp => mcp::run().map(|()| 0),
        JevCommand::Hook(args) => hook::run(&args),
        JevCommand::Check(args) => check(*args),
        JevCommand::Rules {
            action: Some(options::RulesAction::Add { names, force }),
            ..
        } => crate::custom::gallery::run(&repository()?, &names, force),
        command => configured(command),
    }
}

/// The repository around the working directory, for the commands that run
/// before its configuration is read: `init`, so an invalid file can be
/// replaced, and `rules add`, so a question file that no longer loads can.
fn repository() -> Result<std::path::PathBuf> {
    Ok(config::repository_root(
        &std::env::current_dir()?.canonicalize()?,
    ))
}

fn init(force: bool) -> Result<u8> {
    let root = repository()?;
    let (path, allow) = init::run(&root, force)?;
    say!("Wrote {}", path.display());
    if allow.is_empty() {
        say!("No supported source found; set upload_allow before checking.");
    } else {
        say!("Uploads limited to: {}", allow.join(", "));
    }
    say!("Next: jevgate auth login, then jevgate check --dry-run --show-requests");
    Ok(0)
}

/// `check`. When a run that cannot finish passes (`--on-incomplete`, and
/// by default for `--staged` and `--pre-push`), so does one that fails
/// before it can judge anything, such as on a configuration that does not
/// load, saying loudly that the change was not checked.
fn check(mut args: CheckArgs) -> Result<u8> {
    crate::check::validate(&args)?;
    // One offer for the whole invocation, which checks each pushed ref.
    let _skip = args
        .moment()
        .filter(|_| !args.dry_run)
        .and_then(git_hooks::offer_skip);
    let result = (|| {
        let context =
            ConfigContext::discover(args.config.as_deref(), args.question_directory.as_deref())?;
        crate::check::configure(&mut args, &context)?;
        if args.pre_push {
            return crate::check::pre_push(&mut args, &context);
        }
        crate::check::resolve_change(&mut args, &context.root)?;
        crate::check::run(&args, &context)
    })();
    match result {
        Err(error) if args.passes_incomplete() && cancellation::signal().is_none() => {
            git_hooks::not_checked(&args, &format!("{error:#}"));
            Ok(0)
        }
        other => other,
    }
}

/// The commands that read the repository's configuration.
fn configured(command: JevCommand) -> Result<u8> {
    let (file, questions) = match &command {
        JevCommand::Rules {
            action: Some(options::RulesAction::Test(args)),
            ..
        } => (args.config.clone(), args.question_directory.clone()),
        _ => (None, None),
    };
    let context = ConfigContext::discover(file.as_deref(), questions.as_deref())?;
    match command {
        JevCommand::Auth { .. }
        | JevCommand::Init { .. }
        | JevCommand::Completions { .. }
        | JevCommand::Man { .. }
        | JevCommand::Mcp
        | JevCommand::Hook(_)
        | JevCommand::Check(_)
        | JevCommand::Rules {
            action: Some(options::RulesAction::Add { .. }),
            ..
        } => {
            unreachable!("handled before repository configuration")
        }
        JevCommand::Baseline {
            action: Some(action),
            ..
        } => baseline_action(&context, action),
        JevCommand::Baseline {
            merge,
            reason,
            action: None,
        } => accept(&context, merge, reason),
        JevCommand::Rules {
            action: Some(options::RulesAction::Test(args)),
            ..
        } => crate::rules_test::run(&args, &context),
        JevCommand::Rules {
            action: Some(options::RulesAction::Propose(args)),
            ..
        } => crate::custom::propose::run(&args, &context),
        JevCommand::Rules {
            action: Some(options::RulesAction::Accept { ids }),
            ..
        } => crate::custom::propose::accept(&ids, &context),
        JevCommand::Rules {
            format,
            action: None,
        } => rules(&context, format),
        JevCommand::Serve { port } => {
            cancellation::install()?;
            server::run(&context.root, port)?;
            Ok(0)
        }
    }
}

/// `rules`: every rule and custom question, as a table or JSON.
fn rules(context: &ConfigContext, format: options::RulesFormat) -> Result<u8> {
    match format {
        options::RulesFormat::Json => say!(
            "{}",
            serde_json::to_string_pretty(&catalog::describe(context.questions))?
        ),
        options::RulesFormat::Table => say!("{}", catalog::table(context.questions)),
    }
    Ok(0)
}

/// The rule keys `--rule` names select: each a rule ID, name, key or group.
fn rule_keys(context: &ConfigContext, names: &[String]) -> Result<Vec<&'static str>> {
    let known = context.rules();
    let mut keys = Vec::new();
    for name in names {
        keys.extend(catalog::select_in(&known, name).ok_or_else(|| {
            anyhow::anyhow!("Unknown rule or group: {name}; `jevgate rules` lists the rules")
        })?);
    }
    Ok(keys)
}

/// `baseline`: accept the last check's findings.
fn accept(
    context: &ConfigContext,
    merge: bool,
    reason: Option<options::Disposition>,
) -> Result<u8> {
    let written = baseline::write(&context.root, merge, reason)?;
    let path = written.path.display();
    if merge {
        say!(
            "Accepted {} from the last check in {path}; kept {} for files it did not cover",
            output::count(written.accepted, "finding"),
            output::count(written.kept, "earlier finding")
        );
    } else {
        say!(
            "Accepted {} in {path}",
            output::count(written.accepted, "finding")
        );
    }
    Ok(0)
}

/// `baseline mark`, `baseline list` and `baseline stats`: offline edits,
/// listings and counts of the baseline.
fn baseline_action(context: &ConfigContext, action: options::BaselineAction) -> Result<u8> {
    match action {
        options::BaselineAction::Mark {
            reason,
            targets,
            rules,
            note,
        } => {
            let note = note.as_deref().map(baseline::note).transpose()?;
            let keys = rule_keys(context, &rules)?;
            let marked = baseline::mark(
                &context.root,
                &baseline::Mark {
                    reason,
                    note,
                    targets: &targets,
                    rules: &keys,
                },
            )?;
            say!(
                "Marked {} as {}",
                output::count(marked, "finding"),
                output::label(&reason)
            );
        }
        options::BaselineAction::List {
            reasons,
            rules,
            format,
        } => {
            let keys = rule_keys(context, &rules)?;
            let listed = baseline::list(&context.root, &reasons, &keys)?;
            match format {
                options::ListFormat::Json => say!("{}", serde_json::to_string_pretty(&listed)?),
                _ if listed.is_empty() => say!("No accepted findings match."),
                options::ListFormat::Text => say!("{}", baseline::list_text(&listed)),
                options::ListFormat::Md => say!("{}", baseline::list_markdown(&listed)),
            }
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
