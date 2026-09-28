//! The subcommands, the baseline actions and their help text.
use super::CheckArgs;
use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub enum JevCommand {
    /// Save, inspect or remove your API key: TypeSafe, OpenRouter or Vercel AI Gateway
    ///
    /// A check uses the first key it finds: TYPESAFE_API_KEY in the
    /// environment; then the file named by `check --env-file` (TYPESAFE_API_KEY,
    /// OPENROUTER_API_KEY or AI_GATEWAY_API_KEY), else TYPESAFE_API_KEY in the
    /// repository's `.env`; then the key saved by `jevgate auth login`; then
    /// OPENROUTER_API_KEY or AI_GATEWAY_API_KEY in the environment, which
    /// other tools read too. The key goes only to its own provider. In CI, set
    /// one variable from a secret; nothing needs to be saved.
    #[command(after_long_help = AUTH_EXAMPLES)]
    Auth {
        #[command(subcommand)]
        command: crate::auth::AuthCommand,
    },
    /// Review code with TypeSafe Jev; exit 1 when the gate fails, 2 when the run is incomplete
    ///
    /// Parses the selected files locally, sends small evidence units (a
    /// function, a file outline, a pair of copies, a test, a documentation
    /// section, a code comment) with short questions, and composes the answers into findings.
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
    /// A single finding can instead be accepted in the code, with a comment
    /// `jevgate: allow(RULE) reason` on its line or directly above it.
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
        /// by what the check found, and the rest are kept. A `--base` check that
        /// judged only what its change touched covers only the deleted files:
        /// the other entries of the files it checked stay.
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
    /// List every rule with its default, the levels that fail the check by default, and its question
    ///
    /// A rule is named by its ID (`maintainability/shared-logic`), its key
    /// (`shared_logic`) or its group (`maintainability`, `tests`, `security`,
    /// `documentation`, plus `default` and `all`) anywhere a rule is accepted:
    /// `--rule`, `--skip-rule`, `--fail-on TARGET=LEVEL` and `[rules]`.
    ///
    /// Each rule shows how often its reviews and considers were right on
    /// projects JevGate was never tuned on, from findings labeled by hand. The
    /// levels right at least 80% of the time over at least 20 labels are
    /// mature: by default only they fail the check (`--fail-on mature`), and
    /// the other findings are reported without failing it.
    Rules {
        /// `table` for people; `json` adds scope, evidence unit, version, labels per level and decision policy
        #[arg(long, value_enum, default_value_t = RulesFormat::Table)]
        format: RulesFormat,
    },
    /// Write a commented jevgate.toml, or set up a coding agent's hooks (offline)
    ///
    /// Without --agent: limits uploads to the detected source and test
    /// directories and to agent instruction files, denies credential files,
    /// and lists every rule group with its gate level. Review the file before
    /// the first paid check.
    ///
    /// With --agent: writes the agent's hooks, which run `jevgate hook` when a
    /// turn starts, after each edit and when the turn ends, and a short text
    /// telling the agent how JevGate's findings work (a block between
    /// `<!-- jevgate:begin -->` and `<!-- jevgate:end -->` in AGENTS.md or
    /// GEMINI.md, or a rules file of its own). It merges into the files already
    /// there and changes nothing else, so running it again changes nothing, and
    /// --remove takes out only what it wrote. Every file is read before the
    /// first is written: a settings file that is not plain JSON (comments
    /// included) stops it with nothing written. It then runs the `jevgate` on
    /// your PATH, which the agent will run, and warns when that one cannot
    /// answer the hooks.
    #[command(after_long_help = INIT_EXAMPLES)]
    Init {
        /// Replace an existing jevgate.toml
        #[arg(long, conflicts_with = "agents")]
        force: bool,
        #[command(flatten)]
        setup: crate::setup::AgentSetup,
    },
    /// Print a shell completion script (offline)
    #[command(after_long_help = COMPLETIONS_EXAMPLES)]
    Completions {
        /// bash, zsh, fish, elvish or powershell
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print a man page in roff (offline)
    ///
    /// Without a command, the page for `jevgate`; with one, the page for that
    /// command, such as `jevgate-check`.
    #[command(after_long_help = MAN_EXAMPLES)]
    Man {
        /// A command: auth, check, baseline, rules, init, serve, mcp, hook or completions
        command: Option<String>,
    },
    /// Run a Model Context Protocol server on stdin and stdout, for coding agents
    ///
    /// Offers three tools: `jevgate_check` runs a check in the repository and
    /// returns its findings, `jevgate_findings` reads the last report, and
    /// `jevgate_rules` lists the rules. Register it with an agent as the
    /// command `jevgate mcp`, started in the repository.
    #[command(after_long_help = MCP_EXAMPLES)]
    Mcp,
    /// Answer one coding-agent hook event read on stdin; always exits 0
    ///
    /// Configured as a hook of Claude Code, Codex, Gemini CLI, Cursor,
    /// OpenCode, Copilot CLI or VS Code, it reads the event as JSON on stdin
    /// and prints one JSON reply. VS Code names its edit tools its own way,
    /// which is not verified yet: there only the end of a turn is sure to be
    /// checked. When a turn starts, it records a snapshot of
    /// the working tree under `.jevgate/turns/`; after each edit, it checks
    /// what the turn changed in the edited files and passes the findings to
    /// the agent, never blocking; when the turn ends, it checks everything
    /// the turn changed and blocks the agent while findings fail the gate, at
    /// most 3 times a turn. Checks judge a turn as `check --base` judges a
    /// change, with the repository's jevgate.toml and key.
    ///
    /// It always exits 0, since agents read exit 2 as a block: an outage, an
    /// HTTP 402, a missing key or a directory outside Git never blocks the
    /// agent, and the reply says so to the person and to the agent.
    #[command(after_long_help = HOOK_EXAMPLES)]
    Hook(crate::hook::HookArgs),
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
  jevgate auth login                        Save an API key: TypeSafe, OpenRouter or Vercel AI Gateway
  jevgate check --dry-run --show-requests   Print every request body; no key, no network
  jevgate check                             Review and apply the gate
  jevgate baseline                          Accept current findings; later checks fail only on new ones
  jevgate baseline --merge                  Accept a partial check's findings, keeping the rest
  jevgate baseline mark wrong PATH[:LINE]   Record why a finding was accepted; `baseline stats` counts them

For agents and CI:
  jevgate init --agent claude                        Hooks for Claude Code (also codex, cursor, gemini, opencode)
  jevgate check --base origin/main                   Only what changed since a revision
  jevgate check --base origin/main --format json     The full report, raw probabilities included
  jevgate check --base origin/main --format github   Annotations and a job summary on GitHub
  jevgate rules --format json                        Every rule and the question it asks
  jevgate hook                                       Answer a coding agent's hook event read on stdin

Exit codes:
  0      Gate passed, or no supported file changed since --base
  1      Gate failed
  2      Run incomplete (no key, provider rejection, request budget reached), invalid
         configuration or invalid usage
  128+N  Interrupted by signal N
  `jevgate hook` always exits 0: agents read 2 as a block, so its JSON reply says what happened

Files (at the repository root):
  jevgate.toml            Configuration; `jevgate init` writes a commented one
  jevgate-baseline.json   Accepted findings; commit it
  .jevgate/cache/         Answers by request hash; safe to restore and save in CI
  .jevgate/latest.json    The last report, the same JSON as --format json
  .jevgate/report.html    HTML dashboard, with --report

Environment:
  TYPESAFE_API_KEY          A TypeSafe key; wins over --env-file, .env and the saved key
  OPENROUTER_API_KEY        An OpenRouter key, used when no key comes from those
  AI_GATEWAY_API_KEY        A Vercel AI Gateway key, used when neither comes first
  JEVGATE_BASE_URL          Send requests to this API root instead (https, or http to localhost),
                            for a self-hosted proxy; never read from jevgate.toml or .env
  JEVGATE_CREDENTIAL_STORE  Where `auth login` saves: auto, keyring or file
  JEVGATE_CONFIG_DIR        Absolute directory for file-stored credentials
  CI                        When set, --report writes the dashboard without opening a browser
  NO_COLOR, CLICOLOR_FORCE  Turn agent output color off or on where --color is auto

`jevgate <command> --help` explains each command; -h prints a summary. `jevgate completions SHELL`
and `jevgate man [COMMAND]` print shell completions and man pages.";

const CHECK_EXAMPLES: &str = "\
Examples:
  jevgate check                                    Discovered application source, default rules
  jevgate check src/billing --verbose              One directory, with notes and per-file detail
  jevgate check --base origin/main --format json   Only what changed, machine-readable
  jevgate check --base origin/main --whole-files   Every unit of each changed file
  jevgate check --rule default --rule security     Add the opt-in security group
  jevgate check --rule documentation               Agent instruction files, project docs and code comments
  jevgate check --rule comments                    Only code comments: repeated code, filler, narrated edits
  jevgate check --include-tests                    Also judge test value and redundancy
  jevgate check --fail-on none                     Advisory: never exits 1; exits 2 when incomplete
  jevgate check --fail-on review                   Fail on every review, not only on mature rules
  jevgate check --fail-on review --fail-on security=consider
  jevgate check --dry-run --show-requests          Exactly what would be uploaded, offline
  jevgate check --cache-only                       Replay cached answers; never contact the provider

Reading the JSON report (--format json or .jevgate/latest.json):
  complete           false when any selected file was not judged; the exit code is then 2
  scope              whole-files, or changed-lines when --base judged what changed
  gate               passed, reasons, new_findings, baselined_findings
  fail_on            the gate levels; fail_on_mature says what `mature` stands for
  files[].status     clear, note, consider, review, uncertain, needs-context,
                     not-applicable, skipped or error
  files[].findings   rule, strength, line, message, action, locations,
                     concern_probability, fingerprint, baselined, and gate:
                     fails, measuring (its rule and level are still being
                     measured) or advisory (below the level in force)
  files[].dimensions per rule: status, unit counts and the units left undecided
  files[].judgments  every raw answer, first pass and follow-ups
  api_requests, paid_input_tokens, paid_output_tokens   this run's usage
  paid_models        input tokens by the model that answered them
  estimated_usd      this run's cost; null when a response reported no usage
                     (unmetered_requests) or the model has no known price";

const COMPLETIONS_EXAMPLES: &str = "\
Examples:
  jevgate completions bash > ~/.local/share/bash-completion/completions/jevgate
  jevgate completions zsh > \"${fpath[1]}/_jevgate\"
  jevgate completions fish > ~/.config/fish/completions/jevgate.fish
  jevgate completions powershell >> $PROFILE";

const MCP_EXAMPLES: &str = "\
Examples:
  claude mcp add jevgate -- jevgate mcp       Claude Code, in the repository
  {\"mcpServers\": {\"jevgate\": {\"command\": \"jevgate\", \"args\": [\"mcp\"]}}}
                                              Clients configured with JSON, such as Cursor";

const HOOK_EXAMPLES: &str = "\
Examples (`jevgate init --agent` writes each agent's hooks):
  jevgate hook                      Claude Code, Codex, Gemini CLI, Copilot CLI: detected from the event
  jevgate hook --agent cursor       Cursor's own hooks.json
  jevgate hook --agent opencode     OpenCode, through JevGate's plugin
  jevgate hook --timeout 20         Give up after 20 seconds, whatever the event

Claude Code and Codex run `jevgate hook || echo '{\"systemMessage\": …}'` and Gemini CLI
`jevgate hook; exit 0`, so a missing or older jevgate says so instead of blocking the agent.

Events: a session or turn start records the working tree (SessionStart, UserPromptSubmit,
BeforeAgent, beforeSubmitPrompt); an edit is checked (PostToolUse, AfterTool, postToolUse);
the end of a turn is checked and can be blocked (Stop, AfterAgent, stop). Others get {}.";

const INIT_EXAMPLES: &str = "\
Examples:
  jevgate init                                  jevgate.toml for this repository
  jevgate init --agent claude                   Claude Code's hooks, for every repository you open
  jevgate init --agent codex,gemini --project   This repository's Codex and Gemini CLI hooks
  jevgate init --agent cursor --dry-run         What would change, without writing
  jevgate init --agent claude --remove          Take out what JevGate wrote

Files, for your user and with --project:
  claude     ~/.claude/settings.json, rules/jevgate.md       .claude/settings.json, .claude/rules/jevgate.md
  codex      ~/.codex/hooks.json, AGENTS.md                  .codex/hooks.json, AGENTS.md
  cursor     ~/.cursor/hooks.json                            .cursor/hooks.json, .cursor/rules/jevgate.mdc
  gemini     ~/.gemini/settings.json, GEMINI.md              .gemini/settings.json, GEMINI.md
  opencode   ~/.config/opencode/plugins/jevgate.js, AGENTS.md   .opencode/plugins/jevgate.js, AGENTS.md
CLAUDE_CONFIG_DIR, CODEX_HOME and XDG_CONFIG_HOME move the user files as they move the agents'.";

const MAN_EXAMPLES: &str = "\
Examples:
  jevgate man > ~/.local/share/man/man1/jevgate.1
  jevgate man check > ~/.local/share/man/man1/jevgate-check.1
  jevgate man check | man -l -                  Read a page without installing it (man-db)";

const AUTH_EXAMPLES: &str = "\
Examples:
  jevgate auth login                               Asks the kind of key, then a hidden prompt
  jevgate auth login --with-key < key.txt          Read a TypeSafe key from stdin
  jevgate auth login --with-key --provider openrouter < key.txt
  jevgate auth status                              Show which key a check would use and verify it
  jevgate auth status --offline --json             Same, without contacting the provider
  jevgate auth logout";

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum RulesFormat {
    Table,
    Json,
}
