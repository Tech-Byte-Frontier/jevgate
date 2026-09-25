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
mod check;
mod command;
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
mod manual;
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

use clap::Parser;
use options::JevCommand;

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
pub struct Cli {
    #[command(subcommand)]
    command: JevCommand,
}

fn main() -> std::process::ExitCode {
    let result = command::run(Cli::parse().command);
    let code = match result {
        Ok(code) => code,
        Err(error) => {
            note!("jevgate: {error:#}");
            2
        }
    };
    std::process::ExitCode::from(cancellation::signal().map_or(code, |s| (128 + s) as u8))
}

#[cfg(test)]
mod tests;
