# JevGate

[![crates.io](https://img.shields.io/crates/v/jevgate.svg)](https://crates.io/crates/jevgate)
[![CI](https://github.com/Tech-Byte-Frontier/jevgate/actions/workflows/ci.yml/badge.svg)](https://github.com/Tech-Byte-Frontier/jevgate/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/jevgate.svg)](#license)
[![MSRV 1.90](https://img.shields.io/badge/rustc-1.90+-informational.svg)](https://www.rust-lang.org)

**JevGate is a code-review gate. It asks small, precise questions about your code and turns the answers into findings you can act on.**

JevGate parses your repository locally and builds small units of evidence: a function, a file outline, a pair of copies, a test, a documentation section. It asks [TypeSafe Jev](https://docs.typesafe.ai) short, typed questions about each one. Code, not a chat model, combines the answers into a verdict. Each finding has a location, how often findings like it were right and a concrete next step, so an agent or CI job can act on it and a person can check it quickly. By default the gate fails only on the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on: among the default rules, function-simplification reviews, right 20 of the 23 times they were labeled there (87%), and 80 of 93 times on 27 more public projects ([accuracy](https://tech-byte-frontier.github.io/jevgate/accuracy.html)).

![JevGate's terminal output on zoxide: the gate fails on 1 function-simplification review (a function mixing separate jobs); 3 more reviews (a file holding several features, two sets of importers repeating the same steps) and a consider (branching that hides a main path) are reported without failing it, since their rules and levels are still being measured; each finding ends with how often findings of its rule and level were right](site/src/images/terminal.svg)

JevGate 0.28.0 on [zoxide](https://github.com/ajeetdsouza/zoxide/tree/09a18b4424b3f1033094ffd97da6d47585e38259), rerun from its answer cache, so it cost nothing.

**[Documentation](https://tech-byte-frontier.github.io/jevgate/)** · [Rules](https://tech-byte-frontier.github.io/jevgate/reference/rules.html) · [Configuration](https://tech-byte-frontier.github.io/jevgate/configuration.html) · [CI](https://tech-byte-frontier.github.io/jevgate/ci.html) · [Troubleshooting](https://tech-byte-frontier.github.io/jevgate/troubleshooting.html) · [Changelog](CHANGELOG.md)

## What it finds

| Group | Rules | On |
|---|---|---|
| Maintainability | File organization, function simplification, shared logic; hardcoded values (opt-in) | by default |
| Tests | Test value (mock-only checks, expected values recomputed with the code's own logic), test redundancy | with `--include-tests` |
| Security | Injection, sensitive data, unsafe settings, SQL access control, GitHub workflows; each finding names a CWE | `--rule security` |
| Documentation | Agent instruction files, large and stale docs, duplicated sections, code comments | `--rule documentation` |
| Custom | Your team's conventions as yes/no questions in `jevgate.toml` or `.jevgate/questions/`, drafted from `AGENTS.md` by `jevgate rules propose` or added from a measured gallery with `jevgate rules add` ([custom questions](https://tech-byte-frontier.github.io/jevgate/custom-questions.html)) | once defined |

It reads Rust, Python, JavaScript, TypeScript, Go, C#, Ruby, PHP, Java and Bend 2 (and, in preview, C, C++, Kotlin, Swift, Bash, Dart, Scala, Elixir and Lua for function simplification, file organization, shared logic and comments), the scripts of Astro, Vue and Svelte files and the inline scripts of server templates (ERB, EJS, JSP, Handlebars, Jinja and others), SQL for PostgreSQL and Supabase, GitHub Actions workflows, and Markdown, MDX, reStructuredText and AsciiDoc, and knows the routes, handlers and settings of frameworks from Express, Next.js and SvelteKit to Django, Laravel, ASP.NET Core and Spring MVC. [What it finds](https://tech-byte-frontier.github.io/jevgate/what-it-finds.html) and [supported languages and frameworks](https://tech-byte-frontier.github.io/jevgate/languages.html) have the details; `jevgate rules` prints every rule with the question it asks.

## Install

```sh
brew install tech-byte-frontier/tap/jevgate   # macOS and Linux, with Homebrew
curl -fsSL https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/install.sh | sh   # Linux and macOS
cargo binstall jevgate            # any platform, with cargo-binstall
cargo install jevgate --locked    # build from source; needs Rust 1.90 or later
```

Releases have binaries for Linux, macOS and Windows with checksums and build provenance. Reviewing needs an API key from [TypeSafe](https://console.typesafe.ai/settings/keys) or [OpenRouter](https://openrouter.ai/settings/keys), which serve the same model at the same price; a [Vercel AI Gateway](https://vercel.com/docs/ai-gateway/authentication-and-byok/api-keys) key is accepted too, but has not been tried with a real key yet. [Install](https://tech-byte-frontier.github.io/jevgate/install.html) covers verifying a download, shell completions and man pages.

## Quick start

```sh
jevgate init                              # write a commented jevgate.toml for this repository
jevgate auth login                        # validate and save your API key: TypeSafe, OpenRouter or Vercel
jevgate check --dry-run --show-requests   # see exactly what would be uploaded; free and offline
jevgate check --report                    # review, then open a local HTML dashboard
jevgate baseline                          # accept today's findings; later checks fail only on new ones
jevgate init --agent claude               # check each edit and turn of Claude Code; also codex, cursor, gemini, opencode
```

`jevgate check --report` writes the same findings to a local dashboard you can filter by path and classification, with each file's findings, undecided units and the answers behind them:

![JevGate's HTML report on zoxide: the gate's result and what fails it by default, totals for files, findings, notes and cost, then a list of files by classification, with src/util.rs open to show its two review findings, the one that fails the gate marked, each with how often findings like it were right, and how each rule classified the file](site/src/images/report.png)

`jevgate --help` gives the workflow, exit codes, files and environment, and `jevgate check --help` explains each flag and the JSON report. In a coding agent, JevGate's hooks give the agent each edit's findings and keep it working while findings fail the gate; a Claude Code plugin bundles them with the MCP server, `jevgate mcp` ([coding agents](https://tech-byte-frontier.github.io/jevgate/coding-agents.html)).

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
          version: 0.25.0
```

It reviews only what the pull request changed (the functions, tests and comments on changed lines, and copies where either copy changed), annotates each finding on its line and writes a job summary; unchanged code is answered from the cache for free. [Continuous integration](https://tech-byte-frontier.github.io/jevgate/ci.html) covers pre-commit, other CI systems, pull requests from forks, budgets and a gate policy the change cannot edit.

## Output and exit codes

`--format agent` (default, for people and coding agents), `json` (the full report, also always at `.jevgate/latest.json`), `jsonl`, `github` (annotations and a job summary) or `sarif` (for GitHub code scanning); see [output and exit codes](https://tech-byte-frontier.github.io/jevgate/output.html).

| Exit code | Meaning |
|---|---|
| 0 | Gate passed, or no supported file changed since `--base` |
| 1 | Gate failed |
| 2 | Run incomplete, invalid configuration or invalid usage |

Findings are `review` (act on it), `consider` (worth a look) or `note` (optional). A file whose answers stay undecided is `uncertain`, never hidden or counted as clear. By default only the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on fail the gate (`jevgate rules` shows them), outside the preview languages, and custom questions at their own level; the other findings are reported without failing it. `--fail-on` and `jevgate.toml` set what fails the gate, per rule and per path. `jevgate baseline` accepts today's findings, and a `jevgate: allow(RULE) reason` comment accepts one where it is.

## Privacy and cost

- **What is uploaded:** only the selected units of source, bounded by `upload_allow` and `upload_deny`; `--dry-run --show-requests` prints every request body offline.
- **Secrets:** out of scope on purpose, because judging secrets would mean uploading them. Use a local secret scanner.
- **Cost:** every run prints its input tokens and an estimated cost, and cached answers cost nothing, so a rerun of unchanged code sends no request. Jev 1.13 costs $0.042 per million input tokens: checked as pull requests with every rule and nothing cached, the last commits of 118 corpus projects sent 4.55M first-pass input tokens, $0.19 for all 118.

[Privacy and cost](https://tech-byte-frontier.github.io/jevgate/privacy-and-cost.html) and [limits](https://tech-byte-frontier.github.io/jevgate/limits.html) say more, and [how it works](https://tech-byte-frontier.github.io/jevgate/how-it-works.html) explains the evidence units and how code turns answers into findings.

## Contributing

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). Report a finding that looks wrong with the [wrong finding](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml) template, and a vulnerability as [SECURITY.md](SECURITY.md) describes.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
