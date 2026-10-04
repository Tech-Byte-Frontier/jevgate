//! The baseline actions: `baseline mark` (which findings to dismiss or
//! accepted findings to mark, and why), `baseline list` (which marks to
//! print, and how) and `baseline stats` (how to print the counts).
use super::RulesFormat;
use clap::{Subcommand, ValueEnum};

/// Why a finding was dismissed or accepted into the baseline.
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
    /// Dismiss findings with a reason, or record why accepted ones were accepted
    ///
    /// Each target is a path or directory as the check output prints it, a
    /// `PATH:LINE`, or a fingerprint (at least its first 8 characters) from
    /// the JSON report. A finding of the last check that the baseline does
    /// not hold yet is accepted with the reason when a `PATH:LINE` or
    /// fingerprint names it: a coding agent dismisses what its hook reported
    /// this way, and `baseline stats` counts the reasons for a person to
    /// audit. A path or directory marks only findings already accepted.
    /// `--rule` narrows the match to rules or groups.
    Mark {
        /// intended, later or wrong
        #[arg(value_enum)]
        reason: Disposition,
        #[arg(required = true, value_name = "TARGET")]
        targets: Vec<String>,
        /// Only findings of this rule ID, name, key or group (repeatable)
        #[arg(long = "rule", value_name = "RULE")]
        rules: Vec<String>,
        /// A short note to keep with the reason, such as the issue that will fix a `later` finding
        ///
        /// One line of at most 200 characters. Without it, a finding marked
        /// again keeps its note; an empty note removes it.
        #[arg(long, value_name = "TEXT")]
        note: Option<String>,
    },
    /// List accepted findings with their reason, rule, location, unit, fingerprint and note
    ///
    /// Ordered by path, then line. `--format md` prints a checklist to paste
    /// into a cleanup issue.
    List {
        /// Only findings marked with this reason (repeatable)
        #[arg(long = "reason", value_enum, value_name = "REASON")]
        reasons: Vec<Disposition>,
        /// Only findings of this rule ID, name, key or group (repeatable)
        #[arg(long = "rule", value_name = "RULE")]
        rules: Vec<String>,
        /// `text` for people; `md` for a Markdown checklist; `json` for scripts
        #[arg(long, value_enum, default_value_t = ListFormat::Text)]
        format: ListFormat,
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

/// How `baseline list` prints the findings.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum ListFormat {
    /// Aligned columns
    Text,
    /// A Markdown checklist
    Md,
    /// A JSON array
    Json,
}
