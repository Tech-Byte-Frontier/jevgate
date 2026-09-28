# Coding agents

JevGate's default output is written for coding agents as much as for people: ranked findings, each with a location, a probability and a next step, and nothing hidden when an answer stays undecided.

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

## In the agent's loop: `jevgate hook`

`jevgate hook` runs as a hook of the agent, so the check happens without being asked for. It reads one hook event as JSON on stdin and prints one JSON reply:

- **When a turn starts** (the person sends a prompt), it records a snapshot of the working tree under `.jevgate/turns/`: tracked and untracked files, not ignored ones. The repository's own index and stash list are never touched.
- **After each edit**, it checks the edited files against that snapshot and passes their findings to the agent as context, one line each: `- path:line level rule (fails the gate): why Next: step`. A finding already given this turn is counted, not repeated. It never blocks after an edit.
- **When the turn ends**, it checks every file the turn changed. While findings fail the gate, it keeps the agent working with those findings as the reason: at most 3 times a turn, and not again when the agent changed nothing since the last time (for example because a finding is wrong and it said so). Findings that don't fail the gate are counted for the person, not sent to the agent.

A reply lists at most 10 findings and stays under 8,000 characters, which every agent reads whole; `.jevgate/latest.json` holds the rest, as after any check. Since those reports cover only the files a turn changed, `jevgate baseline` after one needs `--merge`, which keeps what was accepted for the other files. Checks use the repository's `jevgate.toml` (rules, upload patterns, `fail_on`) and key like `jevgate check`, so what blocks the agent is what fails your gate. An edit re-asks only the requests that hold what it changed, so the end of the turn is mostly answered from the cache.

The hook always exits 0 and speaks through its JSON: agents read exit 2 as "block" and exit 1 as a silent error, the opposite of `check`. A missing key, an HTTP 402, an outage, a check that runs past its time, another JevGate process holding the repository's session lock, or a directory outside Git never blocks the agent, and is always said: to the person as a message, and to the agent as context (at the next prompt, when it happened at the end of a turn).

The snapshots are of the working tree, so a turn's changes include any another agent or person made in the same checkout meanwhile; give parallel agents their own worktrees.

The hook detects the agent from the event; `--agent` names it. It gives up after 10 s at a session or turn start, 30 s after an edit and 50 s at the end of a turn (`--timeout` sets one budget for every event); set the agent's own hook timeouts above those, as below.

| Agent | Where the hooks go | Events |
|---|---|---|
| Claude Code (also Devin CLI) | `.claude/settings.json`, or `~/.claude/settings.json` for every repository | `SessionStart`, `UserPromptSubmit`, `PostToolUse` (`Edit\|Write\|MultiEdit\|NotebookEdit`), `Stop` |
| Codex | `.codex/hooks.json` or `~/.codex/hooks.json`, same shape; approve new hooks in `/hooks` | `SessionStart`, `UserPromptSubmit`, `PostToolUse` (`apply_patch`), `Stop` |
| Gemini CLI | `hooks` in `.gemini/settings.json`; `timeout` is in milliseconds | `SessionStart`, `BeforeAgent`, `AfterTool` (`write_file\|replace`), `AfterAgent` |
| Cursor | `.cursor/hooks.json`, running `jevgate hook --agent cursor` | `beforeSubmitPrompt`, `postToolUse` (`Write`), `stop` |
| Copilot CLI, VS Code | the repository's `.claude/settings.json` (VS Code with `chat.useClaudeHooks`) | Claude Code's |
| OpenCode | a plugin relaying `session.created`, `chat.message`, `tool.execute.after` and `session.idle` to `jevgate hook --agent opencode` | |

For Claude Code:

```json
{
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "jevgate hook", "timeout": 20}]}],
    "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "jevgate hook", "timeout": 20}]}],
    "PostToolUse": [{"matcher": "Edit|Write|MultiEdit|NotebookEdit",
                     "hooks": [{"type": "command", "command": "jevgate hook", "timeout": 40}]}],
    "Stop": [{"hooks": [{"type": "command", "command": "jevgate hook", "timeout": 60}]}]
  }
}
```

Cursor also runs the hooks in Claude Code's files, and Copilot CLI those in the repository's `.claude/settings.json`: configure JevGate in one of them per agent, or it runs twice. Gemini CLI starts hooks without your shell's environment, so `TYPESAFE_API_KEY` may not reach them; `jevgate auth login` or the repository's `.env` works there.

## As an MCP server

`jevgate mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server on stdin and stdout, so an agent can call JevGate as a tool instead of running a shell command. Register it, started in the repository:

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
- `errors`: the run's errors, then one `Failed N: reason` line per reason files failed, such as a missing key or exhausted credit. `skipped`: why files were not judged, such as a syntax error in the file just edited.
- `findings`: new findings before accepted ones and reviews before considers, each by rank, with its location, message, next step and probability, and its fingerprint as `id`, the id the baseline and the SARIF and GitLab reports use. At most `max_findings` (default 20); `total_findings` counts them all.
- `verify`: the units Jev left undecided, highest concern first. At most `max_verify` (default 5; 0 leaves them out); `total_verify` counts them all.

A verify item is not a finding and never fails the gate: it is a question Jev could not settle about one unit. It holds the question as it was asked, the evidence the question named (such as `functions[0].source`, the unit's code at its location), and each likely answer with what it means and its probability. Read the code there and change it only if you agree it should change. This follows TypeSafe's confidence-routing pattern: a case the classifier leaves open goes to a stronger reasoner, with the question and its evidence.

A call that carries a progress token (`_meta.progressToken`) gets a progress notification when the check starts and after each stage that answers requests, such as `42 files: 340 requests answered, 290 from the cache, ~$0.0021 so far`, so a long first check never looks idle.

## Structured output

`--format json` prints the full report: every file, finding, raw answer and probability, and the gate. The same report is always written to `.jevgate/latest.json`, whatever the output format, so an agent can run the check once and read the details after. `jevgate check --help` explains its fields.

`jevgate rules --format json` lists every rule with the question it asks, so an agent can tell what a finding means without guessing.

## Watching while editing

`jevgate check --watch` re-checks the selected files after each save and prints one JSON report per line. Alongside it, `jevgate serve` answers local tools, never browser pages, with read-only JSON:

| Path | What it returns |
|---|---|
| `/snapshot` | The full latest report |
| `/evidence` | Findings and context per file |
| `/context-requests` | Evidence a file still needs |
| `/changes?since=GENERATION` | What changed since a report generation |

## Documentation for agents

The opt-in documentation rules judge the instruction files agents load at the start of every session (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and Cursor, Copilot, Windsurf, Cline, Kiro, Junie and Roo Code rules): sections that only restate the manifest or generic advice, and text loaded in every session that applies to one directory. Their considers on instruction files fail the check by default, the one consider level that does (22 of 24 were right on projects JevGate was never tuned on). `jevgate check --rule documentation` also estimates the tokens each harness loads.
