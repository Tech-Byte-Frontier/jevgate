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

**[Documentation](https://tech-byte-frontier.github.io/jevgate/)** · [Rules](https://tech-byte-frontier.github.io/jevgate/reference/rules.html) · [Configuration](https://tech-byte-frontier.github.io/jevgate/configuration.html) · [CI](https://tech-byte-frontier.github.io/jevgate/ci.html) · [Troubleshooting](https://tech-byte-frontier.github.io/jevgate/troubleshooting.html) · [Changelog](CHANGELOG.md)

## What it finds

| Group | Rules | On |
|---|---|---|
| Maintainability | File organization, function simplification, shared logic, hardcoded values | by default |
| Tests | Test value (mock-only checks, expected values recomputed with the code's own logic), test redundancy | with `--include-tests` |
| Security | Injection, sensitive data, unsafe settings, SQL access control, GitHub workflows; each finding names a CWE | `--rule security` |
| Documentation | Agent instruction files, large and stale docs, duplicated sections, code comments | `--rule documentation` |

It reads Rust, Python, JavaScript, TypeScript, Go, C#, Ruby, PHP and Java, the scripts of Astro, Vue and Svelte files, SQL for PostgreSQL and Supabase, GitHub Actions workflows, and Markdown, MDX, reStructuredText and AsciiDoc, and knows the routes, handlers and settings of frameworks from Express, Next.js and SvelteKit to Django, Laravel, ASP.NET Core and Spring MVC. [What it finds](https://tech-byte-frontier.github.io/jevgate/what-it-finds.html) and [supported languages and frameworks](https://tech-byte-frontier.github.io/jevgate/languages.html) have the details; `jevgate rules` prints every rule with the question it asks.

## Install

```sh
brew install tech-byte-frontier/tap/jevgate   # macOS and Linux, with Homebrew
curl -fsSL https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/install.sh | sh   # Linux and macOS
cargo binstall jevgate            # any platform, with cargo-binstall
cargo install jevgate --locked    # build from source; needs Rust 1.90 or later
```

Releases have binaries for Linux, macOS and Windows with checksums and build provenance. Reviewing needs a [TypeSafe API key](https://console.typesafe.ai/settings/keys). [Install](https://tech-byte-frontier.github.io/jevgate/install.html) covers verifying a download, shell completions and man pages.

## Quick start

```sh
jevgate init                              # write a commented jevgate.toml for this repository
jevgate auth login                        # validate and save your TypeSafe API key
jevgate check --dry-run --show-requests   # see exactly what would be uploaded; free and offline
jevgate check --report                    # review, then open a local HTML dashboard
jevgate baseline                          # accept today's findings; later checks fail only on new ones
```

`jevgate --help` gives the workflow, exit codes, files and environment, and `jevgate check --help` explains each flag and the JSON report. Coding agents can also call JevGate as a tool through its MCP server, `jevgate mcp` ([coding agents](https://tech-byte-frontier.github.io/jevgate/coding-agents.html)).

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
          version: 0.19.0
```

It reviews only the changed files, annotates each finding on its line and writes a job summary; unchanged code is answered from the cache for free. [Continuous integration](https://tech-byte-frontier.github.io/jevgate/ci.html) covers pre-commit, other CI systems, pull requests from forks, budgets and a gate policy the change cannot edit.

## Output and exit codes

`--format agent` (default, for people and coding agents), `json` (the full report, also always at `.jevgate/latest.json`), `jsonl`, `github` (annotations and a job summary) or `sarif` (for GitHub code scanning); see [output and exit codes](https://tech-byte-frontier.github.io/jevgate/output.html).

| Exit code | Meaning |
|---|---|
| 0 | Gate passed, or no supported file changed since `--base` |
| 1 | Gate failed |
| 2 | Run incomplete, invalid configuration or invalid usage |

Findings are `review` (act on it), `consider` (worth a look) or `note` (optional). A file whose answers stay undecided is `uncertain`, never hidden or counted as clear. `--fail-on` and `jevgate.toml` set what fails the gate, per rule and per path. `jevgate baseline` accepts today's findings, and a `jevgate: allow(RULE) reason` comment accepts one where it is.

## Privacy and cost

- **What is uploaded:** only the selected units of source, bounded by `upload_allow` and `upload_deny`; `--dry-run --show-requests` prints every request body offline.
- **Secrets:** out of scope on purpose, because judging secrets would mean uploading them. Use a local secret scanner.
- **Cost:** every run prints its input tokens and an estimated cost, and cached answers cost nothing.

[Privacy and cost](https://tech-byte-frontier.github.io/jevgate/privacy-and-cost.html) and [limits](https://tech-byte-frontier.github.io/jevgate/limits.html) say more, and [how it works](https://tech-byte-frontier.github.io/jevgate/how-it-works.html) explains the evidence units and how code turns answers into findings.

## Contributing

Issues and pull requests are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). Report a finding that looks wrong with the [wrong finding](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml) template, and a vulnerability as [SECURITY.md](SECURITY.md) describes.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
