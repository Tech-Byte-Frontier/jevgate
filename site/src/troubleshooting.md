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

**`… is not in a Git repository, so JevGate cannot tell what a turn changed`**
: The hook compares snapshots of the working tree, which needs Git. Run `git init`, or leave the hook out of that agent's settings for directories outside Git.

**`another JevGate process in this repository (a check, --watch or another hook) held its session lock`**
: The hook waits up to 10 s for another JevGate process in the same repository, such as a `check --watch`, then lets the agent go on. Stop the watch while the agent works, or rely on the hook instead.

**`the check did not finish within 30 s`**
: The provider was slow, or the turn changed many files. The answers received so far are cached, so the next check continues from them. Raise `--timeout`, and the agent's own hook timeout above it.

**`JevGate did not check this turn: it has no snapshot of the turn's start`**
: The hook that runs when a prompt is sent (`UserPromptSubmit`, `BeforeAgent`, `beforeSubmitPrompt`) is not configured. The end of the turn records a snapshot, so the next turn is checked.

Nothing at all appears: check that the agent runs the hook (Claude Code's `/hooks`, Codex's `/hooks`, which also approves new or changed hooks, Gemini CLI's `/hooks panel`, Cursor's Hooks output channel), and that `jevgate` is on the `PATH` the agent starts hooks with.

## Many files are uncertain

A file is `uncertain` when some of its answers stayed undecided after the follow-up questions. JevGate reports this instead of hiding it or counting the file as clear. `--verbose` lists each undecided unit and the question it stayed undecided on. It never fails the gate unless you ask for that with `--fail-on uncertain`.

## A review did not fail the check

By default only the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on fail the check; `jevgate rules` shows which, and how often each rule's reviews and considers were right. The other findings are reported, and the output says their rules are still being measured. To fail on them, set a level: `--fail-on review` or `fail_on = ["review"]` for every rule, or `--fail-on maintainability/shared-logic=review` for one.

## A finding is wrong

Accept it with `jevgate baseline`, and record why with `jevgate baseline mark wrong PATH:LINE`; `jevgate baseline stats` counts each rule's mistaken findings. Reporting it with the [wrong finding template](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml), with the finding from `.jevgate/latest.json` and a small piece of the code, is how the rules improve.

## A `--base` check leaves out a finding

With `--base`, only what the change touches is asked about and reported: units on changed lines, copies where either copy changed, a file's outline when the change adds members, and documents naming a path it removed. A finding elsewhere in a changed file comes back with `--whole-files`, or in a check without `--base`. The report's `scope` says which a check used.

## A file is skipped

Skipped files are listed with the reason: generated, vendored or minified code, migrations, an unsupported language, syntax errors, a parser that did not finish within 10 seconds, Bend 1 code (JevGate reads Bend 2), or a path outside the upload patterns. `generated`, `tests` and the upload patterns in `jevgate.toml` change what is selected. A file larger than `max_file_bytes` is not skipped but reported as `needs-context`, never truncated, and so is a unit whose request the provider refuses as beyond the model's context.

## No colors, or escape codes in a log

Agent output is colored only on a terminal. `--color never` or `NO_COLOR` turns it off, and `--color always` or `CLICOLOR_FORCE` turns it on for pipes and logs.
