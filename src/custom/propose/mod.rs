//! `jevgate rules propose`: custom questions proposed from the lines of the
//! project's agent instruction files. Code finds the files and splits them
//! into candidate lines (`files`, `lines`); Jev answers of each line whether
//! it states a rule one piece of the code shows, and which piece (`ask`);
//! each line it calls a rule becomes a question that quotes it
//! (`proposal`), written for a person to edit and accept (`saved`,
//! `accept`). Jev classifies; it writes nothing.
mod accept;
mod ask;
mod files;
mod lines;
mod proposal;
mod render;
mod saved;
#[cfg(test)]
mod tests;

pub use accept::accept;

use crate::{
    config::ConfigContext,
    options::{CheckArgs, ProposeArgs, ProposeFormat},
    storage::Store,
    transport::Evaluator,
};
use anyhow::{Result, ensure};

/// Proposals, one question file each, relative to the repository root.
/// `.jevgate/.gitignore` keeps them out of Git.
pub const PROPOSALS: &str = ".jevgate/proposals";

/// What a run prints: its output, the notes for stderr, and its exit code.
pub struct Printed {
    pub stdout: String,
    pub notes: Vec<String>,
    pub code: u8,
}

pub fn run(args: &ProposeArgs, context: &ConfigContext) -> Result<u8> {
    let check = settings(args, context)?;
    let mut client = crate::transport::Client::new(
        &crate::check::credential_path(&check, context),
        args.env_file.is_some(),
        check.provider,
    )?;
    let printed = propose(args, context, (&check, &mut client))?;
    if !printed.stdout.is_empty() {
        say!("{}", printed.stdout);
    }
    for line in &printed.notes {
        note!("jevgate: {line}");
    }
    Ok(printed.code)
}

/// The session's settings: `check`'s defaults under the repository's
/// configuration (model, request budget, concurrency, file size), with this
/// command's credential file.
fn settings(args: &ProposeArgs, context: &ConfigContext) -> Result<CheckArgs> {
    ensure!(
        !args.show_requests || args.output_format() == ProposeFormat::Json,
        "--show-requests uses JSON output; omit --format or use --format json"
    );
    let mut check = CheckArgs::defaults();
    context.configure(&mut check)?;
    check.env_file.clone_from(&args.env_file);
    check.dry_run = args.dry_run;
    check.provider = crate::check::planned_provider(&check, context);
    Ok(check)
}

/// Read the files, ask about their lines, and write or print the proposals.
fn propose(
    args: &ProposeArgs,
    context: &ConfigContext,
    (check, evaluator): (&CheckArgs, &mut dyn Evaluator),
) -> Result<Printed> {
    let (files, skipped) = files::read(&args.paths, context, check.max_file_bytes)?;
    let plan = ask::plan(&files, check.model());
    let format = args.output_format();
    if args.dry_run {
        let price = ask::price(&plan, &context.root, check);
        let stdout = match format {
            ProposeFormat::Json => {
                render::dry_run_json(&files, &skipped, (&plan, &price), args.show_requests)?
            }
            _ => render::dry_run(&files, &skipped, (&plan, &price), check.model()),
        };
        return Ok(Printed {
            stdout,
            notes: Vec::new(),
            code: 0,
        });
    }
    crate::cancellation::install()?;
    let store = Store::open(&context.root)?;
    let mut session = crate::check::session(check, context, &store, evaluator);
    let answers = ask::ask(&plan, &mut session);
    let usage = render::Usage {
        requests: session.requests,
        tokens: session.paid.input_tokens,
        usd: session.paid.usd(),
    };
    let mut saved = saved::Saved::load(&context.root, context.questions);
    let mut candidates = proposal::decide(&files, &plan, &answers, &mut saved);
    let errors = answers.errors();
    let code = if errors.is_empty() { 0 } else { 2 };
    let (stdout, notes) = match format {
        ProposeFormat::Table => {
            saved::write(&context.root, &mut candidates)?;
            let table = render::table(&files, &skipped, &candidates, (&usage, &errors));
            (table, Vec::new())
        }
        ProposeFormat::Toml => (render::toml(&candidates)?, errors),
        ProposeFormat::Json => {
            let json = render::json(&files, &skipped, &candidates, (&usage, &errors))?;
            (json, Vec::new())
        }
    };
    Ok(Printed {
        stdout,
        notes,
        code,
    })
}
