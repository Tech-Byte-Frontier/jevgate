# Coding agents

JevGate's default output is written for coding agents as much as for people: ranked findings, each with a location, a probability and a next step, and nothing hidden when an answer stays undecided.

## Check before finishing

Ask the agent to review its own change before it reports back, for example in `AGENTS.md` or `CLAUDE.md`:

```markdown
Before finishing, run `jevgate check --base origin/main`. Fix each `review` finding;
for a `consider`, fix it or say why the code should stay as it is.
```

`--base` limits the review to the files changed since that revision, plus uncommitted and untracked files, so a check costs only what the change touches, and cached answers make reruns free. The exit code says what to do next:

| Exit code | Meaning for the agent |
|---|---|
| 0 | The gate passed; `consider` findings may still be worth a look |
| 1 | The gate failed: act on the findings listed |
| 2 | The run could not finish (no key, provider rejection, request budget); report it, don't treat it as a pass |

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
| `jevgate_check` | Runs `jevgate check` in the repository with `base`, `paths`, `rules`, `include_tests`, `dry_run` or `verbose`, and returns the ranked findings. An incomplete run (exit 2) is a tool error, never a pass |
| `jevgate_findings` | Reads the last report's findings, optionally under one path, without running anything |
| `jevgate_rules` | Lists every rule with the question it asks |

A check runs as a child process with the repository's `jevgate.toml` and key, so the tool reviews exactly what the command line would.

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

The opt-in documentation rules judge the instruction files agents load at the start of every session (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and Cursor, Copilot, Windsurf, Cline, Kiro, Junie and Roo Code rules): sections that only restate the manifest or generic advice, and text loaded in every session that applies to one directory. `jevgate check --rule documentation` also estimates the tokens each harness loads.
