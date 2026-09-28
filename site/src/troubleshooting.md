# Troubleshooting

## The run exits 2

Exit code 2 means the run could not finish, or the configuration or command line is invalid. The message says which; an outage never passes as a clean review.

**`No API key configured. Run jevgate auth login, set TYPESAFE_API_KEY (or OPENROUTER_API_KEY, AI_GATEWAY_API_KEY), or provide --env-file PATH`**
: A check reads `TYPESAFE_API_KEY` from the environment, then `--env-file` (or `TYPESAFE_API_KEY` in the repository's `.env`), then the key saved by `jevgate auth login`, then `OPENROUTER_API_KEY` or `AI_GATEWAY_API_KEY` from the environment. A gateway's key in the repository's `.env` is read only with `--env-file .env`, since it is usually the application's own; the message says so when one is there. `jevgate auth status` shows which key a check would use and verifies it. On GitHub Actions, pull requests from forks don't receive secrets: skip the job for them (`if: github.event.pull_request.head.repo.full_name == github.repository`).

**`TYPESAFE_API_KEY environment variable: the key was issued by OpenRouter (it starts with sk-or-), not by TypeSafe`**
: A key goes only to the provider that issued it. Set it in the variable the message names, or save it with `jevgate auth login --provider openrouter`.

**`TypeSafe HTTP 400 (unknown model; check the model name)`**, or **`OpenRouter HTTP 404 (not found; check the model name)`**
: A model name is sent as written, and each provider has its own: `jev-1.13.0`, `jev-latest` or `jev-preview` on TypeSafe (which refuses `jev-1.13`, though its docs use it), `typesafe/jev-1.13` on OpenRouter, `typesafe-ai/jev` on Vercel AI Gateway. A `model` in `jevgate.toml` written for one provider needs `--model` with another provider's key.

**`Cannot find revision …; in CI, fetch it (for example fetch-depth: 0)`**, or **`… and HEAD share no history`**
: `--base` needs the history back to the fork point. Check out with `fetch-depth: 0`.

**`TypeSafe HTTP 403 (blocked by the provider's edge protection)`**
: The provider's firewall rejected a request because of what it contained, such as a test fixture holding an attack string. Find the file with `--dry-run --show-requests`, and exclude it with `upload_deny`; JevGate's own repository does this for its HTML report's escaping test.

**`Cannot connect to TypeSafe; request was not sent`**
: A network problem before anything was sent. Rerun; cached answers are kept.

**`OpenRouter HTTP 503; gave up after 6 attempts`**, or **`TypeSafe HTTP 503; gave up after 6 attempts`**
: The provider stayed overloaded through six attempts and 23 seconds or more of pauses. On 2026-09-28 TypeSafe answered 503 to about two attempts in three for at least ten minutes, directly and through OpenRouter alike. The run exits 2 and keeps the answers it received: rerun later, and only the unanswered units are asked.

**`TypeSafe HTTP 402 (credits exhausted; add credits or turn on auto-refill at https://console.typesafe.ai)`**
: The account's prepaid credits ran out. The run stops sending requests and exits 2; the answers it received are kept in the cache. Add credits and rerun: only the unanswered units are asked.

**`TypeSafe HTTP 422 (invalid request: body.questions.q1.criteria missing)`**
: TypeSafe refused a request as malformed. The message names each invalid field and its error type, never the text TypeSafe sends with it, which can quote your source. It is a JevGate bug: please [report it](https://github.com/Tech-Byte-Frontier/jevgate/issues/new) with the request id.

A provider error ends with the provider's request id when it sent one (`; request id req_…`); quote it to the provider's support. The report also keeps the id of the request behind each answer (`files[].judgments[].request_id`).

**`Session API request budget exhausted; restart with an explicit larger --max-requests`**
: `max_requests` or `--max-requests` capped the run. Raise it, or check fewer files with `--base` or paths; `--dry-run` estimates what a run will ask.

**`Another JevGate session owns latest.json`**
: Another `check` or `--watch` is running in the same repository. Stop it first.

Rate limits, overload and server errors (HTTP 408, 429, 500, 502–504, 520–524, 529) are retried up to six attempts before the run gives up, pausing 1, 2, 4, 8 and 8 seconds (each up to a quarter longer, so requests spread out), or as long as the provider asks when that is longer (`retry-after-ms`, or `Retry-After` in seconds or as a date, at most 30 seconds). A pause holds every request of the run. An attempt that has not answered within 20 seconds, or whose connection drops, is retried once, and a connection that fails before anything is sent is tried four times. Requests start at least 50 ms apart, within TypeSafe's limit of 1,200 a minute, and at most 6 are sent at once with a TypeSafe key, 3 with an OpenRouter or Vercel AI Gateway key (`--concurrency` or `concurrency` sets it).

## The agent hook says it could not check

`jevgate hook` never blocks the agent when a check cannot finish; it says why, as `JevGate could not check this turn: REASON. Nothing was blocked.` The reasons are the ones above, and a few of its own:

**`… is not in a Git repository (or Git cannot run), so JevGate cannot tell what a turn changed; it says so once a session`**
: The hook compares snapshots of the working tree, which needs Git on the `PATH`. Run `git init`, or leave the hook out of that agent's settings for directories outside Git. Hooks set up for your user run in every directory an agent opens, so the hook says this at a session's first event there, and then nothing: it keeps an empty mark in a directory of its own in the system's temporary directory (`jevgate-hook-<user>`, which only you can open, and which it does not use when it is a link) and writes nothing else outside Git.

**`another JevGate process in this repository (a check, --watch or another hook) held its session lock`**
: The hook waits up to 10 s for another JevGate process in the same repository, such as a `check --watch`, then lets the agent go on. Stop the watch while the agent works, or rely on the hook instead.

**`the check did not finish within 30 s`**
: The turn changed many files. The answers received so far are cached, so the next check continues from them. If this repeats, raise `--timeout`, and the agent's own hook timeout above it.

**`the provider did not answer within 30 s`**, or a provider's failure such as **`TypeSafe HTTP 503`** or **`Cannot connect to TypeSafe`**
: The provider timed out, refused the connection, limited the rate or failed. No retry it asks for runs past the hook's time, and for the next 5 minutes the hook's checks use only cached answers, saying **`the provider failed a few minutes ago (…), so JevGate asks it again in N minutes`** when those do not cover an edit. An outage then holds the agent once, not at every edit. Raising `--timeout` does not help here. A turn whose end could not be checked is checked with the next one.

**`JevGate did not check this turn: it has no snapshot of the turn's start`**
: The hook that runs when a prompt is sent (`UserPromptSubmit`, `BeforeAgent`, `beforeSubmitPrompt`) is not configured. The end of the turn records a snapshot, so the next turn is checked.

**`JevGate could not check: jevgate hook is missing, older than 0.27 or failed`**, or in Gemini CLI **`jevgate: command not found`** or **`unrecognized subcommand 'hook'`**
: The agent found no `jevgate` on the `PATH` it starts hooks with, or an older one. [Install](install.md) JevGate 0.27 or later where the agent finds it; Codex starts hooks from a login shell, so on macOS and Linux its `PATH` comes from your login profile. `jevgate init --agent` runs the `jevgate` on your own `PATH` and warns when it cannot answer the hooks.

**A Claude Code hook error about `||` on Windows**
: Claude Code runs hooks in Git Bash, or in PowerShell when Git Bash is missing, and Windows PowerShell 5.1 has no `||`. Install Git for Windows, which brings Git Bash, or PowerShell 7.

Nothing at all appears: check that the agent runs the hook (Claude Code's `/hooks`, Codex's `/hooks`, which also approves new or changed hooks, Gemini CLI's `/hooks panel`, Cursor's Hooks output channel), and that `jevgate` is on the `PATH` the agent starts hooks with.

## `jevgate init --agent` stops

It reads every file before writing any, so when it stops, nothing was written.

**`… it is not plain JSON`**
: The agent's settings file holds something JSON does not allow, such as comments, which Gemini CLI accepts. JevGate does not rewrite a file it cannot read whole: take them out, or add the hooks by hand from [coding agents](coding-agents.md#set-up-an-agent-in-one-command).

**`… it was not written by JevGate, so it is left alone`**
: A rules file or plugin of JevGate's name (`.claude/rules/jevgate.md`, `.cursor/rules/jevgate.mdc`, `jevgate.js`) is someone else's. Move it away, then run it again.

**`… a symlink leads it outside the repository`**
: With `--project`, a file or directory it writes, such as `AGENTS.md` or `.codex`, links outside the repository. JevGate writes a repository's files only inside it.

**`… a JevGate begin marker without its end marker`**
: The block between `<!-- jevgate:begin` and `<!-- jevgate:end -->` lost one of its lines. Restore it, or delete what is left of the block.

## The agent accepted a finding and the hook still blocks

Within a turn, the agent hook reads `jevgate.toml`, the custom questions in it and in `.jevgate/questions/`, `jevgate-baseline.json` and `jevgate: allow` comments as they were when the turn began, so an agent cannot unblock itself by accepting its own findings or loosening the gate, which includes deleting a custom question or lowering it to a note. Such a finding is marked `(fails the gate; accepted this turn)`, the person is told of the edit when the turn ends, and the edit counts from the next turn. If the finding is wrong, keep the accepting edit; if not, remove it. The same holds for a `jevgate.toml` or baseline the turn leaves unreadable (the person reads that it `does not parse`), and for a generated-code marker (`// @generated`, `DO NOT EDIT`) added to a file JevGate judged when the turn began: the file is judged this turn, and a guard says it is skipped from now on.
## A custom question is not asked, or is ignored by Git

**`jevgate: custom/<id> was not asked: it needs --base`** (or `--include-tests`)
: A `hunk` question asks about what changed since a revision, and a `test` question about tests, which are judged only with `include_tests`. Run with the flag it names.

**`jevgate: Git ignores .jevgate/questions (…), so its questions never reach a commit or CI`**
: A `.gitignore` entry such as `/.jevgate/` hides the question files, and CI would never ask them. Replace it with `/.jevgate/*` and `!/.jevgate/questions/`; JevGate's own `.jevgate/.gitignore` already keeps `questions/` tracked.

**`custom/<id> left N units unasked`**
: A question asks at most 2,000 units a run. Narrow it with `paths`, or check the changed files with `--base`.

**`Unknown rule or group: <id>; a custom question is named custom/<id>`**
: Custom questions are named by their rule ID, or all together as `custom`.

## `jevgate rules test` fails or exits 2

**`wrong  passing 2  yes 0.89  src/api/audit.ts: `auditOrder` (a check reports it at 0.80 or more)`**
: The question finds code its author says keeps the rule. Read the example against the guidance: it usually names the case as a violation. Guidance that called "an object built from" a request body a violation made `audit.log(redact(req.body))` one at 0.89; saying that a redacted body is fine separates them. If the example is wrong instead, move it to `failing`.

**`wrong  failing 1  yes 0.70 … (a check misses it below 0.80)`**
: The question does not see this violation clearly enough to report it. Name the case in `guidance`, or lower the `threshold` if its passing examples stay well below it; a new threshold is judged from the cached answers.

**`(within 0.10 of 0.80)`**
: Answers of one model move up to about 0.09 between asks, so this example may flip on the next model or a `--refresh`. Move it further from the threshold, or sharpen the guidance.

**`failing example 1: its path … is outside the question's paths; set `path` …`**
: A check never asks the question there. Set `path` to a file the question's `paths` match: the example is asked as that file.

**`… is outside upload_allow or inside upload_deny; allow it, or write the example as `code``**
: Example files are uploaded, so the upload patterns apply. Add `".jevgate/questions/**"` (or wherever the examples are) to `upload_allow`.

**`it holds no function`** (or no test case, no heading section with text, no changed line)
: The example has no unit of the question's kind. A `test` example needs a test file's path where its language decides by name (`tests/test_api.py`), and a `hunk` example's added lines start with `+`.

**`No current cached response; rerun without --cache-only to allow an API request`**
: The model, the question or the example changed since the examples were last asked. Run without `--cache-only`; `--dry-run` shows what that costs.

## Many files are uncertain

A file is `uncertain` when some of its answers stayed undecided after the follow-up questions. JevGate reports this instead of hiding it or counting the file as clear. `--verbose` lists each undecided unit and the question it stayed undecided on; the JSON report also quotes each such question as it was asked, with what each answer means and the probabilities Jev gave them, and the MCP tools hand them to an agent as verify items. It never fails the gate unless you ask for that with `--fail-on uncertain`.

A unit whose request carried a comment or string written to steer a reviewer is uncertain too, listed as `text written to steer a reviewer (line N)`, since its answers may be the text's rather than the code's. Remove the text and the unit is judged again.

A custom question with a high threshold leaves more units undecided, since a unit is clear only at one minus the threshold or below: the gallery's `todo-without-owner`, at 0.95, leaves most comments undecided. Lower the threshold, or narrow the question with `paths`, if the listing is in the way.

## A review did not fail the check

By default only the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on fail the check; `jevgate rules` shows which, and how often each rule's reviews and considers were right ([accuracy](accuracy.md) says how that is measured). The other findings are reported, and the output says their rules are still being measured. A finding in a [preview language](languages.md#support-levels), such as Kotlin or Swift, never fails the default gate, whatever its rule and level, and the output says the language is in preview. To fail on them, set a level: `--fail-on review` or `fail_on = ["review"]` for every rule, or `--fail-on maintainability/shared-logic=review` for one.

## A finding is wrong

Each rule's page in the [rules reference](reference/rules.md) shows findings it got wrong and why, which may match yours. Accept it with `jevgate baseline`, and record why with `jevgate baseline mark wrong PATH:LINE`; `jevgate baseline stats` counts each rule's mistaken findings. Reporting it with the [wrong finding template](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml), with the finding from `.jevgate/latest.json` and a small piece of the code, is how the rules improve.

## A `--base` check leaves out a finding

With `--base`, only what the change touches is asked about and reported: units on changed lines, copies where either copy changed, a file's outline when the change adds members, and documents naming a path it removed. A finding elsewhere in a changed file comes back with `--whole-files`, or in a check without `--base`. The report's `scope` says which a check used. The agent hook judges a turn the same way, from the snapshot taken when the turn began, so a review elsewhere in a file the agent edits neither reaches the agent nor blocks it. One in a function the turn changes does, even when it was there before the turn, as it would in a pull request check: to hold the hook to what agents add, accept what the repository already has first with `jevgate check`, then `jevgate baseline`.

## A file is skipped

Skipped files are listed with the reason: generated, vendored or minified code, migrations, an unsupported language, code the parser could not read with nothing else left to judge, a parser that did not finish within 10 seconds, Bend 1 code (JevGate reads Bend 2), or a path outside the upload patterns. A file the parser read in part is judged for the units it read, and the ones left out are listed after the findings (all of them with `--verbose`) and in the report's `left_out`; its outline is asked only when 90% of its lines parsed. Grammars miss some valid code, so a left-out unit is most often correct code the parser cannot read yet: [generic support](languages.md#generic-support) lists the gaps found in the preview languages' grammars. Test files of the preview languages are not skipped but not judged yet, and the agent text counts them after the skipped files. `generated`, `tests` and the upload patterns in `jevgate.toml` change what is selected. A file whose syntax nests more than 1,000 levels deep is not skipped but fails the run (exit 2): mark it generated or deny its upload in `jevgate.toml`, or nest it less. A file larger than `max_file_bytes` is not skipped but reported as `needs-context`, never truncated, and so is a unit whose request the provider refuses as beyond the model's context.

## No colors, or escape codes in a log

Agent output is colored only on a terminal. `--color never` or `NO_COLOR` turns it off, and `--color always` or `CLICOLOR_FORCE` turns it on for pipes and logs.
