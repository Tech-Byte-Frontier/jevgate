//! Running each command: offline commands and the agent hook first (the hook
//! finds its repository from the event, and agent setup writes the agents'
//! files), then the ones that read the repository's configuration; `check`
//! runs in its own module.
use crate::{
    auth, baseline, cancellation, catalog, config,
    config::ConfigContext,
    hook, init, manual, mcp,
    options::{self, JevCommand},
    output, revision, server, setup,
};
use anyhow::Result;

pub fn run(command: JevCommand) -> Result<u8> {
    match command {
        JevCommand::Auth { command } => auth::run(command),
        JevCommand::Completions { shell } => manual::completions(shell).map(|()| 0),
        JevCommand::Man { command } => manual::man(command.as_deref()).map(|()| 0),
        JevCommand::Init { setup, .. } if !setup.agents.is_empty() => setup::run(&setup),
        JevCommand::Init { force, .. } => init(force),
        JevCommand::Mcp => mcp::run().map(|()| 0),
        JevCommand::Hook(args) => hook::run(&args),
        command => configured(command),
    }
}

/// `init` runs before configuration is read, so an invalid file can be replaced.
fn init(force: bool) -> Result<u8> {
    let root = config::repository_root(&std::env::current_dir()?.canonicalize()?);
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

/// The commands that read the repository's configuration.
fn configured(command: JevCommand) -> Result<u8> {
    let file = match &command {
        JevCommand::Check(args) => args.config.clone(),
        JevCommand::Rules {
            action: Some(options::RulesAction::Test(args)),
            ..
        } => args.config.clone(),
        _ => None,
    };
    let context = ConfigContext::discover(file.as_deref())?;
    match command {
        JevCommand::Auth { .. }
        | JevCommand::Init { .. }
        | JevCommand::Completions { .. }
        | JevCommand::Man { .. }
        | JevCommand::Mcp
        | JevCommand::Hook(_) => {
            unreachable!("handled before repository configuration")
        }
        JevCommand::Check(mut args) => {
            crate::check::configure(&mut args, &context)?;
            if let Some(base) = &args.base {
                args.base = Some(revision::resolve(&context.root, base)?);
            }
            crate::check::run(&args, &context)
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
        } => {
            match format {
                options::RulesFormat::Json => say!(
                    "{}",
                    serde_json::to_string_pretty(&catalog::describe(context.questions))?
                ),
                options::RulesFormat::Table => say!("{}", catalog::table(context.questions)),
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

/// `baseline mark` and `baseline stats`: offline edits and counts of the baseline.
fn baseline_action(context: &ConfigContext, action: options::BaselineAction) -> Result<u8> {
    match action {
        options::BaselineAction::Mark {
            reason,
            targets,
            rules,
        } => {
            let known = context.rules();
            let mut keys = Vec::new();
            for name in &rules {
                keys.extend(catalog::select_in(&known, name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown rule or group: {name}; `jevgate rules` lists the rules"
                    )
                })?);
            }
            let marked = baseline::mark(&context.root, reason, &targets, &keys)?;
            say!(
                "Marked {} as {}",
                output::count(marked, "accepted finding"),
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
