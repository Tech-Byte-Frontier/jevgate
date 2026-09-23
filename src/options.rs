use clap::{Args, Subcommand, ValueEnum};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Subcommand)]
pub enum JevCommand {
    /// Save, inspect or remove your TypeSafe API credential
    ///
    /// A check finds its key in this order: the TYPESAFE_API_KEY environment
    /// variable, then the file named by `check --env-file` (by default the
    /// repository's `.env`), then the key saved by `jevgate auth login`. In CI, set TYPESAFE_API_KEY
    /// from a secret; nothing needs to be saved.
    #[command(after_long_help = AUTH_EXAMPLES)]
    Auth {
        #[command(subcommand)]
        command: crate::auth::AuthCommand,
    },
    /// Review code with TypeSafe Jev; exit 1 when the gate fails, 2 when the run is incomplete
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
    Baseline,
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

/// Overview, workflow, exit codes and files, shown by `jevgate --help`.
pub const OVERVIEW: &str = "\
Workflow:
  jevgate init                              Write jevgate.toml: upload scope, rules and gate
  jevgate auth login                        Save an API key (or set TYPESAFE_API_KEY)
  jevgate check --dry-run --show-requests   Print every request body; no key, no network
  jevgate check                             Review and apply the gate
  jevgate baseline                          Accept current findings; later checks fail only on new ones

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
  TYPESAFE_API_KEY          API key; takes precedence over every saved credential
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
  jevgate check --cache-only                       Replay cached answers; never contact TypeSafe

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

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum Format {
    /// Ranked findings with locations and next steps, for people and coding agents
    Agent,
    /// The full report as one pretty-printed JSON document
    Json,
    /// One compact JSON report per line; one per evaluation while watching
    Jsonl,
    /// GitHub Actions annotations and a job summary, then the agent text
    Github,
}

/// Results that fail the check. Consider also fails on review findings.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum FailOn {
    /// New review findings
    Review,
    /// New review or consider findings
    Consider,
    /// Files whose answers stayed undecided or that need context
    Uncertain,
    /// Nothing; findings are advisory and only an incomplete run exits 2
    None,
}

impl FailOn {
    pub fn name(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Consider => "consider",
            Self::Uncertain => "uncertain",
            Self::None => "none",
        }
    }

    /// A gate level by name; `report` (judge, never fail) is `none`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "report" => Some(Self::None),
            _ => <Self as ValueEnum>::from_str(name, true).ok(),
        }
    }
}

/// A `--fail-on` value: a level for every rule, or `TARGET=LEVEL` for a rule
/// ID, key or group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailOnSpec {
    pub target: Option<String>,
    pub level: FailOn,
}

fn fail_on_spec(value: &str) -> Result<FailOnSpec, String> {
    let (target, level) = match value.split_once('=') {
        Some((target, level)) => (Some(target.trim().to_string()), level.trim()),
        None => (None, value.trim()),
    };
    let level = FailOn::parse(level).ok_or_else(|| {
        format!("Unknown level {level:?}; use review, consider, uncertain or none")
    })?;
    Ok(FailOnSpec { target, level })
}

const SCOPE: &str = "Scope";
const RULES: &str = "Rules and gate";
const OUTPUT: &str = "Output";
const BUDGETS: &str = "Model, budgets and cache";
const WATCH: &str = "Watch";

/// The model used when neither `--model` nor `model` in jevgate.toml names one.
pub const DEFAULT_MODEL: &str = "jev-1.13.0";
/// Cache lifetime for the `jev-latest` and `jev-preview` aliases, in seconds.
pub const DEFAULT_CACHE_TTL_SECS: u64 = 3600;

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Files or directories to review [default: discovered application source]
    ///
    /// Without paths, JevGate walks the repository (respecting .gitignore) and
    /// selects application source in Rust, Python, JavaScript and TypeScript.
    /// Tests, generated code and vendored files are classified and skipped with
    /// a reason. `upload_allow`/`upload_deny` in jevgate.toml still bound what
    /// is sent.
    pub paths: Vec<PathBuf>,
    /// Review only files changed against this Git revision (commit, branch or tag)
    ///
    /// Includes committed, staged, unstaged and untracked changes. Deleted
    /// files are listed in the report. The revision must exist locally: in CI,
    /// check out with full history (for example `fetch-depth: 0`). When no
    /// supported file changed, the run is complete and exits 0.
    #[arg(long, value_name = "REVISION", help_heading = SCOPE)]
    pub base: Option<String>,
    /// Also judge tests: test value, redundancy, and shared logic among tests
    ///
    /// Test files are otherwise listed as not applicable. Also set by
    /// `include_tests = true` in jevgate.toml.
    #[arg(long, help_heading = SCOPE)]
    pub include_tests: bool,
    /// Related file sent as evidence for shared logic, callers and test subjects (repeatable)
    ///
    /// The file must be inside the repository and is sent only with the
    /// requests it informs. Also set by `context` in jevgate.toml.
    #[arg(long, value_name = "PATH", help_heading = SCOPE)]
    pub context: Vec<PathBuf>,
    /// Also review this file extension as text (repeatable, without the dot)
    #[arg(long, value_name = "EXT", value_parser = source_extension, help_heading = SCOPE)]
    pub source_extension: Vec<String>,
    /// Read this configuration instead of <repository root>/jevgate.toml
    ///
    /// The repository root is still found from the working directory. Use it
    /// in CI to apply a reviewed policy that the change under review cannot
    /// edit.
    #[arg(long, value_name = "FILE", help_heading = SCOPE)]
    pub config: Option<PathBuf>,
    /// Select a rule ID, key or group (repeatable) [default: the `default` group]
    ///
    /// Groups: maintainability, tests, security, documentation, default (every
    /// rule on by default) and all. Naming any rule replaces the configured
    /// selection, so add `--rule default` to keep the defaults. Test rules also
    /// need --include-tests. `jevgate rules` lists every rule.
    #[arg(long = "rule", value_name = "RULE", help_heading = RULES)]
    pub rules: Vec<String>,
    /// Deselect a rule ID, key or group (repeatable); applied after --rule and jevgate.toml
    #[arg(long = "skip-rule", value_name = "RULE", help_heading = RULES)]
    pub skip_rules: Vec<String>,
    /// What fails the gate: LEVEL for every rule, or TARGET=LEVEL (repeatable) [default: review]
    ///
    /// LEVEL is review, consider (also fails on review), uncertain, or none
    /// (advisory; `report` is accepted as a synonym). TARGET is a rule ID, key
    /// or group, for example `security=consider`; the most specific target
    /// wins. Flags replace `fail_on` and `[rules]` levels from jevgate.toml
    /// for the rules they address. Notes and baselined findings never fail the
    /// gate. An incomplete run exits 2 regardless of the gate.
    #[arg(long = "fail-on", value_name = "[TARGET=]LEVEL", value_parser = fail_on_spec, help_heading = RULES)]
    pub fail_on_specs: Vec<FailOnSpec>,
    /// The resolved levels for rules without their own: from --fail-on, else configuration.
    #[arg(skip)]
    pub fail_on: Vec<FailOn>,
    /// Resolved levels of each enabled rule key that differ from `fail_on`.
    #[arg(skip)]
    pub rule_fail_on: BTreeMap<String, Vec<FailOn>>,
    /// Output format [default: agent; jsonl with --watch; json with --show-requests]
    #[arg(long, value_enum, help_heading = OUTPUT)]
    pub format: Option<Format>,
    /// Show optional notes, every consider finding and per-file detail in agent output
    #[arg(long, help_heading = OUTPUT)]
    pub verbose: bool,
    /// Also write .jevgate/report.html and open it in a browser (not opened when CI is set)
    #[arg(long, conflicts_with = "dry_run", help_heading = OUTPUT)]
    pub report: bool,
    /// List the selected files and rules without credentials, network or saved state
    #[arg(long, help_heading = OUTPUT)]
    pub dry_run: bool,
    /// With --dry-run, include every initial request body (the exact source and questions)
    ///
    /// Follow-up requests depend on answers and are not known in advance.
    #[arg(long, requires = "dry_run", help_heading = OUTPUT)]
    pub show_requests: bool,
    /// TypeSafe model; pin a version for repeatable results [default: jev-1.13.0]
    ///
    /// Also set by `model` in jevgate.toml. Answers are cached per model, so
    /// changing it re-asks every unit.
    #[arg(long, help_heading = BUDGETS)]
    pub model: Option<String>,
    /// Stop after this many API attempts in this invocation, watch updates included
    ///
    /// Reaching the budget leaves the run incomplete (exit 2) rather than
    /// passing on partial evidence. `max_requests` in jevgate.toml is a
    /// ceiling this flag can only lower.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=1000000), help_heading = BUDGETS)]
    pub max_requests: Option<u32>,
    /// Maximum simultaneous TypeSafe requests (1-8)
    #[arg(long, value_name = "N", default_value_t = 6, value_parser = clap::value_parser!(u32).range(1..=MAX_CONCURRENCY as i64), help_heading = BUDGETS)]
    pub concurrency: u32,
    /// Per-file read limit; a larger file is reported as needs-context, never truncated
    #[arg(long, value_name = "BYTES", default_value_t = DEFAULT_MAX_FILE_BYTES, value_parser = clap::value_parser!(u64).range(1..=1048576), help_heading = BUDGETS)]
    pub max_file_bytes: u64,
    /// Total bytes of --context files per request; context is never truncated
    #[arg(long, value_name = "BYTES", default_value_t = 32768, value_parser = clap::value_parser!(u64).range(1..=1048576), help_heading = BUDGETS)]
    pub max_context_bytes: u64,
    /// Cache lifetime for the jev-latest and jev-preview aliases [default: 3600]
    ///
    /// Answers from a pinned model version never expire. Also set by
    /// `cache_ttl_secs` in jevgate.toml.
    #[arg(long, value_name = "SECONDS", help_heading = BUDGETS)]
    pub cache_ttl_secs: Option<u64>,
    /// Ignore cached answers for this invocation and ask again
    #[arg(long, help_heading = BUDGETS)]
    pub refresh: bool,
    /// Use cached answers only and never contact TypeSafe; unanswered units leave the run incomplete
    #[arg(long, conflicts_with = "refresh", help_heading = BUDGETS)]
    pub cache_only: bool,
    /// Credential file holding TYPESAFE_API_KEY [default: <repository root>/.env]
    ///
    /// The TYPESAFE_API_KEY environment variable takes precedence.
    #[arg(long, value_name = "FILE", help_heading = BUDGETS)]
    pub env_file: Option<PathBuf>,
    /// Keep running and re-check the selected files after each save
    ///
    /// Writes .jevgate/latest.json after every evaluation and prints one JSON
    /// report per line. Pair with `jevgate serve` or --report.
    #[arg(long, help_heading = WATCH)]
    pub watch: bool,
    /// Wait this long after the last save before evaluating
    #[arg(long, value_name = "MS", default_value_t = 500, value_parser = clap::value_parser!(u64).range(50..=60000), help_heading = WATCH)]
    pub debounce_ms: u64,
    /// How often to look for saves
    #[arg(long, value_name = "MS", default_value_t = 250, value_parser = clap::value_parser!(u64).range(50..=60000), help_heading = WATCH)]
    pub poll_ms: u64,
    /// Compatibility flag; has no effect
    #[arg(long, hide = true)]
    pub quick: bool,
}

/// Upper bound on simultaneous requests; rate-limit retries share one cooldown.
pub const MAX_CONCURRENCY: u32 = 8;

/// Default read limit per file. Units are sent separately, so this bounds
/// local reading rather than one request. Configuration and
/// `--max-file-bytes` can only narrow it.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 262_144;

fn names(levels: &[FailOn]) -> Vec<String> {
    levels.iter().map(|f| f.name().to_string()).collect()
}

fn source_extension(value: &str) -> Result<String, String> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Use an extension without a dot, for example: --source-extension zig".into());
    }
    Ok(value.to_ascii_lowercase())
}

impl CheckArgs {
    /// Whether a rule is selected, by key or ID.
    pub fn enabled(&self, key: &str) -> bool {
        self.rules
            .iter()
            .any(|r| r == key || r == crate::catalog::id(key))
    }

    /// Whether a rule that judges application source is selected.
    pub fn code_rules(&self) -> bool {
        self.rules.iter().any(|r| {
            crate::catalog::find(r).is_some_and(|rule| {
                !crate::catalog::DOCUMENTATION.contains(&rule.key)
                    && ![crate::catalog::ACCESS_CONTROL, crate::catalog::WORKFLOWS]
                        .contains(&rule.key)
            })
        })
    }

    /// Whether any documentation rule is selected, so instruction files are found.
    pub fn documentation(&self) -> bool {
        crate::catalog::DOCUMENTATION
            .iter()
            .any(|key| self.enabled(key))
    }

    pub fn fail_on_names(&self) -> Vec<String> {
        names(&self.fail_on)
    }

    /// Levels that differ from `fail_on`, by rule ID, for the report.
    pub fn rule_fail_on_names(&self) -> BTreeMap<String, Vec<String>> {
        self.rule_fail_on
            .iter()
            .map(|(key, levels)| (crate::catalog::id(key).to_string(), names(levels)))
            .collect()
    }

    /// The gate levels of a rule, by ID or key.
    pub fn levels(&self, rule: &str) -> &[FailOn] {
        crate::catalog::find(rule)
            .and_then(|r| self.rule_fail_on.get(r.key))
            .unwrap_or(&self.fail_on)
    }

    /// The model to ask: `--model`, else configuration, else [`DEFAULT_MODEL`].
    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }

    pub fn cache_ttl_secs(&self) -> u64 {
        self.cache_ttl_secs.unwrap_or(DEFAULT_CACHE_TTL_SECS)
    }

    pub fn output_format(&self) -> Format {
        self.format.unwrap_or(if self.show_requests {
            Format::Json
        } else if self.watch {
            Format::Jsonl
        } else {
            Format::Agent
        })
    }
}
