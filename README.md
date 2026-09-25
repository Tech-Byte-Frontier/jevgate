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
| Injection | Variables reaching SQL, shell commands, evaluated code, HTML, file paths, outbound URLs or redirect targets without binding, escaping or checks; in C#, types named by input or chosen by the data being deserialized; in Django code, request data given to `pickle` or `yaml.load`; in PHP, `unserialize` and uploaded file names |
| Sensitive data | Passwords, tokens or personal data written to logs; internal error details sent to clients, judged per error message and once per error handler (`app.onError`, `setErrorHandler`, Express error middleware, Flask and FastAPI handlers, Django error views and `process_exception` middleware, Django REST framework's `EXCEPTION_HANDLER`, NestJS filters, axum `IntoResponse` and actix-web `ResponseError` for error types, ASP.NET Core exception handlers, PHP `set_exception_handler`, Slim and Laravel handler classes); in Django code, also the server's environment or settings sent to clients (`request.META`) |
| Unsafe settings | Certificate checks turned off, weak password hashing, non-cryptographic random secrets, permissive CORS, session cookies without `Secure`/`HttpOnly`, secrets in environment variables the build puts into browser code (`NEXT_PUBLIC_`, `VITE_`); in C#, also developer exception pages outside development, token signature or lifetime checks turned off, signing keys written in the code, and secrets derived from data others know; in Django code, also debug mode for the deployed site, `csrf_exempt` views and secret keys written in settings |
| Access control | SQL row-level policies that let every user reach other users' rows or trust `user_metadata`; SECURITY DEFINER functions without a fixed `search_path` or a caller check; grants that open writes to every user. SpacetimeDB modules (TypeScript and Rust, any kind of application): public tables of users' private data, views that return other users' rows, reducers that change rows their arguments choose or admin-only settings without checking the caller, and scheduled reducers clients can call in 1.x |
| Workflows | GitHub Actions `run` scripts that execute text outside people write (`${{ github.event.pull_request.title }}`); `pull_request_target` or `workflow_run` jobs that run pull request code with secrets |

**Documentation** (opt-in with `--rule documentation`)

| Rule | Example finding |
|---|---|
| Agent context | A section of `CLAUDE.md` only lists the scripts `package.json` already shows, and six harnesses load it at the start of every session. |
| Large docs | `docs/operations/runbook.md` holds several unrelated subjects; `docs/plans/v0.2-plan.md` mainly records finished work. |
| Staleness | `docs/plans/v0.4-auth.md` is a plan whose work is finished: the repository has a release tag v0.4.0, and 6 paths it names were since removed. |
| Duplication | Section `Release Workflow` of `CLAUDE.md` states everything section `Release` of `README.md` states. |
| Code comments | `save_skill` has 3 comments to clean up: at lines 214, 218 and 222 they repeat the code (`# Create skill directory` above `skill_dir.mkdir(…)`). A module docstring saying it was "split out of `portfolio.py` to stay under the 500-line budget" narrates an edit instead of the code as it is. |

The documentation rules read the instruction files that coding agents load (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and Claude, Cursor, Copilot, Windsurf, Cline, Kiro, Junie and Roo Code rules), even when hidden or gitignored. Each section is asked whether it only restates the stack, the manifest's commands, generic advice or a configured linter's rules, and whether text loaded in every session applies to only one directory. Project documentation in Markdown, MDX, reStructuredText or AsciiDoc of 300 or more lines is judged from its headings alone. Code finds staleness and duplication candidates: named paths or scripts that no longer exist, release tags, deleted files, and shared wording outside code examples. Jev then judges each candidate. Code comments and docstrings of application code are judged one at a time with the code they are about (the declaration they document, the lines below them or the line they end): whether they only repeat that code, hold sentences that add nothing, narrate an edit instead of the code as it is, or are code turned off. License headers, tool directives, type annotations, authorship tags and Sphinx version notes are left out; documentation that only repeats its declaration or says it at length (framework section banners included) is at most a `note`, as is every such comment in a project whose README says its code is written for learners; and the comments of one definition that span fewer than three lines in all are a `note`. Documentation findings are at most `consider`. The run also estimates the tokens each harness loads at session start; these estimates are evidence and never fail the gate.

`jevgate rules` prints every rule with its question and default.

## Supported languages and frameworks

✅ judged · ➖ not applicable

| Language or file | Extensions | Maintainability | Tests | Security | Documentation |
|---|---|:---:|:---:|:---:|:---:|
| Rust | `.rs` | ✅ | ✅ `#[test]`, `#[cfg(test)]` | ✅ | ✅ comments |
| Python | `.py` | ✅ | ✅ pytest, unittest | ✅ | ✅ comments |
| JavaScript | `.js` `.jsx` `.mjs` `.cjs` | ✅ | ✅ `describe`/`it`/`test` | ✅ | ✅ comments |
| TypeScript | `.ts` `.tsx` `.mts` `.cts` | ✅ | ✅ `describe`/`it`/`test` | ✅ | ✅ comments |
| Go | `.go` | ✅ | ✅ `Test…(t *testing.T)` | ✅ | ✅ comments |
| C# | `.cs` | ✅ | ✅ xUnit, NUnit, MSTest | ✅ | ✅ comments |
| Ruby | `.rb` | ✅ | ✅ RSpec, Minitest, Rails `test "…" do` | ✅ no Ruby framework handlers yet | ✅ comments |
| PHP | `.php` `.phtml` | ✅ | ✅ PHPUnit `…TestCase` classes, Pest `test`/`it` | ✅ | ✅ comments |
| Java | `.java` | ✅ | ✅ JUnit 4 and 5, TestNG: `@Test`, `@ParameterizedTest`, `@Nested`, JUnit 3 `TestCase` | ✅ | ✅ comments |
| Astro, Vue, Svelte | `.astro` `.vue` `.svelte` | ✅ scripts only | ➖ | ✅ scripts only | ✅ script comments |
| SQL (PostgreSQL, Supabase) | `.sql` | ➖ | ➖ | ✅ access control | ➖ |
| GitHub Actions | `.github/workflows/*.yml` | ➖ | ➖ | ✅ workflows | ➖ |
| Markdown, MDX | `.md` `.mdx` at the root, in `docs/` or `doc/`, READMEs and CONTRIBUTING files; agent instruction files | ➖ | ➖ | ➖ | ✅ |
| reStructuredText, AsciiDoc | `.rst` `.adoc` `.asciidoc`, in the same places | ➖ | ➖ | ➖ | ✅ |

| Framework or platform | What JevGate understands |
|---|---|
| Hono, Express, Fastify, Koa | Route handlers written inline (`app.post('/pages', async (c) => …)`); error handlers (`app.onError`, `setErrorHandler`, four-parameter Express middleware) |
| NestJS | Exception filters (`@Catch`) |
| Next.js (App Router and Pages Router) | Route handlers (`app/**/route.ts`), Server Actions (`'use server'` files and functions), `pages/api` routes, middleware, client components, error boundaries and pages are named to Jev with who calls them and where they run, so a Server Action's arguments read as client input and a client component's requests as the user's own; `dangerouslySetInnerHTML`, redirects to client-chosen URLs, raw Prisma and Drizzle queries (`$queryRawUnsafe`, `sql.raw`) as opposed to their binding tagged templates, `NEXT_PUBLIC_` secrets, and `next.config` headers |
| SvelteKit | Server load functions and form actions (`+page.server.js`, `export const actions = {…}`), endpoints (`+server.js`) and server hooks are named to Jev with who calls them, so their request, form data, URL and cookies read as client input, and `cookies.set` is read with its secure defaults |
| Flask, FastAPI | Error handlers (`@app.errorhandler`, `@app.exception_handler`) |
| Django, Django REST framework | Views and viewsets with the URL routes that reach them, the templates they render with `\|safe` or autoescaping off, and the module constants they use; settings modules, with secret literals redacted, the settings modules that import and override them, and the files that select them (`DJANGO_SETTINGS_MODULE`); management commands as run by hand; `handler500`-style error views, middleware `process_exception` and `EXCEPTION_HANDLER` as error handlers |
| PHP pages, Slim, Laravel | A file's top-level code is judged like a function, since a page script reads the request and writes the response; route closures (`$app->get('/users', function …)`, `Route::post(…)`) and configuration closures (`return function (App $app) {…}`); error handlers (`set_exception_handler`, subclasses of Slim's `ErrorHandler` and Laravel's `ExceptionHandler`) |
| axum, actix-web, Rocket | Error responses (`IntoResponse` or `ResponseError` for an error type, `#[catch]`) |
| MDX sites (Next.js, Nextra, Docusaurus, Astro) | Imports, exports, comments and component markup are dropped, but the prose components carry (a `<Note>`'s text, a properties table's descriptions, with names and types as code) is read; frontmatter `title` names the page |
| Sphinx, AsciiDoc | Section titles by their adornment or `=` level; comments, attribute entries and options dropped; code directives, literal and listing blocks read as code; `:file:` and `include::` targets checked as paths, `:attr:` and `:class:` read as code, and `_build/` skipped |
| ASP.NET Core | Controller actions, minimal API route handlers (`app.MapGet("/orders", …)`) and inline middleware; exception handlers (`UseExceptionHandler` with a handler, `IExceptionFilter`, `IExceptionHandler`, middleware classes that catch what the pipeline throws); Entity Framework Core raw and interpolated SQL, CORS and cookie options, `UseDeveloperExceptionPage`, JWT validation options and the constants a setup names |
| .NET projects | Test projects named like `Shop.Tests` or `Shop.UnitTests`, and classes of `[Fact]`, `[Theory]`, `[Test]` or `[TestMethod]` methods anywhere; designer and source-generated files (`.Designer.cs`, `.g.cs`) are skipped as generated |
| Supabase and PostgreSQL | Row-level security policies, `SECURITY DEFINER` functions, grants, and the claims an access token hook sets |
| SpacetimeDB (TypeScript and Rust modules) | Public tables, views and reducers, checked against the caller (`ctx.sender`) and the framework version's scheduling rules |
| React | JSX components and hooks as functions; text shown as a JSX child is not treated as markup injection |
| RSpec, Minitest | Examples with the groups they are declared in, the `before` hooks and the `let`/`subject` definitions they read, and the helpers they call from support files such as `spec/support` and `test_helper.rb` |
| Sinatra and other Ruby DSLs | Methods of classes and modules (`def`, `def self.`, `class << self`, `define_method`); blocks passed at class or file level as units named by their call (`get('/invoices')`); constants as hardcoded values |
| Monorepos and examples | Copies are compared within a package and across packages linked by a local dependency, not across separate example apps, templates or variants of one example (`examples/login/raw` and `examples/login/sdk`); copies inside example code are notes |
| Java classes | Methods and constructors belong to their class, interface, enum constant or record; `static` fields are constants; `equals` and `hashCode` overrides, constructors storing fields and setters given literals are boilerplate or data, never copies; initial capacities and a number a method returns whole are not values to name; a class of the same package counts as imported |
| Spring MVC | A MockMvc or RestTemplate test request reaches the controller method whose `@GetMapping`, `@PostMapping` or `@RequestMapping` route serves it, so the test is judged with that method as its code under test |
| Bundlers and compilers | Minified and compiled output (a source map reference, very long lines) is skipped as generated |
| Copied libraries | A library copied into the repository (a versioned file name such as `jquery-3.6.0.js`, the readable build beside a `.min.js`, a license banner naming a version, or a script under `assets`, `static` or `vendor` that opens with a whole license and copyright) is skipped as vendored, whatever its size |
| Migrations | Directories named `migrations`, Rails' `db/migrate` and timestamped scripts under `db/`, and Alembic's `alembic/versions` are skipped as migrations; SQL migrations are still read for access control |

Other files, such as Kotlin, are listed as skipped with the reason and never fail the gate.

## Install

```sh
brew install tech-byte-frontier/tap/jevgate   # macOS and Linux, with Homebrew
curl -fsSL https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/install.sh | sh   # Linux and macOS
cargo binstall jevgate            # any platform, with cargo-binstall
cargo install jevgate --locked    # build from source; needs Rust 1.90 or later
```

Each [release](https://github.com/Tech-Byte-Frontier/jevgate/releases) has binaries for Linux (x86_64 and arm64, static), macOS (Apple silicon and Intel) and Windows (x86_64), with SHA-256 checksums and build provenance: `gh attestation verify <archive> --repo Tech-Byte-Frontier/jevgate`. The install script checks the checksum and installs to `~/.local/bin`; set `JEVGATE_VERSION` or `JEVGATE_INSTALL_DIR` to change the version or place.

`jevgate completions bash|zsh|fish|powershell` prints a shell completion script and `jevgate man` a man page; Homebrew installs both.

Reviewing needs a [TypeSafe API key](https://console.typesafe.ai/settings/keys). Git is needed only for `--base` and the staleness rule.

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
jevgate check --rule documentation                # agent instruction files, project docs and code comments
jevgate check --rule comments                     # only code comments
jevgate check --include-tests                     # also judge tests
jevgate check --base origin/main --format json    # changed files only, for agents and scripts
jevgate check --watch                             # re-check on save
jevgate baseline --merge                          # after a --base or path check: accept its findings, keep the rest
jevgate baseline mark wrong src/api/search.ts:41  # record why an accepted finding was accepted
jevgate baseline stats                            # each rule's rate of findings marked wrong
```

Every command documents itself: `jevgate --help` gives the workflow, exit codes, files and environment, and `jevgate check --help` explains each flag and the JSON report. `-h` prints a short summary.

## Continuous integration

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
          version: 0.17.0
```

The action installs a checked release binary, keeps `.jevgate/cache` in the Actions cache and runs `jevgate check --base <pull request base> --format github`; `args` passes more flags, such as `--rule security`. It runs on Linux, macOS and Windows runners.

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

Before each commit, with [pre-commit](https://pre-commit.com), review what is staged:

```yaml
repos:
  - repo: https://github.com/Tech-Byte-Frontier/jevgate
    rev: v0.18.0
    hooks:
      - id: jevgate-system   # the jevgate on PATH; `jevgate` builds it with Rust instead
```

Other CI systems work the same way: install with `install.sh` or `cargo binstall`, set `TYPESAFE_API_KEY`, keep `.jevgate/cache` between runs, and read the exit code or the JSON report.

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
| `max_file_bytes` | `262144` | Files larger than this are reported as needs-context, never truncated; generated and vendored files are skipped instead |
| `max_context_bytes` | `32768` | Ceiling on context bytes per request |

Rules are named by ID (`maintainability/shared-logic`), key (`shared_logic`) or group (`maintainability`, `tests`, `security`, `documentation`, `default`, `all`). The same names work in `--rule`, `--skip-rule` and `--fail-on TARGET=LEVEL`, and the most specific entry wins.

## Output and exit codes

| Format | Use |
|---|---|
| `agent` (default) | Ranked findings with locations and next steps, for people and coding agents |
| `json` | The full report: every file, finding, raw answer and probability, gate and usage |
| `jsonl` | One compact report per line; one per evaluation with `--watch` |
| `github` | GitHub Actions annotations and job summary, then the agent text |
| `sarif` | A SARIF 2.1.0 log for [GitHub code scanning](https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/uploading-a-sarif-file-to-github) and other SARIF readers: the findings the annotations show, `error` when they fail the gate |

Agent output is colored on a terminal; `--color never`, or `NO_COLOR` set to any value, turns it off, and `--color always` or `CLICOLOR_FORCE` turns it on for pipes and logs.

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

- **Languages and frameworks:** see [the support table](#supported-languages-and-frameworks). Astro, Vue and Svelte markup is not read, only their scripts. PHP's inline HTML is read only for its `<?= … ?>` echoes, and variables a page gets from the files it includes are not followed there: they are judged where those files set them.
- **Security scope:** one function plus at most one hop of callers. This is not whole-program data-flow analysis. Access control reads the final state of policies, SECURITY DEFINER functions and grants across a project's SQL files in path order, leaving out uninstall, teardown, rollback and down scripts; with `--base`, unchanged migrations are read for that state but not judged. It does not judge application-level authorization or dynamic SQL inside database functions.
- **Documentation scope:** staleness works only from the paths, scripts, tags and deletions that Git and the manifests show; it does not compare prose with code behavior. Paraphrases that share little wording are not found as duplicates, nor are code examples that share only code; a translation is not a duplicate. Sphinx and AsciiDoc includes are not followed, and MDX expressions are not evaluated. A code comment is judged with the code next to it, not against what the whole program does, so a comment that no longer matches its code is not found. Token counts are estimates at four bytes per token.
- **Probabilities:** these are model judgments, not measured accuracy. JevGate complements linters, type checkers, tests and dedicated security scanners; it does not replace them.

## Contributing

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). Report a finding that looks wrong with the [wrong finding](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml) template, and a vulnerability as [SECURITY.md](SECURITY.md) describes. Changes are listed in [CHANGELOG.md](CHANGELOG.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
