//! Shell completion scripts and man pages, generated from the command-line
//! definition so they never drift from `--help`.
use anyhow::{Context, Result};
use clap::CommandFactory;
use std::io::Write;

pub fn completions(shell: clap_complete::Shell) -> Result<()> {
    let mut command = crate::Cli::command();
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut command, "jevgate", &mut out);
    std::io::stdout().write_all(&out)?;
    Ok(())
}

/// The page for `jevgate`, or for one of its commands as `jevgate-NAME`.
pub fn man(name: Option<&str>) -> Result<()> {
    let mut root = crate::Cli::command();
    root.build();
    let page = match name {
        None => root,
        Some(name) => {
            let command = root
                .find_subcommand(name)
                .filter(|c| c.get_name() != "help")
                .with_context(|| format!("Unknown command: {name}"))?;
            let title = format!("jevgate-{}", command.get_name());
            command
                .clone()
                .name(title)
                .version(env!("CARGO_PKG_VERSION"))
        }
    };
    let mut out = Vec::new();
    clap_mangen::Man::new(page).render(&mut out)?;
    std::io::stdout().write_all(&out)?;
    Ok(())
}
