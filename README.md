# JevGate

[![crates.io](https://img.shields.io/crates/v/jevgate.svg)](https://crates.io/crates/jevgate)
[![CI](https://github.com/Tech-Byte-Frontier/jevgate/actions/workflows/ci.yml/badge.svg)](https://github.com/Tech-Byte-Frontier/jevgate/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/jevgate.svg)](#license)
[![MSRV 1.90](https://img.shields.io/badge/rustc-1.90+-informational.svg)](https://www.rust-lang.org)

**JevGate is a code-review gate. It asks small, precise questions about your code and turns the answers into findings you can act on.**

JevGate parses your repository locally and builds small units of evidence: a function, a file outline, a pair of copies, a test, a documentation section. It asks [TypeSafe Jev](https://docs.typesafe.ai) short, typed questions about each one. Code, not a chat model, combines the answers into a verdict. Each finding has a location, a probability and a concrete next step, so an agent or CI job can act on it and a person can check it quickly.

```text
JevGate: consider · gate passed · 42 files · 118 API requests · 263410 input tokens · ~$0.0111

Consider (2):
  src/billing/invoices.ts:88 [maintainability/shared-logic] `createInvoice` and `createReceipt`
    perform the same steps for the same purpose (0.93). Differences: `invoices`→`receipts`.
    → Move the shared steps into one implementation
  src/api/search.py:41 [security/injection] `search_orders` places its parameters into a
    database query without binding, escaping or checking them; a caller passing outside
    input would make it exploitable (0.88).
    → Pass the values as bound query parameters
```

- [What it finds](#what-it-finds)
- [Supported languages and frameworks](#supported-languages-and-frameworks)
- [Install](#install)
- [Quick start](#quick-start)
- [Continuous integration](#continuous-integration)
- [Configuration](#configuration)
- [Output and exit codes](#output-and-exit-codes)
- [How it works](#how-it-works)
- [Privacy and cost](#privacy-and-cost)
- [Limits](#limits)

## What it finds

**Maintainability** (on by default)

| Rule | Example finding |
|---|---|
| File organization | This file holds several features that would be easier to find apart; the upload helpers would be most useful as their own module. Test files are judged too, at most as a consider. |
| Function simplification | `sync_accounts` mixes separate jobs in long blocks; lines 40–71 would be most useful as their own function. |
| Shared logic | `createInvoice` and `createReceipt` perform the same steps; one shared implementation would serve both. |
| Hardcoded values | Module constants fix a value that differs between deployments; `apply_discount` special-cases one specific customer. |

**Tests** (with `--include-tests`; file organization judges test files without it)

| Rule | Example finding |
|---|---|
| Test value | `test_total` computes its expected value with the logic it tests. |
| Test redundancy | Three tests of `parse_date` check the same behavior; one parameterized test could hold them. |

**Security** (opt-in with `--rule security`; each finding names a CWE)

| Rule | Covers |
|---|---|
| Injection | Variables reaching SQL, shell commands, evaluated code, HTML, file paths or outbound URLs without binding, escaping or checks |
| Sensitive data | Passwords, tokens or personal data written to logs; internal error details sent to clients, judged per error message and once per error handler (`app.onError`, `setErrorHandler`, Express error middleware, Flask and FastAPI handlers, NestJS filters, axum `IntoResponse` and actix-web `ResponseError` for error types) |
| Unsafe settings | Certificate checks turned off, weak password hashing, non-cryptographic random secrets, permissive CORS, session cookies without `Secure`/`HttpOnly` |
| Access control | SQL row-level policies that let every user reach other users' rows or trust `user_metadata`; SECURITY DEFINER functions without a fixed `search_path` or a caller check; grants that open writes to every user. SpacetimeDB modules (TypeScript and Rust, any kind of application): public tables of users' private data, views that return other users' rows, reducers that change rows their arguments choose or admin-only settings without checking the caller, and scheduled reducers clients can call in 1.x |
| Workflows | GitHub Actions `run` scripts that execute text outside people write (`${{ github.event.pull_request.title }}`); `pull_request_target` or `workflow_run` jobs that run pull request code with secrets |

**Documentation** (opt-in with `--rule documentation`)

| Rule | Example finding |
|---|---|
| Agent context | A section of `CLAUDE.md` only lists the scripts `package.json` already shows, and six harnesses load it at the start of every session. |
| Large docs | `docs/operations/runbook.md` holds several unrelated subjects; `docs/plans/v0.2-plan.md` mainly records finished work. |
| Staleness | `docs/plans/v0.4-auth.md` is a plan whose work is finished: the repository has a release tag v0.4.0, and 6 paths it names were since removed. |
| Duplication | Section `Release Workflow` of `CLAUDE.md` states everything section `Release` of `README.md` states. |

The documentation rules read the instruction files that coding agents load (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and Claude, Cursor, Copilot, Windsurf and Cline rules), even when hidden or gitignored. Each section is asked whether it only restates the stack, the manifest's commands, generic advice or a configured linter's rules, and whether text loaded in every session applies to only one directory. Project Markdown of 300 or more lines is judged from its headings alone. Code finds staleness and duplication candidates: named paths or scripts that no longer exist, release tags, deleted files, and shared wording. Jev then judges each candidate. Documentation findings are at most `consider`. The run also estimates the tokens each harness loads at session start; these estimates are evidence and never fail the gate.

`jevgate rules` prints every rule with its question and default.

## Supported languages and frameworks

✅ judged · ➖ not applicable

| Language or file | Extensions | Maintainability | Tests | Security | Documentation |
|---|---|:---:|:---:|:---:|:---:|
| Rust | `.rs` | ✅ | ✅ `#[test]`, `#[cfg(test)]` | ✅ | ➖ |
| Python | `.py` | ✅ | ✅ pytest, unittest | ✅ | ➖ |
| JavaScript | `.js` `.jsx` `.mjs` `.cjs` | ✅ | ✅ `describe`/`it`/`test` | ✅ | ➖ |
| TypeScript | `.ts` `.tsx` `.mts` `.cts` | ✅ | ✅ `describe`/`it`/`test` | ✅ | ➖ |
| Go | `.go` | ✅ | ✅ `Test…(t *testing.T)` | ✅ | ➖ |
| Astro, Vue, Svelte | `.astro` `.vue` `.svelte` | ✅ scripts only | ➖ | ✅ scripts only | ➖ |
| SQL (PostgreSQL, Supabase) | `.sql` | ➖ | ➖ | ✅ access control | ➖ |
| GitHub Actions | `.github/workflows/*.yml` | ➖ | ➖ | ✅ workflows | ➖ |
| Markdown | `README.md`, `docs/**`, agent instruction files | ➖ | ➖ | ➖ | ✅ |

| Framework or platform | What JevGate understands |
|---|---|
| Hono, Express, Fastify, Koa | Route handlers written inline (`app.post('/pages', async (c) => …)`); error handlers (`app.onError`, `setErrorHandler`, four-parameter Express middleware) |
| NestJS | Exception filters (`@Catch`) |
| Flask, FastAPI | Error handlers (`@app.errorhandler`, `@app.exception_handler`) |
| axum, actix-web, Rocket | Error responses (`IntoResponse` or `ResponseError` for an error type, `#[catch]`) |
| Supabase and PostgreSQL | Row-level security policies, `SECURITY DEFINER` functions, grants, and the claims an access token hook sets |
| SpacetimeDB (TypeScript and Rust modules) | Public tables, views and reducers, checked against the caller (`ctx.sender`) and the framework version's scheduling rules |
| React | JSX components and hooks as functions; text shown as a JSX child is not treated as markup injection |
| Monorepos | Copies are compared within a package and across packages linked by a local dependency, not across separate example apps or templates |
| Bundlers and compilers | Minified and compiled output (a source map reference, very long lines) is skipped as generated |

Other files, such as Java, C#, PHP or Ruby, are listed as skipped with the reason and never fail the gate.

## Install

```sh
cargo install jevgate --locked
```

JevGate needs Rust 1.90 or later to build, and a [TypeSafe API key](https://console.typesafe.ai/settings/keys) to review. Git is needed only for `--base` and the staleness rule.

## Quick start

```sh
jevgate init                              # write a commented jevgate.toml for this repository
jevgate auth login                        # validate and save your TypeSafe API key
jevgate check --dry-run --show-requests   # see exactly what would be uploaded; free and offline
jevgate check --report                    # review, then open a local HTML dashboard
jevgate baseline                          # accept today's findings; later checks fail only on new ones
```

More ways to run it:

```sh
jevgate check src/billing --verbose               # one directory, with notes and per-file detail
jevgate check --rule default --rule security      # add the security group
jevgate check --rule documentation                # only agent instruction files and project docs
jevgate check --include-tests                     # also judge tests
jevgate check --base origin/main --format json    # changed files only, for agents and scripts
jevgate check --watch                             # re-check on save
jevgate baseline --merge                          # after a --base or path check: accept its findings, keep the rest
jevgate baseline mark wrong src/api/search.ts:41  # record why an accepted finding was accepted
jevgate baseline stats                            # each rule's rate of findings marked wrong
```

Every command documents itself: `jevgate --help` gives the workflow, exit codes, files and environment, and `jevgate check --help` explains each flag and the JSON report. `-h` prints a short summary.

## Continuous integration

A pull request review on GitHub Actions:

```yaml
name: JevGate
on: pull_request
permissions:
  contents: read
jobs:
  review:
    runs-on: ubuntu-latest
    env:
      JEVGATE_VERSION: 0.10.0
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0 # --base compares with the fork point
      - uses: actions/cache@v4
        with:
          path: ~/.cargo/bin/jevgate
          key: jevgate-${{ env.JEVGATE_VERSION }}-${{ runner.os }}-${{ runner.arch }}
      - run: command -v jevgate || cargo install jevgate --version "$JEVGATE_VERSION" --locked
      - uses: actions/cache@v4
        with:
          path: .jevgate/cache
          key: jevgate-answers-${{ github.sha }}
          restore-keys: jevgate-answers-
      - run: jevgate check --base "${{ github.event.pull_request.base.sha }}" --format github
        env:
          TYPESAFE_API_KEY: ${{ secrets.TYPESAFE_API_KEY }}
```

`--format github` annotates the changed lines with each finding. A finding that fails the gate is an error; the others are warnings. A Markdown table goes to the job summary, and the usual text goes to the log. The full JSON report is always at `.jevgate/latest.json` if you want to keep it as an artifact.

- **Changed files only:** `--base` reviews what changed since the fork point with that revision, the same files a pull request diff shows, plus uncommitted and untracked files. It needs the history, so check out with `fetch-depth: 0`. When no supported file changed, the run passes without any request.
- **Cache:** answers are stored under a hash of the exact request: source, questions and model. Restoring an older cache is always safe, and unchanged code costs nothing on the next run.
- **Advisory or blocking:** `fail_on = ["none"]` in `jevgate.toml` or `--fail-on none` reports findings without failing. A run that could not finish (missing key, provider rejection, request budget reached) still exits 2, so an outage never passes as a clean review.
- **A policy the change cannot edit:** a pull request can edit `jevgate.toml`. To apply the reviewed policy of the base branch instead, read it with `--config`:

  ```sh
  git show "$BASE_SHA:jevgate.toml" > "$RUNNER_TEMP/jevgate.toml"
  jevgate check --config "$RUNNER_TEMP/jevgate.toml" --base "$BASE_SHA" --format github
  ```

- **Forks:** GitHub withholds secrets from pull requests opened from forks, so there the run exits 2 with "No API key configured". Skip the job for forks, or run it only on branches of the repository.
- **Budgets:** `max_requests` caps the API attempts of one run. Reaching it leaves the run incomplete instead of passing on partial evidence. `--dry-run` counts the planned requests the cache already answers, so its estimate covers only what the cache lacks; follow-ups depend on answers and are not counted.
- **Transient failures:** rate limits, overload and server or edge errors (HTTP 408, 429, 500, 502–504, 520–524, 529) are retried up to four attempts; a timeout or dropped connection is retried once, since the first send may have run.
- **Report-only paths:** give tooling its own level with `[[scope]]` (below), so scripts are reported while product code gates.

Other CI systems work the same way: set `TYPESAFE_API_KEY`, keep `.jevgate/cache` between runs, and read the exit code or the JSON report.

## Configuration

`jevgate init` writes a commented `jevgate.toml` at the repository root. The command line wins over the file, except that upload patterns and budgets in the file are ceilings that flags can only narrow. Unknown keys are errors.

```toml
upload_allow = ["src/**", "tests/**"]   # only these paths may be uploaded
upload_deny = ["**/.env*", "**/*.pem", "**/*.key"]
include_tests = true
max_requests = 300

[rules]                                  # a level per group or rule
maintainability = "review"               # judge, and fail the gate on review findings
tests = "consider"
security = "consider"                    # opt-in group, enabled by naming it
"maintainability/hardcoded-values" = "report"   # judge but never fail; "off" skips it

[[scope]]                                # levels for the files these paths match
paths = ["scripts/**", "tools/**"]
fail_on = ["report"]                     # every rule: judge, never fail
rules = { security = "consider" }        # except these
```

| Key | Default | Meaning |
|---|---|---|
| `upload_allow` | every path | Globs of the paths that may be uploaded, including instruction files and context |
| `upload_deny` | none | Globs never uploaded, even when allowed |
| `generated` | built-in names | Globs of generated files, which are skipped |
| `tests` | built-in conventions | Globs of additional test files |
| `context` | none | Files always sent as related evidence, like `--context` |
| `rules` | the `default` group | A list selects rules. A table gives each group or rule a level: `review`, `consider`, `uncertain`, `report` (judge, never fail) or `off` |
| `[[scope]]` | none | `paths` (globs), with `fail_on` for every rule and `rules` for rules or groups, as above; `off` is not accepted (use `upload_deny`). The last scope that matches a file and addresses a rule wins; flags win over scopes |
| `fail_on` | `["review"]` | The level for rules without their own, like `--fail-on` |
| `include_tests` | `false` | Judge tests, like `--include-tests` |
| `model` | `jev-1.13.0` | TypeSafe model; a pinned version keeps results repeatable |
| `cache_ttl_secs` | `3600` | Cache lifetime for the `jev-latest` and `jev-preview` aliases; pinned versions never expire |
| `max_requests` | unlimited | Ceiling on API attempts per invocation |
| `concurrency` | `6` | Ceiling on simultaneous requests (1–8) |
| `max_file_bytes` | `262144` | Files larger than this are reported as needs-context, never truncated |
| `max_context_bytes` | `32768` | Ceiling on context bytes per request |

Rules are named by ID (`maintainability/shared-logic`), key (`shared_logic`) or group (`maintainability`, `tests`, `security`, `documentation`, `default`, `all`). The same names work in `--rule`, `--skip-rule` and `--fail-on TARGET=LEVEL`, and the most specific entry wins.

## Output and exit codes

| Format | Use |
|---|---|
| `agent` (default) | Ranked findings with locations and next steps, for people and coding agents |
| `json` | The full report: every file, finding, raw answer and probability, gate and usage |
| `jsonl` | One compact report per line; one per evaluation with `--watch` |
| `github` | GitHub Actions annotations and job summary, then the agent text |

Findings are `review` (act on it), `consider` (worth a look) or `note` (optional, shown with `--verbose`, never failing the gate). A file whose answers stay undecided is `uncertain`, and one that cannot be judged without more evidence is `needs-context`; neither is hidden or counted as clear. A finding's message shows the probability that set its level; a note shows none, and the JSON report keeps every raw value. Finished plans that share a directory are one finding. A hardcoded-value finding that cannot name its value is one level lower.

| Exit code | Meaning |
|---|---|
| 0 | Gate passed, or no supported file changed since `--base` |
| 1 | Gate failed |
| 2 | Run incomplete, invalid configuration or invalid usage |

`--fail-on review|consider|uncertain|none` sets what fails the gate; `--fail-on security=consider` sets it for one group or rule. Baselined findings and notes never fail it.

`jevgate baseline` can record why each finding was accepted: `intended` (right, and meant to be so), `later` (right, to fix later) or `wrong` (mistaken), with `--reason` or `jevgate baseline mark`. Reasons survive later rewrites of the baseline, and `jevgate baseline stats` reports each rule's share of findings marked wrong: labels from daily use, not the model's own probabilities.

## How it works

1. **Local analysis, nothing uploaded.** Tree-sitter parsers find functions, methods, types and registered callbacks, such as route handlers written inline in `app.post('/pages', async (c) => …)`. They measure nesting, group a file's members, find renamed copies, map tests to the functions they call, and list the statements where a value reaches another program. This evidence locates and scopes; it never decides a finding.
2. **Small, literal questions.** Each request covers one small unit and asks a few questions, such as "Would splitting this function make it easier to understand?" or "Does this function put a variable into the text of an SQL query instead of binding it?"
3. **Follow-ups only where needed.** When an answer is split, JevGate gathers more evidence (callee signatures, callers, a specific check) and asks once more instead of guessing.
4. **Composition in code.** Answers become `review`, `consider`, `note`, `clear` or `uncertain` at a 0.80 threshold. Raw probabilities stay in the JSON report.

[docs/classification-cascade.md](docs/classification-cascade.md) describes the evidence units and composition rules in detail.

## Privacy and cost

- **What is uploaded:** only the selected units of source, bounded by `upload_allow` and `upload_deny`. `--dry-run --show-requests` prints every initial request body without credentials or network access.
- **Instruction files:** uploaded only when a documentation rule is selected, and still bounded by the upload patterns.
- **Credentials:** a check reads `TYPESAFE_API_KEY` from the environment, then `--env-file` or the repository's `.env`, then the key saved by `jevgate auth login` (OS credential store, or an owner-only file). The key is never printed or written to reports.
- **Cost:** every run prints its input tokens and an estimated cost. Cached answers cost nothing.
- **Secrets:** out of scope on purpose, because judging secrets would mean uploading them. Use a local secret scanner.

## Limits

- **Languages and frameworks:** see [the support table](#supported-languages-and-frameworks). Astro, Vue and Svelte markup is not read, only their scripts.
- **Security scope:** one function plus at most one hop of callers. This is not whole-program data-flow analysis. Access control reads the final state of policies, SECURITY DEFINER functions and grants across a project's SQL files in path order; with `--base`, unchanged migrations are read for that state but not judged. It does not judge application-level authorization or dynamic SQL inside database functions.
- **Documentation scope:** staleness works only from the paths, scripts, tags and deletions that Git and the manifests show; it does not compare prose with code behavior. Paraphrases that share little wording are not found as duplicates; a translation is not a duplicate. Code comments are not judged yet. Token counts are estimates at four bytes per token.
- **Probabilities:** these are model judgments, not measured accuracy. JevGate complements linters, type checkers, tests and dedicated security scanners; it does not replace them.

## Contributing

Issues and pull requests are welcome. Run `cargo fmt`, `cargo clippy --all-targets` and `cargo test` before opening a pull request. When changing questions, validate on small, frozen sets of real code, and keep the probabilities and uncertainty visible.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
