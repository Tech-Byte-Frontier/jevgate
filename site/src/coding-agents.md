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
