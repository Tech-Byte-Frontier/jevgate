# Coding agents

JevGate's default output is written for coding agents as much as for people: ranked findings, each with a location, how often findings like it were right and a next step, and nothing hidden when an answer stays undecided.

## Check before finishing

Ask the agent to review its own change before it reports back, for example in `AGENTS.md` or `CLAUDE.md`:

```markdown
Before finishing, run `jevgate check --base origin/main`. Fix each finding marked
"fails the gate". Weigh the other `review` and `consider` findings: fix one when it
is right, or say why the code should stay as it is.
```

`--base` limits the review to what changed since that revision, uncommitted and untracked changes included: the functions, tests and comments on changed lines, and copies where either copy changed. A check asks about and reports only what the change touches, and cached answers make reruns free. The exit code says what to do next:

| Exit code | Meaning for the agent |
|---|---|
| 0 | The gate passed; `consider` findings, and reviews from rules still being measured, may still be worth fixing |
| 1 | The gate failed: act on the findings listed |
| 2 | The run could not finish (no key, provider rejection, request budget); report it, don't treat it as a pass |

## Set up an agent in one command

`jevgate init --agent` writes an agent's hooks, which run [`jevgate hook`](#in-the-agents-loop-jevgate-hook), and a short text telling the agent how JevGate's findings work:

```sh
jevgate init --agent claude                    # Claude Code, for every repository you open
jevgate init --agent codex,gemini --project    # this repository's Codex and Gemini CLI
jevgate init --agent cursor --dry-run          # what would change, without writing
jevgate init --agent claude --remove           # take out what JevGate wrote
```

| Agent | Hooks, yours / with `--project` | Instructions, yours / with `--project` |
|---|---|---|
| `claude` (Claude Code) | `~/.claude/settings.json` / `.claude/settings.json` | `~/.claude/rules/jevgate.md` / `.claude/rules/jevgate.md` |
| `codex` | `~/.codex/hooks.json` / `.codex/hooks.json` | a block in `~/.codex/AGENTS.md` / `AGENTS.md` |
| `cursor` | `~/.cursor/hooks.json` / `.cursor/hooks.json` | none (your rules live in Cursor's settings) / `.cursor/rules/jevgate.mdc` |
| `gemini` (Gemini CLI) | `~/.gemini/settings.json` / `.gemini/settings.json` | a block in `~/.gemini/GEMINI.md` / `GEMINI.md` |
| `opencode` (OpenCode 1.x) | a plugin: `~/.config/opencode/plugins/jevgate.js` / `.opencode/plugins/jevgate.js` | a block in `~/.config/opencode/AGENTS.md` / `AGENTS.md` |

`CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `XDG_CONFIG_HOME` move your files as they move the agents'. With `--project`, the files go at the top of the Git work tree, for everyone who works in the repository.

It changes only JevGate's parts of each file:

- **Merged, not replaced.** A hook is JevGate's when it runs `jevgate hook`, wherever `jevgate` lives. Other tools' hooks keep their place, also in a group shared with JevGate's, and the rest of the file keeps its key order, indentation, line ends, byte-order mark and the text of its numbers and strings; a file written on one line is laid out over several, and a `hooks` object JevGate's hooks leave empty is taken out. Running it again changes nothing.
- **The text** sits between `<!-- jevgate:begin … -->` and `<!-- jevgate:end -->` in a file others write too, or is a file of JevGate's own where the agent reads a directory of rules. A file of that name that JevGate did not write is left alone.
- **All or nothing.** Every file is read before the first is written, so a settings file that is not plain JSON (Gemini CLI accepts comments) stops the run with nothing written.
- **`--remove`** takes out JevGate's hooks and text and nothing else, and deletes a file only when JevGate's parts were all it held.

Then it runs the `jevgate` on your `PATH`, which the agent will run, on an event it ignores, and warns when that one is missing or cannot answer the hooks: a JevGate before 0.27, or the unrelated npm package named `jevgate`. It also warns when JevGate would run twice: the [plugin](#the-claude-code-plugin) beside `init --agent claude`, or Cursor, which also runs Claude Code's hooks.

Hooks set up for your user check every Git repository you run the agent in, and upload what `jevgate check` would there; a repository's `jevgate.toml` bounds it. To choose the repositories, use `--project` in each.

What each agent runs:

- **Claude Code and Codex** run `jevgate hook || echo '{"systemMessage": …}'`. When `jevgate` is missing from the agent's `PATH` or older than 0.27, the hook says so and blocks nothing; a plain `jevgate hook` would block, since an older JevGate exits 2 on `hook` and both agents read exit 2 as a block (with JevGate 0.25.0, Claude Code dropped the prompt and Codex ended the turn). On Windows, Claude Code runs hooks in Git Bash, which Git for Windows installs, or PowerShell 7; Windows PowerShell 5.1 has no `||`.
- **Codex** runs new or changed hooks only once you trust them in `/hooks`. On macOS and Linux it starts hooks from a login shell, so `jevgate` must be on the `PATH` your login profile sets.
- **Gemini CLI** runs `jevgate hook; exit 0`. Antigravity CLI, which replaced Gemini CLI for its free, Pro and Ultra users, reads `GEMINI.md` but runs hooks only from its own files, which `init` does not write yet; there the instructions tell the agent that JevGate's hooks are not running. It denies on any exit but 0 and 1, so a missing `jevgate` would block every prompt; with exit 0 it shows the shell's error instead, in bash and in PowerShell. With `security.environmentVariableRedaction` on, hooks don't get `TYPESAFE_API_KEY`: use `jevgate auth login` or the repository's `.env`. JevGate's handlers are named `jevgate` (`/hooks disable jevgate`).
- **Cursor** runs `jevgate hook --agent cursor`. It shows a hook's messages to you only in its Hooks output channel (View > Output > Hooks), not in the chat, and its prompt hook takes no context, so the agent hears of a check that failed at the end of a turn after its next edit or session start.
- **OpenCode** has no command hooks, so it gets a plugin (OpenCode 1.x) that relays its events to `jevgate hook --agent opencode`. An edit's findings are appended to the tool's output, a blocked end of turn is sent back as the next prompt, and a failure is shown as a toast, never thrown into OpenCode. OpenCode 2 runs a different plugin API and does not load it yet.

These commands stay the same across versions, so Codex's and Gemini CLI's trust in them holds after an upgrade.

## The Claude Code plugin

This repository is also a Claude Code plugin marketplace. Its plugin bundles the hooks `init --agent claude` writes, the [MCP server](#as-an-mcp-server) and a skill, `/jevgate:findings`, on acting on findings:

```text
/plugin marketplace add Tech-Byte-Frontier/jevgate
/plugin install jevgate@jevgate
```

It runs the `jevgate` command, 0.27 or later, which you [install](install.md) separately. Use the plugin or `init --agent claude`, not both, or the hooks run twice. The plugin's version follows JevGate's releases.

## In the agent's loop: `jevgate hook`

`jevgate hook` runs as a hook of the agent, so the check happens without being asked for. It reads one hook event as JSON on stdin and prints one JSON reply:

- **When a session starts**, it tells the agent `JevGate's hooks run in this session: they check each edit and the end of each turn.` The instructions `init --agent` writes quote that line and ask an agent that never reads it to run `jevgate check --base HEAD` itself before finishing, or to say that JevGate did not check: an agent that reads those instructions but not JevGate's hooks (Codex before you trust them, OpenCode 2, Antigravity CLI reading `GEMINI.md`, Claude Code or Cursor reading a repository's `AGENTS.md`) would otherwise take the silence for a pass.
- **When a turn starts** (the person sends a prompt), it records a snapshot of the working tree under `.jevgate/turns/`: tracked and untracked files, not ignored ones or the directories a check never reads, such as `node_modules` and `target`; a file over 1 MiB is recorded as a stand-in naming its size and time, so Git never copies a dataset into its object store. The repository's own index and stash list are never touched.
- **After each edit**, it checks what the turn changed in the edited files, as `check --base` judges a change: the functions, tests and comments on lines changed since that snapshot, and a new file whole. It passes their findings to the agent as context, one line each: `- path:line level rule (fails the gate): why Right 87% of the time (23 labels). Next: step`, where the sentence after the why says how often findings of its rule and level were right on projects JevGate was never tuned on (`Not yet measured.` below 20 labels). A finding already given this turn is counted, not repeated. It never blocks after an edit.
- **When the turn ends**, it checks everything the turn changed. While findings fail the gate, it keeps the agent working with those findings as the reason: at most 3 times a turn, and not again when the agent changed nothing since the last time (for example because a finding is wrong and it said so). Findings that don't fail the gate are counted for the person, not sent to the agent. A finding already in a function the turn changes counts, as in a pull request check, so in a repository that has findings run `jevgate check` and `jevgate baseline` first: accepted findings never block.
- **Throughout the turn**, its checks read `jevgate.toml`, the [custom questions](custom-questions.md), `jevgate-baseline.json` and `jevgate: allow` comments as they were when the turn began, even when the agent leaves one unreadable, and judge a file the turn marked as generated code as they judged it then. A question the agent deletes, lowers to a note or breaks still asks its question until the turn ends; the snapshot of the turn's start keeps question files even when a `.gitignore` entry such as `/.jevgate/` hides them, since `jevgate check` asks those too. Accepting a finding or loosening the gate is the person's call, so what the agent writes there counts from the next turn: a finding its own edits accept still blocks, marked `(fails the gate; accepted this turn)`.
- **A changed file of code it did not judge** (marked as generated, larger than `max_file_bytes`, not parsed, a preview language's test file when tests are judged, or edited inside a submodule or nested clone, whose files the snapshots do not record), and each unit the turn touched that the parser could not read, are named to the agent after the edit and to the person when the turn ends, so silence about them is never a pass. A turn that was blocked and then passes is said to be fixed only when nothing it changed went unreviewed.

The hook also reports what a turn does to the checks around the code, JevGate's [guards](output.md#guards): new suppressions of other tools and new `jevgate: allow` comments, skipped, focused or removed tests, a rewritten test Jev reads as checking less than before, edits to `jevgate.toml`, a custom question file or the baseline, and text written to steer a reviewer. The agent hears of each once, after the edit that made it, and the person reads the turn's list when it ends. None of them blocks the agent: most suppressions and skips are legitimate, JevGate cannot see the other tools' findings, and the two questions behind guards are not yet measured on labeled projects.

A reply lists at most 10 findings and stays under 8,000 characters, which every agent reads whole; `.jevgate/latest.json` holds the rest, as after any check. Since those reports cover only what a turn changed, `jevgate baseline` after one needs `--merge`, which keeps what was accepted for everything else. Checks use the repository's `jevgate.toml` (rules, upload patterns, `fail_on`) and key like `jevgate check`, so what blocks the agent is what fails your gate: by default only the [rules and levels measured right](configuration.md#what-fails-the-check-by-default) on projects JevGate was never tuned on, which among the default rules are function-simplification reviews, outside the [preview languages](languages.md#support-levels): a Kotlin or Swift file's findings reach the agent, with how often they were right in that language (`Not yet measured in Kotlin.`), and never block it. The other findings reach the agent after its edits without blocking; `fail_on` makes them block too. With `uncertain` among a rule's levels, the units Jev left undecided that fail the check also block the end of a turn, each named with its open questions. An edit re-asks only the requests that hold what it changed, so the end of the turn is mostly answered from the cache.

The hook always exits 0 and speaks through its JSON: agents read exit 2 as "block" and exit 1 as a silent error, the opposite of `check`. A missing key, an HTTP 402, an outage, a check that runs past its time, another JevGate process holding the repository's session lock, or a directory outside Git never blocks the agent, and is always said: to the person as a message (in Cursor, in its Hooks output channel), and to the agent as context (at the next prompt, when it happened at the end of a turn; in Cursor, at its next edit). A turn whose end could not be checked is not let through for good: the next turn begins where it did, so the next end of a turn checks both. The exception is a turn that began with a `jevgate.toml` that does not load, which every check from its start would fail: the next turn begins where it ended, and the person is told its changes were not checked. When the provider times out, refuses connections, limits the rate or fails, no retry runs past the hook's time, and the hook's checks of the next 5 minutes use only cached answers, so an outage holds the agent once instead of for the whole budget at every event.

The snapshots are of the working tree, so a turn's changes include any another agent or person made in the same checkout meanwhile; give parallel agents their own worktrees.

The hook detects the agent from the event; `--agent` names it. It gives up after 10 s at a session or turn start, 30 s after an edit and 50 s at the end of a turn (`--timeout` sets one budget for every event); set the agent's own hook timeouts above those, as below.

| Agent | Where the hooks go | Events |
|---|---|---|
| Claude Code (also Devin CLI) | `.claude/settings.json`, or `~/.claude/settings.json` for every repository | `SessionStart`, `UserPromptSubmit`, `PostToolUse` (`Edit\|Write\|MultiEdit\|NotebookEdit`), `Stop` |
| Codex | `.codex/hooks.json` or `~/.codex/hooks.json`, same shape; approve new hooks in `/hooks` | `SessionStart`, `UserPromptSubmit`, `PostToolUse` (`apply_patch`), `Stop` |
| Gemini CLI | `hooks` in `.gemini/settings.json`; `timeout` is in milliseconds | `SessionStart`, `BeforeAgent`, `AfterTool` (`write_file\|replace`), `AfterAgent` |
| Cursor | `.cursor/hooks.json`, running `jevgate hook --agent cursor` | `sessionStart`, `beforeSubmitPrompt`, `postToolUse` (`Write`), `stop` |
| Copilot CLI, VS Code | the repository's `.claude/settings.json` (VS Code with `chat.useClaudeHooks`) | Claude Code's; VS Code sends its own names for its edit tools, which are not verified yet, so there only the end of a turn is sure to be checked |
| OpenCode | a plugin relaying `session.created`, `chat.message`, `tool.execute.after` and `session.idle` to `jevgate hook --agent opencode` | |

`jevgate init --agent` writes these for you, and the [plugin](#the-claude-code-plugin) carries the same hooks. By hand, for Claude Code, add them to `.claude/settings.json`. Each runs `jevgate hook || echo '{"systemMessage": …}'`, which says so instead of blocking when `jevgate` is missing or older than 0.27:

```json
{
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "jevgate hook || echo '{\"systemMessage\": \"JevGate could not check: jevgate hook is missing, older than 0.27 or failed. Install JevGate 0.27 or later where this agent finds it (https://tech-byte-frontier.github.io/jevgate/install.html). Nothing was blocked.\"}'", "timeout": 20}]}],
    "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "jevgate hook || echo '{\"systemMessage\": \"JevGate could not check: jevgate hook is missing, older than 0.27 or failed. Install JevGate 0.27 or later where this agent finds it (https://tech-byte-frontier.github.io/jevgate/install.html). Nothing was blocked.\"}'", "timeout": 20}]}],
    "PostToolUse": [{"matcher": "Edit|Write|MultiEdit|NotebookEdit",
                     "hooks": [{"type": "command", "command": "jevgate hook || echo '{\"systemMessage\": \"JevGate could not check: jevgate hook is missing, older than 0.27 or failed. Install JevGate 0.27 or later where this agent finds it (https://tech-byte-frontier.github.io/jevgate/install.html). Nothing was blocked.\"}'", "timeout": 40, "statusMessage": "JevGate is checking the edit"}]}],
    "Stop": [{"hooks": [{"type": "command", "command": "jevgate hook || echo '{\"systemMessage\": \"JevGate could not check: jevgate hook is missing, older than 0.27 or failed. Install JevGate 0.27 or later where this agent finds it (https://tech-byte-frontier.github.io/jevgate/install.html). Nothing was blocked.\"}'", "timeout": 60, "statusMessage": "JevGate is checking this turn"}]}]
  }
}
```

Cursor also runs the hooks in Claude Code's files (`~/.claude/settings.json`, `.claude/settings.json` and `.claude/settings.local.json`), and Copilot CLI those in the repository's `.claude/settings.json`: configure JevGate in one of them per agent. When two copies run anyway, the one that starts second while the first answers the same event replies with nothing, so the agent is told each finding and blocked once.

## As an MCP server

`jevgate mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server on stdin and stdout, so an agent can call JevGate as a tool instead of running a shell command. The Claude Code plugin registers it; otherwise, register it started in the repository:

```sh
claude mcp add jevgate -- jevgate mcp          # Claude Code
```

```json
{"mcpServers": {"jevgate": {"command": "jevgate", "args": ["mcp"]}}}
```

The second form is for clients configured with JSON, such as Cursor. The server offers three tools:

| Tool | What it does |
|---|---|
| `jevgate_check` | Runs `jevgate check` in the repository with `base`, `whole_files`, `paths`, `rules`, `include_tests`, `dry_run` or `verbose`, and returns the findings and verify items. An incomplete run (exit 2) is a tool error, never a pass |
| `jevgate_findings` | Reads the last report's findings and verify items, optionally under one `path`, without running anything |
| `jevgate_rules` | Lists every rule with the question it asks |

A check runs as a child process with the repository's `jevgate.toml` and key, so the tool reviews exactly what the command line would.

Each tool returns a structured result, described by its output schema, and text for clients that read only text: the agent text for `jevgate_check`, the same result as JSON for the others. Claude Code shows the model only the structured result, so it holds everything the text does:

- `headline`, `status`, `complete`, `exit_code` and `gate`: what the run found and whether the gate passed.
- `errors`: the run's errors, then one `Failed N: reason` line per reason files failed, such as a missing key or exhausted credit. `skipped`: why files were not judged, such as a syntax error in the file just edited. `left_out`: each unit the parser could not read in a file judged otherwise, `path:line unit: reason` (at most 20; `total_left_out` counts them), which was not reviewed.
- `findings`: those that fail the gate first, then new findings before accepted ones and reviews before considers, each by rank, with its location, message, next step, precision, probability and how the gate counted it (`gate`: `fails`, `measuring` or `advisory`), and its fingerprint as `id`, the id the baseline and the SARIF and GitLab reports use. Its message ends as the agent text's does, with how often findings of its rule and level were right on projects JevGate was never tuned on, and `precision` holds the counts (`{"right": 20, "labeled": 23}`; none for a note); `probability` is how sure the answer that set its level was, not how often such findings are right. The hook's one-line findings are written from the same fields. At most `max_findings` (default 20); `total_findings` counts them all.
- `verify`: the units Jev left undecided, those whose open questions lean most toward the concern first. At most `max_verify` (default 5; 0 leaves them out); `total_verify` counts them all.
- `guards`: what the change does to the checks around the code, as the JSON report records [guards](output.md#guards), for the person to look at. At most 20; `total_guards` counts them all.

A verify item is not a finding and never fails the gate: it is a unit whose questions Jev could not settle. It holds each such question as it was asked, the evidence the question named (such as `functions[0].source`, the unit's code at its location), and each likely answer with what it means and its probability. Read the code there and change it only if you agree it should change. This follows TypeSafe's confidence-routing pattern: a case the classifier leaves open goes to a stronger reasoner, with the question and its evidence.

A call that carries a progress token (`_meta.progressToken`) gets a progress notification when the check starts and after each stage that answers requests, such as `42 files: 340 requests answered, 290 from the cache, ~$0.0021 so far`, so a long first check never looks idle.

## Structured output

`--format json` prints the full report: every file, finding, raw answer and probability, and the gate. The same report is always written to `.jevgate/latest.json`, whatever the output format, so an agent can run the check once and read the details after. `jevgate check --help` explains its fields.

`jevgate rules --format json` lists every rule with the question it asks, so an agent can tell what a finding means without guessing.

## Watching while editing

`jevgate check --watch` re-checks the selected files after each save and prints one JSON report per line. It reads the configuration once, so it stops when `jevgate.toml`, the root `.gitignore` or a [custom question](custom-questions.md) file changes, and says to restart it. Alongside it, `jevgate serve` answers local tools, never browser pages, with read-only JSON:

| Path | What it returns |
|---|---|
| `/snapshot` | The full latest report |
| `/evidence` | Findings and context per file |
| `/context-requests` | Evidence a file still needs |
| `/changes?since=GENERATION` | What changed since a report generation |

## Documentation for agents

The opt-in documentation rules judge the instruction files agents load at the start of every session (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and Cursor, Copilot, Windsurf, Cline, Kiro, Junie and Roo Code rules): sections that only restate the manifest or generic advice, and text loaded in every session that applies to one directory. Their considers on instruction files fail the check by default, the one consider level that does (22 of 24 were right on projects JevGate was never tuned on). `jevgate check --rule documentation` also estimates the tokens each harness loads.

The conventions in those files can gate code too. `jevgate rules propose` drafts a [custom question](custom-questions.md#proposed-from-instruction-files) from each line that states a rule a reviewer could check in one function, test, comment, section, file or change, for a person to edit and accept; an accepted question fails the gate on code that breaks the rule, whether a person or an agent wrote it.
