//! The baseline actions: `baseline mark` (which accepted findings, and why
//! they were accepted) and `baseline stats` (how to print the counts).
use super::RulesFormat;
use clap::{Subcommand, ValueEnum};

/// Why a finding was accepted into the baseline.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Disposition {
    /// The finding is right; the code is meant to be this way
    Intended,
    /// The finding is right; it will be fixed later
    Later,
    /// The finding is mistaken
    Wrong,
}

#[derive(Subcommand)]
pub enum BaselineAction {
    /// Record why accepted findings were accepted
    ///
    /// Each target is a path or directory as the check output prints it, a
    /// `PATH:LINE`, or a fingerprint (at least its first 8 characters) from
    /// the JSON report. `--rule` narrows the match to rules or groups.
    Mark {
        /// intended, later or wrong
        #[arg(value_enum)]
        reason: Disposition,
        #[arg(required = true, value_name = "TARGET")]
        targets: Vec<String>,
        /// Only findings of this rule ID, name, key or group (repeatable)
        #[arg(long = "rule", value_name = "RULE")]
        rules: Vec<String>,
    },
    /// Count accepted findings by rule and reason, with each rule's rate of wrong findings
    ///
    /// The rate is `wrong` among the findings that have a reason; findings
    /// without one are counted apart. These are labels people gave in daily
    /// use, the accuracy evidence a model's probabilities are not.
    Stats {
        /// `table` for people; `json` for scripts
        #[arg(long, value_enum, default_value_t = RulesFormat::Table)]
        format: RulesFormat,
    },
}
