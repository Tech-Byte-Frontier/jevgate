# Continuous integration

A pull request review on GitHub Actions, with the [JevGate action](https://github.com/Tech-Byte-Frontier/jevgate-action):

```yaml
name: JevGate
on: pull_request
permissions:
  contents: read
jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0 # --base compares with the fork point
      - uses: Tech-Byte-Frontier/jevgate-action@v1
        with:
          api-key: ${{ secrets.TYPESAFE_API_KEY }}
          version: 0.25.0
```

The action installs a checked release binary, keeps `.jevgate/cache` in the Actions cache and runs `jevgate check --base <pull request base> --format github`; `args` passes more flags, such as `--rule security`. It runs on Linux, macOS and Windows runners. For an OpenRouter or Vercel AI Gateway key, leave `api-key` out and set the key's variable in the step's `env`, such as `OPENROUTER_API_KEY: ${{ secrets.OPENROUTER_API_KEY }}`, with `version` 0.26.0 or later; jevgate-action 1.2 adds `api-key-kind: openrouter` or `api-key-kind: vercel` for the same.

`--format github` annotates the changed lines with each finding. A finding that fails the gate is an error; the others are warnings, and a warning whose rule and level are still being measured says so, with how often such findings were right. A Markdown table goes to the job summary, and the usual text goes to the log. The full JSON report is always at `.jevgate/latest.json` if you want to keep it as an artifact.

- **Only what changed:** `--base` reviews what changed since the fork point with that revision, as a pull request diff shows it, plus uncommitted and untracked files. It asks about and reports only what the change touches: functions, tests, comments and values on changed lines, copies where either copy changed, a file's outline when the change adds members to it, and a document when a section names a path the change deleted or renamed. A new file is judged whole. On the last commits of 118 corpus projects (90 open-source, 28 of the maintainer's own private repositories), this took the findings off the changed lines from 54% to 6% and halved the first-pass requests. `--whole-files` judges every unit of each changed file instead, as `--base` did before 0.26. It needs the history, so check out with `fetch-depth: 0`. When no supported file changed, the run passes without any request.
- **Cache:** answers are stored under a hash of the exact request: source, questions and model. Restoring an older cache is always safe, and unchanged code costs nothing on the next run. A gateway's model names are aliases, so with an OpenRouter or Vercel AI Gateway key answers expire after `cache_ttl_secs` (an hour by default); raise it to reuse answers across runs further apart, at the price of noticing a new model version later.
- **Advisory or blocking:** by default only the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on fail the check ([what fails by default](configuration.md#what-fails-the-check-by-default)); the other findings are warnings. On the last commits of those 118 projects, a pull request check with the defaults fails 9 of them, all on function-simplification reviews, where 0.25.0 failed 18; 8 of the 9 are the maintainer's own repositories. `fail_on = ["review"]` in `jevgate.toml` or `--fail-on review` fails on every review, and `fail_on = ["none"]` or `--fail-on none` reports findings without failing. A run that could not finish (missing key, provider rejection, request budget reached) still exits 2, so an outage never passes as a clean review.
- **A policy the change cannot edit:** a pull request can edit `jevgate.toml`. To apply the reviewed policy of the base branch instead, read it with `--config`:

  ```sh
  git show "$BASE_SHA:jevgate.toml" > "$RUNNER_TEMP/jevgate.toml"
  jevgate check --config "$RUNNER_TEMP/jevgate.toml" --base "$BASE_SHA" --format github
  ```

- **Forks:** GitHub withholds secrets from pull requests opened from forks, so there the run exits 2 with "No API key configured". Skip the job for forks, or run it only on branches of the repository.
- **Budgets:** `max_requests` caps the API attempts of one run. Reaching it leaves the run incomplete instead of passing on partial evidence. `--dry-run` counts the planned requests the cache already answers, so its estimate covers only what the cache lacks; follow-ups depend on answers and are not counted.
- **Transient failures:** rate limits, overload and server or edge errors (HTTP 408, 429, 500, 502–504, 520–524, 529) are retried up to six attempts, with pauses of 1 to 8 seconds; an attempt that has not answered in 20 seconds, or whose connection drops, is retried once, since the first send may have run. A provider that fails every attempt ends a run of 100 requests incomplete after 8 minutes or more (16 with a gateway's key, which sends 3 requests at once).
- **Report-only paths:** give tooling its own level with `[[scope]]` (below), so scripts are reported while product code gates.

Before each commit, with [pre-commit](https://pre-commit.com), review what is staged:

```yaml
repos:
  - repo: https://github.com/Tech-Byte-Frontier/jevgate
    rev: v0.25.0
    hooks:
      - id: jevgate-system   # the jevgate on PATH; `jevgate` builds it with Rust instead
```

On GitLab, a merge request pipeline can show the findings in the merge request with a Code Quality report. Set `TYPESAFE_API_KEY` as a masked CI/CD variable (or `OPENROUTER_API_KEY` or `AI_GATEWAY_API_KEY` for a gateway's key):

```yaml
jevgate:
  image: buildpack-deps:bookworm-scm   # any image with git, curl and tar
  variables:
    GIT_DEPTH: 0                       # --base compares with the fork point
  cache:
    key: jevgate-answers
    paths: [.jevgate/cache]
  script:
    - curl -fsSL https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/install.sh | sh
    - ~/.local/bin/jevgate check --base "$CI_MERGE_REQUEST_DIFF_BASE_SHA" --format gitlab > gl-code-quality-report.json
  artifacts:
    when: always
    reports:
      codequality: gl-code-quality-report.json
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
```

Other CI systems work the same way: install with `install.sh` or `cargo binstall`, set `TYPESAFE_API_KEY` (or a gateway's variable), keep `.jevgate/cache` between runs, and read the exit code or the JSON report.
