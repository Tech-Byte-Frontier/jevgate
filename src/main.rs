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
#[cfg(test)]
mod config_schema;
mod context;
mod context_units;
mod discovery;
mod docs;
mod evaluate;
mod file_kind;
mod gate;
mod github;
mod gitlab;
mod guards;
mod hook;
mod html_report;
mod init;
mod inventory;
mod line_ranges;
mod locations;
mod manual;
mod maturity;
mod mcp;
mod model;
mod options;
mod output;
mod packages;
mod policy;
mod provider;
mod provider_error;
mod requests;
mod response;
mod response_headers;
mod revision;
mod sarif;
mod schema;
mod server;
mod setup;
mod storage;
mod suppress;
mod syntax;
mod test_locations;
mod token_budget;
mod transport;
mod units;
mod view;
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
/// Rule groups: maintainability (on by default, except hardcoded values),
/// tests (with --include-tests), and the opt-in security and documentation
/// groups. By default only rules and levels measured right at least 80% of
/// the time on projects JevGate was never tuned on fail the check.
#[derive(Parser)]
#[command(version, after_long_help = options::OVERVIEW)]
pub struct Cli {
    #[command(subcommand)]
    command: JevCommand,
}

fn main() -> std::process::ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // An agent reads exit 2 as a block: the hook answers even this.
        Err(error) if error.use_stderr() && hook::invoked() => return hook::usage_error(&error),
        Err(error) => error.exit(),
    };
    let result = command::run(cli.command);
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
