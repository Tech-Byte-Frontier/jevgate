# Troubleshooting

## The run exits 2

Exit code 2 means the run could not finish, or the configuration or command line is invalid. The message says which; an outage never passes as a clean review.

**`No API key configured. Run jevgate auth login, set TYPESAFE_API_KEY, or provide --env-file PATH`**
: A check reads `TYPESAFE_API_KEY` from the environment, then `--env-file` or the repository's `.env`, then the key saved by `jevgate auth login`. `jevgate auth status` shows which one a check would use and verifies it. On GitHub Actions, pull requests from forks don't receive secrets: skip the job for them (`if: github.event.pull_request.head.repo.full_name == github.repository`).

**`Cannot find revision …; in CI, fetch it (for example fetch-depth: 0)`**, or **`… and HEAD share no history`**
: `--base` needs the history back to the fork point. Check out with `fetch-depth: 0`.

**`TypeSafe HTTP 403 (blocked by the provider's edge protection)`**
: The provider's firewall rejected a request because of what it contained, such as a test fixture holding an attack string. Find the file with `--dry-run --show-requests`, and exclude it with `upload_deny`; JevGate's own repository does this for its HTML report's escaping test.

**`Cannot connect to TypeSafe; request was not sent`**
: A network problem before anything was sent. Rerun; cached answers are kept.

**`Session API request budget exhausted; restart with an explicit larger --max-requests`**
: `max_requests` or `--max-requests` capped the run. Raise it, or check fewer files with `--base` or paths; `--dry-run` estimates what a run will ask.

**`Another JevGate session owns latest.json`**
: Another `check` or `--watch` is running in the same repository. Stop it first.

Rate limits, overload and server errors (HTTP 408, 429, 500, 502–504, 520–524, 529) are retried up to four attempts before the run gives up, and a timeout or dropped connection is retried once.

## Many files are uncertain

A file is `uncertain` when some of its answers stayed undecided after the follow-up questions. JevGate reports this instead of hiding it or counting the file as clear. `--verbose` lists each undecided unit and the question it stayed undecided on. It never fails the gate unless you ask for that with `--fail-on uncertain`.

## A finding is wrong

Accept it with `jevgate baseline`, and record why with `jevgate baseline mark wrong PATH:LINE`; `jevgate baseline stats` counts each rule's mistaken findings. Reporting it with the [wrong finding template](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml), with the finding from `.jevgate/latest.json` and a small piece of the code, is how the rules improve.

## A file is skipped

Skipped files are listed with the reason: generated, vendored or minified code, migrations, an unsupported language, or a path outside the upload patterns. `generated`, `tests` and the upload patterns in `jevgate.toml` change what is selected. A file larger than `max_file_bytes` is not skipped but reported as `needs-context`, never truncated.

## No colors, or escape codes in a log

Agent output is colored only on a terminal. `--color never` or `NO_COLOR` turns it off, and `--color always` or `CLICOLOR_FORCE` turns it on for pipes and logs.
