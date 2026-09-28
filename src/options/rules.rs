//! `rules test`: which custom questions to ask about their examples, and
//! how to ask them.
use super::RulesFormat;
use clap::{Args, Subcommand};
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
    /// TypeSafe model [default: `model` in jevgate.toml, else jev-1.13.0]
    ///
    /// Answers are cached per model, so another model asks every example
    /// again: try the examples on a model before pinning it.
    #[arg(long)]
    pub model: Option<String>,
    /// Ignore cached answers for this invocation and ask again
    #[arg(long)]
    pub refresh: bool,
    /// Use cached answers only and never contact TypeSafe; an example without one leaves the run incomplete
    #[arg(long, conflicts_with = "refresh")]
    pub cache_only: bool,
    /// Stop after this many API attempts; reaching it leaves the run incomplete
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=1000000))]
    pub max_requests: Option<u32>,
    /// Credential file holding TYPESAFE_API_KEY [default: <repository root>/.env]
    #[arg(long, value_name = "FILE")]
    pub env_file: Option<PathBuf>,
    /// Read this configuration instead of <repository root>/jevgate.toml
    ///
    /// As for `check`, the question files of .jevgate/questions/ are then not read.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

const RULES_TEST_EXAMPLES: &str = "\
Examples:
  jevgate rules test --dry-run                      Requests and new tokens, offline; every example checked
  jevgate rules test                                Exit 1 when a question gets an example wrong
  jevgate rules test --model jev-latest             Try the examples on another model before pinning it
  jevgate rules test --format json                  Every unit's probability, for scripts";
