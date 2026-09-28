//! The rules actions: `rules test` (which custom questions to ask about
//! their examples, and how), `rules propose` (which instruction files to
//! read, and how to print what is proposed), `rules accept` (which
//! proposals to accept) and `rules add` (which gallery questions to add).
use super::RulesFormat;
use clap::{
    Args, Subcommand, ValueEnum,
    builder::{PossibleValue, PossibleValuesParser},
};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum RulesAction {
    /// Ask custom questions about their examples; exit 1 when one gets an example wrong, 2 when incomplete
    ///
    /// A question's `failing` examples are code that breaks its rule: the
    /// answer about each must reach the question's threshold, so a check would
    /// find it. Its `passing` examples are code that keeps the rule: the answer
    /// must stay below the threshold. Each example is asked as a check asks the
    /// same units, and answers are cached like a check's: a rerun is free, and
    /// a new model or a reworded question asks again, so a question that stops
    /// separating its examples fails here before it misleads a check.
    #[command(after_long_help = RULES_TEST_EXAMPLES)]
    Test(RulesTestArgs),
    /// Propose custom questions from the lines of AGENTS.md and the other agent instruction files
    ///
    /// Splits each instruction file into lines and list items and asks
    /// TypeSafe Jev of each whether it states a rule for how the code is
    /// written that one piece of it shows, and which piece: a function, test,
    /// comment, documentation section, file or change; then, of each rule,
    /// whether a reviewer checks it or a formatter, linter or script already
    /// does. Each rule a reviewer checks becomes a custom question that quotes
    /// the line and cites its file and line, written to `.jevgate/proposals/`
    /// (which Git ignores) as a note. Read one, edit it, then accept it with
    /// `jevgate rules accept ID`; nothing reaches the configuration otherwise.
    /// Answers are cached, so a rerun pays only for changed lines, and it
    /// never overwrites a proposal or proposes a line that is already a
    /// question.
    #[command(after_long_help = PROPOSE_EXAMPLES)]
    Propose(ProposeArgs),
    /// Accept proposed questions: check each one and move it to .jevgate/questions/
    ///
    /// Each ID names `.jevgate/proposals/ID.toml`, which must be a valid
    /// question file whose id no other question uses. Commit the moved file. A
    /// proposal is a note, which never fails the gate, until its `level` says
    /// otherwise.
    Accept {
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,
    },
    /// Add measured questions from JevGate's question gallery to .jevgate/questions/
    ///
    /// Each NAME is a custom question JevGate measured on real projects: the
    /// docs' question gallery page gives how often each was right. It
    /// is written to `.jevgate/questions/NAME.toml`, a file the project owns:
    /// adapt its guidance and paths to the code, and commit it. Like any
    /// custom question, it fails the gate at its own level.
    #[command(after_long_help = RULES_ADD_EXAMPLES)]
    Add {
        /// Gallery questions to add
        #[arg(required = true, value_name = "NAME", value_parser = gallery())]
        names: Vec<String>,
        /// Replace question files of the same names, edits included
        #[arg(long)]
        force: bool,
    },
}

/// The arguments of `rules test`: which questions, and how to ask.
#[derive(Args, Debug)]
pub struct RulesTestArgs {
    /// A custom question, `custom/<id>`, or the group `custom` (repeatable) [default: every question with examples]
    #[arg(long = "rule", value_name = "RULE")]
    pub rules: Vec<String>,
    /// `table` for people; `json` for scripts, with every unit's probability
    #[arg(long, value_enum, default_value_t = RulesFormat::Table)]
    pub format: RulesFormat,
    /// Count the requests the examples need and those the cache answers, without credentials, network or writes
    ///
    /// Every example is still read and its units found, so an example that
    /// cannot be asked exits 2 here too.
    #[arg(long)]
    pub dry_run: bool,
    /// Model, as the key's provider names it [default: `model` in jevgate.toml, else the key's provider's model]
    ///
    /// Answers are cached per model, so another model asks every example
    /// again: try the examples on a model before pinning it.
    #[arg(long)]
    pub model: Option<String>,
    /// Ignore cached answers for this invocation and ask again
    #[arg(long)]
    pub refresh: bool,
    /// Use cached answers only and never contact the provider; an example without one leaves the run incomplete
    #[arg(long, conflicts_with = "refresh")]
    pub cache_only: bool,
    /// Stop after this many API attempts; reaching it leaves the run incomplete
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=1000000))]
    pub max_requests: Option<u32>,
    /// Credential file holding TYPESAFE_API_KEY, OPENROUTER_API_KEY or AI_GATEWAY_API_KEY [default: <repository root>/.env]
    ///
    /// Read as `check` reads it: TYPESAFE_API_KEY in the environment takes
    /// precedence, and the repository's .env is read only for
    /// TYPESAFE_API_KEY.
    #[arg(long, value_name = "FILE")]
    pub env_file: Option<PathBuf>,
    /// Read this configuration instead of <repository root>/jevgate.toml
    ///
    /// As for `check`, the question files of .jevgate/questions/ are then not
    /// read; --questions reads a reviewed copy.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Read custom question files from this directory instead of .jevgate/questions/
    #[arg(long = "questions", value_name = "DIR")]
    pub question_directory: Option<PathBuf>,
}

const RULES_TEST_EXAMPLES: &str = "\
Examples:
  jevgate rules test --dry-run                      Requests and new tokens, offline; every example checked
  jevgate rules test                                Exit 1 when a question gets an example wrong
  jevgate rules test --model jev-latest             Try the examples on another model before pinning it
  jevgate rules test --format json                  Every unit's probability, for scripts";

#[derive(Args, Debug)]
pub struct ProposeArgs {
    /// Instruction files or directories to read [default: every instruction file an agent loads]
    ///
    /// A directory selects the agent instruction files under it. A file is
    /// read whatever its name, such as CONTRIBUTING.md. Translations under a
    /// locale directory (`docs/i18n/ja/CLAUDE.md`) are read only when named.
    pub paths: Vec<PathBuf>,
    /// Output format [default: table; json with --show-requests]
    #[arg(long, value_enum)]
    pub format: Option<ProposeFormat>,
    /// List the files, lines and planned requests without credentials, network or writes
    ///
    /// Requests the cache already answers are counted apart and cost nothing.
    /// What checks each rule is asked only after the answers, so it is not
    /// counted.
    #[arg(long)]
    pub dry_run: bool,
    /// With --dry-run, include every request body (the exact lines and questions)
    #[arg(long, requires = "dry_run")]
    pub show_requests: bool,
    /// Credential file holding TYPESAFE_API_KEY, OPENROUTER_API_KEY or AI_GATEWAY_API_KEY [default: <repository root>/.env]
    ///
    /// Read as `check` reads it: TYPESAFE_API_KEY in the environment takes
    /// precedence, and the repository's .env is read only for
    /// TYPESAFE_API_KEY.
    #[arg(long, value_name = "FILE")]
    pub env_file: Option<PathBuf>,
}

impl ProposeArgs {
    pub fn output_format(&self) -> ProposeFormat {
        self.format.unwrap_or(if self.show_requests {
            ProposeFormat::Json
        } else {
            ProposeFormat::Table
        })
    }
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum ProposeFormat {
    /// Write the proposals to .jevgate/proposals/ and list them
    Table,
    /// Print the proposals as [[question]] tables for jevgate.toml; write nothing
    Toml,
    /// Print every line with its answers and proposal; write nothing
    Json,
}

const PROPOSE_EXAMPLES: &str = "\
Examples:
  jevgate rules propose --dry-run                 Files, lines and price; no key, no network
  jevgate rules propose                           Write proposals to .jevgate/proposals/
  jevgate rules propose AGENTS.md docs/STYLE.md   Only these files
  jevgate rules propose --format toml             Print [[question]] tables for jevgate.toml
  jevgate rules propose --format json             Every line with Jev's answers
  jevgate rules accept never-log-request-bodies   Accept one after editing it";

/// The gallery's names, each with what it catches, for help and completions.
fn gallery() -> PossibleValuesParser {
    PossibleValuesParser::new(
        crate::custom::gallery::ENTRIES
            .iter()
            .map(|entry| PossibleValue::new(entry.name).help(entry.summary())),
    )
}

const RULES_ADD_EXAMPLES: &str = "\
Examples:
  jevgate rules add swallowed-errors resource-leak   Two questions, as .jevgate/questions/*.toml
  jevgate check --rule custom --dry-run              What they would ask, offline
  jevgate check --fail-on custom=report              Ask them without failing the gate while you try them
  jevgate rules add --force n-plus-one               Restore the gallery's wording of one";
