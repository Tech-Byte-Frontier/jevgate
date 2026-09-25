//! The subcommands, the baseline actions and their help text.
use super::CheckArgs;
use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub enum JevCommand {
    /// Save, inspect or remove your TypeSafe API credential
    ///
    /// TypeSafe credentials are checked in this order: the TYPESAFE_API_KEY
    /// environment variable, then the file named by `check --env-file` (by
    /// default the repository's `.env`), then the key saved by `jevgate auth login`.
    #[command(after_long_help = AUTH_EXAMPLES)]
    Auth {
        #[command(subcommand)]
        command: crate::auth::AuthCommand,
    },
    /// Review code with Jev; TypeSafe is the default provider
    ///
    /// Exits 1 when the gate fails and 2 when the run is incomplete
    ///
    /// Parses the selected files locally, sends small evidence units (a
    /// function, a file outline, a pair of copies, a test, a documentation
    /// section) with short questions, and composes the answers into findings.
    /// Unchanged units are answered from `.jevgate/cache`, so a re-run only pays
    /// for what changed. Every run writes the full report to
    /// `.jevgate/latest.json`, whatever the output format.
    ///
    /// Findings are `review` (act on it), `consider` (worth a look) or `note`
    /// (optional; never fails the gate). A file whose answers stay undecided is
    /// `uncertain`; one that cannot be judged without more evidence is
    /// `needs-context`.
    ///
    /// Settings resolve in this order: flags, then `jevgate.toml`, then
    /// defaults. Upload patterns and budgets in the file are ceilings that
    /// flags can only narrow.
    #[command(after_long_help = CHECK_EXAMPLES)]
    Check(Box<CheckArgs>),
    /// Accept the findings of the last complete check, so later checks fail only on new ones
    ///
    /// Writes `jevgate-baseline.json` at the repository root from
    /// `.jevgate/latest.json`. Commit the file. Findings are matched by a
    /// fingerprint of rule, path, unit and evidence, so unrelated edits keep
    /// them accepted. Offline: no source is read or sent.
    ///
    /// Each accepted finding can record why it was accepted: `intended` (right
    /// about the code, which is meant to be this way), `later` (right, to fix
    /// later) or `wrong` (the finding is mistaken). `baseline stats` turns
    /// these reasons into each rule's rate of wrong findings.
    #[command(args_conflicts_with_subcommands = true, after_long_help = BASELINE_EXAMPLES)]
    Baseline {
        /// Keep earlier accepted findings for files the last check did not cover
        ///
        /// Without it, the file is replaced, so after a `--base` or path-limited
        /// check the findings accepted for every other file are dropped. With it,
        /// entries for files the check covered, or that were deleted, are replaced
        /// by what the check found, and the rest are kept.
        #[arg(long)]
        merge: bool,
        /// Record this reason on findings accepted now without one
        ///
        /// Findings already accepted keep the reason they have.
        #[arg(long, value_enum)]
        reason: Option<Disposition>,
        #[command(subcommand)]
        action: Option<BaselineAction>,
    },
    /// List every rule with its group, default and the question it asks
    ///
    /// A rule is named by its ID (`maintainability/shared-logic`), its key
    /// (`shared_logic`) or its group (`maintainability`, `tests`, `security`,
    /// `documentation`, plus `default` and `all`) anywhere a rule is accepted:
    /// `--rule`, `--skip-rule`, `--fail-on TARGET=LEVEL` and `[rules]`.
    Rules {
        /// `table` for people; `json` adds scope, evidence unit, version and decision policy
        #[arg(long, value_enum, default_value_t = RulesFormat::Table)]
        format: RulesFormat,
    },
    /// Write a commented jevgate.toml for this repository (offline)
    ///
    /// Limits uploads to the detected source and test directories and to agent
    /// instruction files, denies credential files, and lists every rule group
    /// with its gate level. Review the file before the first paid check.
    Init {
        /// Replace an existing jevgate.toml
        #[arg(long)]
        force: bool,
    },
    /// Serve the latest report as read-only JSON on localhost (run alongside `check --watch`)
    ///
    /// Answers GET requests from local tools, never from a browser page:
    /// `/snapshot` (the full report), `/evidence` (findings and context per
    /// file), `/context-requests` (evidence a file still needs) and
    /// `/changes?since=GENERATION` (what changed since a report generation).
    Serve {
        /// Local port to listen on
        #[arg(long, default_value_t = 47831)]
        port: u16,
    },
}

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
        /// Only findings of this rule ID, key or group (repeatable)
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

const BASELINE_EXAMPLES: &str = "\
Examples:
  jevgate baseline                                  Accept every finding of the last check
  jevgate baseline --merge --reason later           Accept a partial check's findings as known debt
  jevgate baseline mark wrong src/api/search.ts:41  A mistaken finding
  jevgate baseline mark intended scripts --rule maintainability/hardcoded-values
  jevgate baseline stats                            Wrong findings per rule";

/// Overview, workflow, exit codes and files, shown by `jevgate --help`.
pub const OVERVIEW: &str = "\
Workflow:
  jevgate init                              Write jevgate.toml: upload scope, rules and gate
  jevgate auth login                        Save an API key (or set TYPESAFE_API_KEY)
  jevgate check --dry-run --show-requests   Print every request body; no key, no network
  jevgate check                             Review and apply the gate
  jevgate baseline                          Accept current findings; later checks fail only on new ones
  jevgate baseline --merge                  Accept a partial check's findings, keeping the rest
  jevgate baseline mark wrong PATH[:LINE]   Record why a finding was accepted; `baseline stats` counts them

For agents and CI:
  jevgate check --base origin/main                   Only files changed since a revision
  jevgate check --base origin/main --format json     The full report, raw probabilities included
  jevgate check --base origin/main --format github   Annotations and a job summary on GitHub
  jevgate rules --format json                        Every rule and the question it asks

Exit codes:
  0      Gate passed, or no supported file changed since --base
  1      Gate failed
  2      Run incomplete (no key, provider rejection, request budget reached), invalid
         configuration or invalid usage
  128+N  Interrupted by signal N

Files (at the repository root):
  jevgate.toml            Configuration; `jevgate init` writes a commented one
  jevgate-baseline.json   Accepted findings; commit it
  .jevgate/cache/         Answers by request hash; safe to restore and save in CI
  .jevgate/latest.json    The last report, the same JSON as --format json
  .jevgate/report.html    HTML dashboard, with --report

Environment:
  TYPESAFE_API_KEY          TypeSafe API key; takes precedence over saved credentials
  OPENROUTER_API_KEY        OpenRouter key when --provider openrouter is selected
  JEVGATE_CREDENTIAL_STORE  Where `auth login` saves: auto, keyring or file
  JEVGATE_CONFIG_DIR        Absolute directory for file-stored credentials
  CI                        When set, --report writes the dashboard without opening a browser

`jevgate <command> --help` explains each command; -h prints a summary.";

const CHECK_EXAMPLES: &str = "\
Examples:
  jevgate check                                    Discovered application source, default rules
  jevgate check src/billing --verbose              One directory, with notes and per-file detail
  jevgate check --base origin/main --format json   Changed files only, machine-readable
  jevgate check --rule default --rule security     Add the opt-in security group
  jevgate check --rule documentation               Only agent instruction files and project docs
  jevgate check --include-tests                    Also judge test value and redundancy
  jevgate check --fail-on none                     Advisory: never exits 1; exits 2 when incomplete
  jevgate check --fail-on review --fail-on security=consider
  jevgate check --dry-run --show-requests          Exactly what would be uploaded, offline
  jevgate check --provider openrouter              Use OpenRouter's Jev route
  jevgate check --cache-only                       Replay cached answers; never contact the provider

Reading the JSON report (--format json or .jevgate/latest.json):
  complete           false when any selected file was not judged; the exit code is then 2
  gate               passed, reasons, new_findings, baselined_findings
  files[].status     clear, note, consider, review, uncertain, needs-context,
                     not-applicable, skipped or error
  files[].findings   rule, strength, line, message, action, locations,
                     concern_probability, fingerprint, baselined
  files[].dimensions per rule: status, unit counts and the units left undecided
  files[].judgments  every raw answer, first pass and follow-ups
  api_requests, paid_input_tokens, paid_output_tokens   this run's usage";

const AUTH_EXAMPLES: &str = "\
Examples:
  jevgate auth login                               Hidden prompt; saved in the OS credential store
  jevgate auth login --with-key < key.txt          Read the key from stdin
  jevgate auth status                              Show which key a check would use and verify it
  jevgate auth status --offline --json             Same, without contacting TypeSafe
  jevgate auth logout";

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum RulesFormat {
    Table,
    Json,
}
