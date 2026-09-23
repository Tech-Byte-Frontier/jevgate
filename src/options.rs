use clap::{Args, Subcommand, ValueEnum};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Subcommand)]
pub enum JevCommand {
    /// Save, inspect or remove your TypeSafe API credential
    Auth {
        #[command(subcommand)]
        command: crate::auth::AuthCommand,
    },
    /// Evaluate code quality with TypeSafe (uploads selected units); a failed gate exits 1, incomplete exits 2
    Check(Box<CheckArgs>),
    /// Accept the findings of the last complete check in jevgate-baseline.json (no API calls)
    Baseline,
    /// List the rules and their groups (JSON with --format json)
    Rules {
        #[arg(long, value_enum, default_value_t = RulesFormat::Table)]
        format: RulesFormat,
    },
    /// Write a commented jevgate.toml for this repository (no API calls)
    Init {
        /// Replace an existing jevgate.toml
        #[arg(long)]
        force: bool,
    },
    /// Serve read-only snapshots on localhost (run alongside check --watch)
    Serve {
        #[arg(long, default_value_t = 47831)]
        port: u16,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum RulesFormat {
    Table,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum Format {
    Agent,
    Json,
    Jsonl,
}

/// Results that fail the check. Consider also fails on review findings.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum FailOn {
    Review,
    Consider,
    Uncertain,
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

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Files or directories to review; default is discovered application source
    pub paths: Vec<PathBuf>,
    /// Also judge tests: test value, redundancy, shared logic and support functions
    #[arg(long)]
    pub include_tests: bool,
    /// Fail on these results (repeatable): review, consider, uncertain or none, for
    /// every rule or as TARGET=LEVEL for a rule or group (security=consider) [default: review]
    #[arg(long = "fail-on", value_parser = fail_on_spec)]
    pub fail_on_specs: Vec<FailOnSpec>,
    /// The resolved levels for rules without their own: from --fail-on, else configuration.
    #[arg(skip)]
    pub fail_on: Vec<FailOn>,
    /// Resolved levels of each enabled rule key that differ from `fail_on`.
    #[arg(skip)]
    pub rule_fail_on: BTreeMap<String, Vec<FailOn>>,
    /// Show per-file detail in agent output
    #[arg(long)]
    pub verbose: bool,
    /// Review working-tree changes against this Git revision (includes staged and untracked files)
    #[arg(long)]
    pub base: Option<String>,
    /// Compatibility flag; has no effect
    #[arg(long, hide = true)]
    pub quick: bool,
    /// Additional file extension to review as text (repeatable, without a dot)
    #[arg(long, value_parser = source_extension)]
    pub source_extension: Vec<String>,
    /// Related file for shared-logic, caller and subject evidence (repeatable, inside root)
    #[arg(long)]
    pub context: Vec<PathBuf>,
    /// Total context bytes per request, of explicitly supplied files; never truncated
    #[arg(long, default_value_t = 32768, value_parser = clap::value_parser!(u64).range(1..=1048576))]
    pub max_context_bytes: u64,
    /// Keep watching saves; write latest.json and emit successive snapshots
    #[arg(long)]
    pub watch: bool,
    /// Save a local HTML dashboard and open it in your browser (updates while watching)
    #[arg(long, conflicts_with = "dry_run")]
    pub report: bool,
    /// List scope without credentials, network requests, or writing state
    #[arg(long)]
    pub dry_run: bool,
    /// Include initial request bodies (selected source and questions) in a dry run
    #[arg(long, requires = "dry_run")]
    pub show_requests: bool,
    /// Output format (watch defaults to jsonl; one-shot defaults to agent)
    #[arg(long, value_enum)]
    pub format: Option<Format>,
    /// TypeSafe model (pin a version for repeatable policy)
    #[arg(long, default_value = "jev-1.13.0")]
    pub model: String,
    /// Credential file (default: repository root/.env); environment key takes precedence
    #[arg(long)]
    pub env_file: Option<PathBuf>,
    /// Optional API attempt ceiling for this invocation, including watch updates
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000000))]
    pub max_requests: Option<u32>,
    /// Maximum simultaneous independent TypeSafe requests (questions within each call are parallel)
    #[arg(long, default_value_t = 6, value_parser = clap::value_parser!(u32).range(1..=MAX_CONCURRENCY as i64))]
    pub concurrency: u32,
    /// Per-file read limit. A larger file is not judged; its size and parsed
    /// operation names are reported as needs-context. Source is never truncated.
    #[arg(long, default_value_t = DEFAULT_MAX_FILE_BYTES, value_parser = clap::value_parser!(u64).range(1..=1048576))]
    pub max_file_bytes: u64,
    /// Cache lifetime for the jev-latest and jev-preview aliases. Answers from a
    /// pinned model version do not expire.
    #[arg(long, default_value_t = 3600)]
    pub cache_ttl_secs: u64,
    /// Ignore disk cache for this invocation (unchanged watch files still reuse results)
    #[arg(long)]
    pub refresh: bool,
    /// Reuse valid cached responses only; never contact TypeSafe
    #[arg(long, conflicts_with = "refresh")]
    pub cache_only: bool,
    /// Wait this long after changes settle before evaluating
    #[arg(long, default_value_t = 500, value_parser = clap::value_parser!(u64).range(50..=60000))]
    pub debounce_ms: u64,
    /// Poll interval for watch mode
    #[arg(long, default_value_t = 250, value_parser = clap::value_parser!(u64).range(50..=60000))]
    pub poll_ms: u64,
    /// Enable a rule ID, key or group (repeatable), such as `security`; defaults to
    /// the `default` group. Test rules need --include-tests
    #[arg(long = "rule")]
    pub rules: Vec<String>,
    /// Disable a rule ID, key or group (repeatable)
    #[arg(long = "skip-rule")]
    pub skip_rules: Vec<String>,
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

    /// Whether a rule that judges source code is selected.
    pub fn code_rules(&self) -> bool {
        self.rules.iter().any(|r| {
            crate::catalog::find(r)
                .is_some_and(|rule| !crate::catalog::DOCUMENTATION.contains(&rule.key))
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
