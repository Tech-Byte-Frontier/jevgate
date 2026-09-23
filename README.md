# JevGate

**A code-review gate that asks small, precise questions about your code and turns the answers into findings you can act on.**

JevGate parses your repository locally, builds small units of evidence (a function, a file outline, a pair of copies, a test) and asks [TypeSafe Jev](https://docs.typesafe.ai) short, typed questions about each one. Code, not a chat model, combines the answers into a verdict. Each finding comes with a location, a probability and a concrete next step. An agent or CI job can act on it, and a person can check it quickly.

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

## What it finds

**Maintainability** (on by default)

| Rule | Example finding |
|---|---|
| File organization | This file holds two unrelated responsibilities; the upload helpers would be most useful as their own module. |
| Function simplification | `sync_accounts` mixes separate jobs in long blocks; lines 40–71 would be most useful as their own function. |
| Shared logic | `createInvoice` and `createReceipt` perform the same steps; one shared implementation would serve both. |
| Hardcoded values | Module constants fix a value that differs between deployments; `apply_discount` special-cases one specific customer. |

**Tests** (with `--include-tests`)

| Rule | Example finding |
|---|---|
| Test value | `test_total` computes its expected value with the logic it tests. |
| Test redundancy | Three tests of `parse_date` check the same behavior; one parameterized test could hold them. |

**Security** (opt-in with `--rule security`; each finding names a CWE)

| Rule | Covers |
|---|---|
| Injection | Variables reaching SQL, shell commands, evaluated code, HTML, file paths or outbound URLs without binding, escaping or checks |
| Sensitive data | Passwords, tokens or personal data written to logs; internal error details sent to clients |
| Unsafe settings | Certificate checks turned off, weak password hashing, non-cryptographic random secrets, permissive CORS, session cookies without `Secure`/`HttpOnly` |

`jevgate rules` prints every rule with its question and default.

## Quick start

```sh
cargo install jevgate --locked
jevgate init           # writes a commented jevgate.toml for this repository
jevgate auth login     # stores your TypeSafe API key

jevgate check --dry-run --show-requests   # see exactly what would be uploaded, free
jevgate check src --report                # review and open a local HTML dashboard
```

More ways to run it:

```sh
jevgate check --rule default --rule security          # add the security group
jevgate check --base origin/main --format json        # changed files only, for CI and agents
jevgate check --include-tests                         # also judge tests
jevgate check --watch                                 # re-check on save
```

## How it works

1. **Local analysis, nothing uploaded.** Tree-sitter parsers find functions, methods, types and registered callbacks. They measure nesting, group a file's members, find renamed copies, map tests to the functions they call, and list the statements where a value reaches another program. This evidence only locates and scopes; it never decides a finding.
2. **Small, literal questions.** Each request covers one small unit and asks a few questions, such as "Would splitting this function make it easier to understand?" or "Does this function put a variable into the text of an SQL query instead of binding it?"
3. **Follow-ups only where needed.** When an answer is split, JevGate gathers more evidence (callee signatures, callers, a specific check) and asks once more, instead of guessing.
4. **Composition in code.** Answers become `review`, `consider`, `note`, `clear` or `uncertain` at a 0.80 threshold. Raw probabilities are kept in the JSON report, and an undecided answer is reported as such rather than hidden.

Unchanged units are answered from a local cache, so re-runs only pay for what changed.

## Configuration

`jevgate init` writes a starting `jevgate.toml` at the repository root:

```toml
upload_allow = ["src/**", "tests/**"]   # only these paths may be uploaded
upload_deny = ["**/.env*", "**/*.pem", "**/*.key"]

[rules]                                  # a level per group or rule
maintainability = "review"               # judge, and fail the gate on review findings
tests = "consider"
security = "consider"                    # opt-in group
"maintainability/hardcoded-values" = "report"   # judge but never fail; "off" skips it
```

- **Groups:** `maintainability`, `tests`, `security`, `default` and `all`. They work with `--rule`, `--skip-rule` and `--fail-on`, and anywhere a rule ID does.
- **Precedence:** the command line wins over the file, and a rule's own entry wins over its group's.
- **Validation:** unknown keys are errors, and budgets in the file are ceilings that flags can only lower.

## Gates, baselines and CI

`--fail-on review|consider|uncertain|none` sets what fails the check. Use `--fail-on security=consider` for a single group or rule. Notes never fail the gate.

| Exit code | Meaning |
|---|---|
| 0 | Gate passed |
| 1 | Gate failed |
| 2 | Run incomplete, or an error |

To adopt JevGate on an existing codebase, run a check, then `jevgate baseline`. Later checks only fail on new findings.

In CI, inject `TYPESAFE_API_KEY`, and restore and save `.jevgate/cache` (plus `.jevgate/latest.json` for change tracking). Answers from a pinned model version never expire. Rate limits are retried with backoff; account rejections stop the run.

## Privacy and cost

- **What is uploaded:** only the selected units of source leave your machine, and `upload_allow`/`upload_deny` bound them. `--dry-run --show-requests` prints every request body without credentials or network access.
- **Cost:** every run prints its input tokens and an estimated cost, and cached answers cost nothing.
- **Secrets:** they are deliberately out of scope. Use a local secret scanner, because judging secrets would mean uploading them.

## Limits

- **Languages:** Rust, Python, JavaScript and TypeScript are supported. Other files are listed as skipped, with the reason.
- **Security scope:** security rules look at one function plus at most one hop of callers. They are not whole-program data-flow analysis, and they don't cover SQL files, row-level policies or access control.
- **Probabilities:** these are model judgments, not measured accuracy. JevGate complements linters, type checkers, tests and dedicated security scanners; it does not replace them.

## Contributing

Issues and pull requests are welcome. Run `cargo fmt`, `cargo clippy --all-targets` and `cargo test` before opening a pull request. [docs/classification-cascade.md](docs/classification-cascade.md) explains the evidence units and composition rules. When changing questions, validate on small, frozen sets of real code, and keep the probabilities and uncertainty visible.

Licensed under MIT OR Apache-2.0.
