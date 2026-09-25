//! The `check` arguments, output formats and gate levels; the subcommands and
//! their help text are in `commands`.
mod commands;

pub use commands::{BaselineAction, Disposition, JevCommand, OVERVIEW, RulesFormat};

use clap::{Args, ValueEnum};
use std::{collections::BTreeMap, path::PathBuf};

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
    /// selects application source in Rust, Python, JavaScript, TypeScript, Go,
    /// C#, Ruby and PHP.
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
    /// Without it, test files are judged only for file organization. Also set
    /// by `include_tests = true` in jevgate.toml.
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
    /// Levels for the files `[[scope]]` entries match, in configuration order.
    #[arg(skip)]
    pub path_fail_on: Vec<PathLevels>,
    /// Output format [default: agent; jsonl with --watch; json with --show-requests]
    #[arg(long, value_enum, help_heading = OUTPUT)]
    pub format: Option<Format>,
    /// Show optional notes, every consider finding and per-file detail in agent output
    #[arg(long, help_heading = OUTPUT)]
    pub verbose: bool,
    /// Also write .jevgate/report.html and open it in a browser (not opened when CI is set)
    #[arg(long, conflicts_with = "dry_run", help_heading = OUTPUT)]
    pub report: bool,
    /// List the selected files, rules and planned requests without credentials, network or writes
    ///
    /// Planned first-pass requests the cache already answers are counted apart
    /// and cost nothing; follow-ups depend on the answers and are not known.
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

/// Gate levels of one `[[scope]]`: the rules it addresses for the files its
/// paths match.
#[derive(Clone, Debug)]
pub struct PathLevels {
    pub paths: Vec<String>,
    pub matcher: globset::GlobSet,
    /// Levels by rule key.
    pub rules: BTreeMap<String, Vec<FailOn>>,
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

    /// Whether a rule that judges application source is selected. Access
    /// control judges SpacetimeDB modules as well as SQL.
    pub fn code_rules(&self) -> bool {
        self.rules
            .iter()
            .any(|r| crate::catalog::find(r).is_some_and(|rule| self.code_rules_include(rule.key)))
    }

    /// Whether rule `key` judges application source.
    pub fn code_rules_include(&self, key: &str) -> bool {
        !crate::catalog::DOCUMENTATION.contains(&key) && key != crate::catalog::WORKFLOWS
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

    /// The gate levels of a rule, by ID or key, outside any scope.
    pub fn levels(&self, rule: &str) -> &[FailOn] {
        crate::catalog::find(rule)
            .and_then(|r| self.rule_fail_on.get(r.key))
            .unwrap_or(&self.fail_on)
    }

    /// The gate levels of a rule for one file: the last scope that matches
    /// the file and addresses the rule, else [`Self::levels`].
    pub fn levels_at(&self, rule: &str, path: &std::path::Path) -> &[FailOn] {
        crate::catalog::find(rule)
            .and_then(|r| {
                self.path_fail_on
                    .iter()
                    .rev()
                    .filter(|scope| scope.matcher.is_match(path))
                    .find_map(|scope| scope.rules.get(r.key))
            })
            .map_or_else(|| self.levels(rule), Vec::as_slice)
    }

    /// Scope levels that differ from the rest, by rule ID, for the report.
    pub fn path_fail_on_names(&self) -> Vec<crate::schema::PathFailOn> {
        self.path_fail_on
            .iter()
            .map(|scope| crate::schema::PathFailOn {
                paths: scope.paths.clone(),
                rules: scope
                    .rules
                    .iter()
                    .map(|(key, levels)| (crate::catalog::id(key).to_string(), names(levels)))
                    .collect(),
            })
            .collect()
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
