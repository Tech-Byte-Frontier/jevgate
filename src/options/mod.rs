//! The `check` arguments, output formats and gate levels; the subcommands and
//! their help text are in `commands`, and the rules actions' in `rules`.
mod commands;
mod rules;

pub use commands::{BaselineAction, Disposition, JevCommand, OVERVIEW, RulesFormat};
pub use rules::{ProposeArgs, ProposeFormat, RulesAction, RulesTestArgs};

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
    /// A SARIF 2.1.0 log, for GitHub code scanning and other SARIF readers
    Sarif,
    /// A GitLab Code Quality report, for merge request widgets
    Gitlab,
}

/// When agent output is colored.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum ColorChoice {
    /// On a terminal, unless NO_COLOR is set; CLICOLOR_FORCE turns it on elsewhere
    Auto,
    Always,
    Never,
}

/// Results that fail the check. Consider also fails on review findings.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum FailOn {
    /// New review findings
    Review,
    /// New review or consider findings
    Consider,
    /// New findings of the rule's mature levels, measured right at least 80%
    /// of the time on projects JevGate was never tuned on, or of a custom
    /// question's own level (the default)
    Mature,
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
            Self::Mature => "mature",
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
/// ID, name, key or group.
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
        format!("Unknown level {level:?}; use review, consider, mature, uncertain or none")
    })?;
    Ok(FailOnSpec { target, level })
}

const SCOPE: &str = "Scope";
const RULES: &str = "Rules and gate";
const OUTPUT: &str = "Output";
const BUDGETS: &str = "Model, budgets and cache";
const WATCH: &str = "Watch";

/// The model asked with a TypeSafe key when neither `--model` nor `model` in
/// jevgate.toml names one.
pub const DEFAULT_MODEL: &str = "jev-1.13.0";
/// Cache lifetime for an alias (a model name without an `x.y.z` version, such
/// as `jev-latest`), in seconds.
pub const DEFAULT_CACHE_TTL_SECS: u64 = 3600;

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Files or directories to review [default: discovered application source]
    ///
    /// Without paths, JevGate walks the repository (respecting .gitignore) and
    /// selects application source in Rust, Python, JavaScript, TypeScript, Go,
    /// C#, Ruby, PHP, Java and Bend 2 (and, in preview, C, C++, Kotlin, Swift,
    /// Bash, Dart, Scala, Elixir and Lua), the scripts of Astro, Vue and Svelte files, and
    /// server templates (ERB, EJS, JSP, Handlebars, Jinja and others) that
    /// hold inline scripts or code reading the request. Tests, generated code
    /// and vendored files are classified and skipped with a reason.
    /// `upload_allow`/`upload_deny` in jevgate.toml still bound what is sent.
    pub paths: Vec<PathBuf>,
    /// Review only what changed against this Git revision (commit, branch or tag)
    ///
    /// Compares with the fork point, as a pull request diff does, and includes
    /// committed, staged, unstaged and untracked changes. Only what the change
    /// touches is asked about and reported: functions, tests, comments and
    /// values on changed lines, copies where either copy changed, a file's
    /// outline when the change adds members to it, and documents naming a
    /// path it deleted or renamed. A new file is judged whole. Deleted files
    /// are listed in the report. The revision must exist locally: in CI,
    /// check out with full history (for example `fetch-depth: 0`). When no
    /// supported file changed, the run is complete and exits 0.
    #[arg(long, value_name = "REVISION", help_heading = SCOPE)]
    pub base: Option<String>,
    /// With --base, judge each changed file whole, not only what the change touches
    #[arg(long, requires = "base", help_heading = SCOPE)]
    pub whole_files: bool,
    /// Set by the agent hook: a snapshot of the working tree (a Git tree). The
    /// change is then `base`, the turn's snapshot, to this one, with no merge
    /// base: the fork point of a snapshot and HEAD is HEAD itself.
    #[arg(skip)]
    pub worktree_snapshot: Option<String>,
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
    /// edit. The custom question files of .jevgate/questions/ are then not
    /// read, since the change could edit them too; --questions reads a
    /// reviewed copy.
    #[arg(long, value_name = "FILE", help_heading = SCOPE)]
    pub config: Option<PathBuf>,
    /// Read custom question files from this directory instead of .jevgate/questions/
    ///
    /// With --config, give a reviewed copy of the questions directory, such as
    /// the base revision's, so the change under review cannot edit a question
    /// to pass.
    #[arg(long = "questions", value_name = "DIR", help_heading = SCOPE)]
    pub question_directory: Option<PathBuf>,
    /// Select a rule ID, name, key or group (repeatable) [default: the `default` group]
    ///
    /// Groups: maintainability, tests, security, documentation, custom (the
    /// custom questions; one is `custom/<id>`), default (every rule on by
    /// default) and all. Naming any rule replaces the configured
    /// selection, so add `--rule default` to keep the defaults. Test rules also
    /// need --include-tests. `jevgate rules` lists every rule.
    #[arg(long = "rule", value_name = "RULE", help_heading = RULES)]
    pub rules: Vec<String>,
    /// Deselect a rule ID, name, key or group (repeatable); applied after --rule and jevgate.toml
    #[arg(long = "skip-rule", value_name = "RULE", help_heading = RULES)]
    pub skip_rules: Vec<String>,
    /// What fails the gate: LEVEL for every rule, or TARGET=LEVEL (repeatable) [default: mature]
    ///
    /// LEVEL is review, consider (also fails on review), mature, uncertain, or
    /// none (advisory; `report` is accepted as a synonym). `mature` fails only
    /// on the levels of a rule measured right at least 80% of the time on
    /// projects JevGate was never tuned on, and on a custom question's own
    /// level (`jevgate rules` shows them); other findings are reported without
    /// failing. TARGET is a rule ID, key or
    /// group, for example `security=consider`; the most specific target wins.
    /// Flags replace `fail_on` and `[rules]` levels from jevgate.toml for the
    /// rules they address. Notes and baselined findings never fail the gate.
    /// An incomplete run exits 2 regardless of the gate.
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
    /// Every custom question the configuration defines; `rules` says which
    /// this run asks.
    #[arg(skip)]
    pub questions: &'static [crate::custom::Question],
    /// The opening of the repository's README when the upload boundary
    /// permits it: what the program is and who runs it, for the question of
    /// who reads an error-detail finding's responses.
    #[arg(skip)]
    pub project: Option<String>,
    /// The provider of the key the check will use, found before planning so
    /// that its default model is the one asked; never from jevgate.toml.
    #[arg(skip)]
    pub provider: crate::provider::Provider,
    /// Output format [default: agent; jsonl with --watch; json with --show-requests]
    #[arg(long, value_enum, help_heading = OUTPUT)]
    pub format: Option<Format>,
    /// Color agent output: auto, always or never
    ///
    /// `auto` colors a terminal unless NO_COLOR is set, and any output when
    /// CLICOLOR_FORCE is set; on Windows, only Windows Terminal and terminals
    /// that set TERM count. Other formats are never colored.
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = ColorChoice::Auto, help_heading = OUTPUT)]
    pub color: ColorChoice,
    /// Show optional notes, every consider finding and per-file detail in agent output
    #[arg(long, help_heading = OUTPUT)]
    pub verbose: bool,
    /// Also write .jevgate/report.html and open it in a browser (not opened when CI is set)
    #[arg(long, conflicts_with = "dry_run", help_heading = OUTPUT)]
    pub report: bool,
    /// List the selected files, rules and planned requests without credentials, network or writes
    ///
    /// Planned first-pass requests and questions the cache already answers are
    /// counted apart and cost nothing: a request sends only the questions the
    /// cache lacks. Follow-ups depend on the answers and are not known.
    #[arg(long, help_heading = OUTPUT)]
    pub dry_run: bool,
    /// With --dry-run, include every initial request body (the exact source and questions)
    ///
    /// Follow-up requests depend on answers and are not known in advance.
    #[arg(long, requires = "dry_run", help_heading = OUTPUT)]
    pub show_requests: bool,
    /// Model, as the key's provider names it; pin a version for repeatable results [default: the key's provider's model]
    ///
    /// The default follows the key: jev-1.13.0 with a TypeSafe key,
    /// typesafe/jev-1.13 with an OpenRouter key, typesafe-ai/jev with a Vercel
    /// AI Gateway key. Also set by `model` in jevgate.toml. Answers are cached
    /// per model, so changing it re-asks every unit.
    #[arg(long, help_heading = BUDGETS)]
    pub model: Option<String>,
    /// Stop after this many API attempts in this invocation, watch updates included
    ///
    /// Reaching the budget leaves the run incomplete (exit 2) rather than
    /// passing on partial evidence. `max_requests` in jevgate.toml is a
    /// ceiling this flag can only lower.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=1000000), help_heading = BUDGETS)]
    pub max_requests: Option<u32>,
    /// Maximum simultaneous requests, at most 6; a higher value is lowered to 6 [default: 6, or 3 with a gateway's key]
    ///
    /// The default follows the key: 6 with a TypeSafe key, 3 with an
    /// OpenRouter or Vercel AI Gateway key. Also set by `concurrency` in
    /// jevgate.toml, which this flag can only lower.
    #[arg(long, value_name = "N", value_parser = concurrency, help_heading = BUDGETS)]
    pub concurrency: Option<u32>,
    /// Per-file read limit; a larger file is reported as needs-context, never truncated
    #[arg(long, value_name = "BYTES", default_value_t = DEFAULT_MAX_FILE_BYTES, value_parser = clap::value_parser!(u64).range(1..=1048576), help_heading = BUDGETS)]
    pub max_file_bytes: u64,
    /// Total bytes of --context files per request; context is never truncated
    #[arg(long, value_name = "BYTES", default_value_t = 32768, value_parser = clap::value_parser!(u64).range(1..=1048576), help_heading = BUDGETS)]
    pub max_context_bytes: u64,
    /// Cache lifetime for an alias, a model name without an x.y.z version such as jev-latest [default: 3600]
    ///
    /// Answers from a pinned model version, such as jev-1.13.0, never
    /// expire. Also set by `cache_ttl_secs` in jevgate.toml.
    #[arg(long, value_name = "SECONDS", help_heading = BUDGETS)]
    pub cache_ttl_secs: Option<u64>,
    /// Ignore the answers cached before this invocation and ask again
    #[arg(long, help_heading = BUDGETS)]
    pub refresh: bool,
    /// Use cached answers only and never contact the provider; unanswered units leave the run incomplete
    #[arg(long, conflicts_with = "refresh", help_heading = BUDGETS)]
    pub cache_only: bool,
    /// Credential file holding TYPESAFE_API_KEY, OPENROUTER_API_KEY or AI_GATEWAY_API_KEY [default: <repository root>/.env]
    ///
    /// TYPESAFE_API_KEY in the environment takes precedence; the file comes
    /// before the saved key and a gateway's variable in the environment. The
    /// repository's .env is read only for TYPESAFE_API_KEY: a gateway's key
    /// there is usually the application's own.
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

/// Upper bound on simultaneous requests, and the default with a TypeSafe
/// key: six workers made 18 to 20 requests a second on the corpus's largest
/// runs (0.3 s a request), just under TypeSafe's limit of 1,200 a minute;
/// eight would make about 27. Rate-limit retries share one cooldown. A
/// higher `--concurrency` is lowered to it with a notice, since 0.25
/// accepted up to 8; a higher `concurrency` in jevgate.toml means it.
pub const MAX_CONCURRENCY: u32 = 6;

/// Default read limit per file. Units are sent separately, so this bounds
/// local reading rather than one request. Configuration and
/// `--max-file-bytes` can only narrow it.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 262_144;

fn names(levels: &[FailOn]) -> Vec<String> {
    levels.iter().map(|f| f.name().to_string()).collect()
}

/// `--concurrency`: at least 1. A higher value than [`MAX_CONCURRENCY`] is
/// accepted here and lowered to it with a notice when the check starts.
fn concurrency(value: &str) -> Result<u32, String> {
    match value.parse::<u32>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(format!(
            "Use a whole number from 1 to {MAX_CONCURRENCY}; a higher one is lowered to {MAX_CONCURRENCY}"
        )),
    }
}

fn source_extension(value: &str) -> Result<String, String> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Use an extension without a dot, for example: --source-extension zig".into());
    }
    Ok(value.to_ascii_lowercase())
}

impl CheckArgs {
    /// The arguments a `check` without flags has, for commands that ask as a
    /// check does.
    pub fn defaults() -> Self {
        #[derive(clap::Parser)]
        struct Defaults {
            #[command(flatten)]
            args: CheckArgs,
        }
        <Defaults as clap::Parser>::parse_from(["jevgate"]).args
    }

    /// Whether a rule is selected, by key or ID.
    pub fn enabled(&self, key: &str) -> bool {
        self.rules
            .iter()
            .any(|r| r == key || r == crate::catalog::id(key))
    }

    /// Whether a rule that judges application source is selected. Access
    /// control judges SpacetimeDB modules as well as SQL, and every custom
    /// question but one about documentation sections reads source.
    pub fn code_rules(&self) -> bool {
        self.rules
            .iter()
            .any(|r| crate::catalog::find(r).is_some_and(|rule| self.code_rules_include(rule.key)))
            || self.custom_code()
    }

    /// Whether a selected custom question reads source files: every one but
    /// a question about documentation sections.
    pub fn custom_code(&self) -> bool {
        self.custom()
            .any(|q| q.unit != crate::custom::Kind::Section)
    }

    /// The custom questions this run asks.
    pub fn custom(&self) -> impl Iterator<Item = &'static crate::custom::Question> + '_ {
        self.questions.iter().filter(|q| self.enabled(&q.rule))
    }

    /// The key of a rule named by its ID or key, built-in or custom.
    fn key<'a>(&self, rule: &'a str) -> Option<&'a str> {
        match crate::catalog::find(rule) {
            Some(found) => Some(found.key),
            None => self
                .questions
                .iter()
                .any(|q| q.rule == rule)
                .then_some(rule),
        }
    }

    /// Whether rule `key` judges application source.
    pub fn code_rules_include(&self, key: &str) -> bool {
        !crate::catalog::DOCUMENTATION.contains(&key) && key != crate::catalog::WORKFLOWS
    }

    /// Whether a `--base` check judges only what its change touches.
    pub fn changed_lines(&self) -> bool {
        self.base.is_some() && !self.whole_files
    }

    /// The snapshot of the working tree an agent's turn began with, in the
    /// agent hook's checks of a turn: `base`, when `worktree_snapshot` is set.
    pub fn turn_start(&self) -> Option<&str> {
        self.worktree_snapshot.as_ref().and(self.base.as_deref())
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
        self.key(rule)
            .and_then(|key| self.rule_fail_on.get(key))
            .unwrap_or(&self.fail_on)
    }

    /// The gate levels of a rule for one file: the last scope that matches
    /// the file and addresses the rule, else [`Self::levels`].
    pub fn levels_at(&self, rule: &str, path: &std::path::Path) -> &[FailOn] {
        self.key(rule)
            .and_then(|key| {
                self.path_fail_on
                    .iter()
                    .rev()
                    .filter(|scope| scope.matcher.is_match(path))
                    .find_map(|scope| scope.rules.get(key))
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

    /// The levels `mature` stands for, for a rule by ID or key: a built-in
    /// rule's levels measured mature, and a custom question's own level.
    pub fn mature_levels(&self, rule: &str) -> Vec<crate::schema::Strength> {
        match self.questions.iter().find(|q| q.rule == rule) {
            Some(question) => question.blocks(),
            None => crate::maturity::mature_levels(rule),
        }
    }

    /// What `mature` stands for, for the report: the levels of each selected
    /// rule that has some and whose levels include `mature` outside scopes
    /// or in one, by rule ID.
    pub fn mature_level_names(&self) -> BTreeMap<String, Vec<String>> {
        let uses_mature = |key: &str| {
            self.levels(key).contains(&FailOn::Mature)
                || self.path_fail_on.iter().any(|scope| {
                    scope
                        .rules
                        .get(key)
                        .is_some_and(|l| l.contains(&FailOn::Mature))
                })
        };
        self.rules
            .iter()
            .filter(|key| uses_mature(key))
            .filter_map(|key| {
                let levels = self.mature_levels(key);
                let names = levels.iter().map(crate::output::label).collect::<Vec<_>>();
                (!names.is_empty()).then(|| (crate::catalog::id(key).to_string(), names))
            })
            .collect()
    }

    /// The model to ask: `--model`, else configuration, else the default of
    /// the key's provider ([`DEFAULT_MODEL`] for TypeSafe).
    pub fn model(&self) -> &str {
        self.model
            .as_deref()
            .unwrap_or(self.provider.service().default_model)
    }

    pub fn cache_ttl_secs(&self) -> u64 {
        self.cache_ttl_secs.unwrap_or(DEFAULT_CACHE_TTL_SECS)
    }

    /// The most requests sent at once: `--concurrency` or `concurrency` in
    /// jevgate.toml, else the default of the key's provider.
    pub fn concurrency(&self) -> u32 {
        self.concurrency
            .unwrap_or(self.provider.service().default_concurrency)
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
