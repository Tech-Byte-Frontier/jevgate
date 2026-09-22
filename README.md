# JevGate

Code review with TypeSafe Jev over small evidence units. Five rules:

- **File organization:** do a file's members serve separate purposes that could be their own modules?
- **Function simplification:** does a function perform two or more substantial tasks, or could its nesting be flattened?
- **Shared logic:** do two renamed or exact copies perform the same steps for the same purpose?
- **Test value** (`--include-tests`): does a test only check its mocks, recompute its expected value, assert internal details or mix unrelated behaviors?
- **Test redundancy** (`--include-tests`): do similar tests of one function check the same behavior?

## Use

```sh
cargo install jevgate --locked
jevgate auth login

# Review a directory and open a local dashboard
jevgate check src --report

# Include tests; judge changed files only; machine-readable output
jevgate check --include-tests --base origin/main --format json

# Inspect what would be uploaded, with planned requests and tokens per stage
jevgate check src --dry-run --show-requests
```

`jevgate rules` prints the rule catalog. `--rule` selects a subset. `--context`
adds a related file as evidence for shared logic, callers and test subjects.

## How it works

Local analysis runs first and uploads nothing: parsers find functions, methods
and types; group a file's members by calls and shared types; find Type-2 clone
candidates (identifiers and literals normalized, whole statements, consistent
renaming) across the selected files; and map tests to the functions they call.

Each request then asks short questions about one unit: a pack of up to eight
functions, one file outline (signatures and groups, no bodies), one candidate
pair, a pack of tests, or one pair of tests. A unit that stays uncertain gets one
recheck with more evidence (callee signatures, or the enclosing functions).
Code composes the answers at a 0.80 threshold into `review`, `consider`,
`clear` or `uncertain`, and ranks findings by probability × ln(1 + lines).
Every `review` carries a finding. Raw answers are kept under `files[].judgments`.

Test files are judged only with `--include-tests`. A file that mixes code and
tests keeps them apart: application rules judge the code, test rules the tests.
A test path that still contains other code gets one file-purpose question first.

## Gate, baseline and exit codes

`--fail-on review|consider|uncertain|none` (repeatable, default `review`, or
`fail_on` in `jevgate.toml`) decides what fails the check. `consider` also fails
on review findings. Exit codes: `0` pass, `1` gate failed, `2` incomplete or error.

`jevgate baseline` accepts the findings of the last complete check in
`jevgate-baseline.json` at the project root, without API calls. Later checks mark
those findings `baselined` and fail only on new ones. Findings are matched by a
fingerprint of rule, path, unit and normalized evidence.

## Configuration and CI

The nearest `jevgate.toml` or Git root defines the project boundary:

```toml
upload_allow = ["src/**", "tests/**"]
upload_deny = ["src/private/**"]
generated = ["**/*.generated.*"]
fail_on = ["review"]
```

Unknown keys are errors; configured budgets are ceilings. In CI, restore and
save `.jevgate/cache` (and `.jevgate/latest.json` for change tracking) and inject
`TYPESAFE_API_KEY`. Cached answers from a pinned model version do not expire;
`jev-latest` and `jev-preview` answers expire after `--cache-ttl-secs`. An
unchanged unit is not sent again. Rate limits and overload are retried with
backoff (at most four attempts); account rejections stop further uploads.

## Limits

Parsers: Rust, Python, JavaScript and TypeScript. Files in other languages,
with syntax errors, or not UTF-8 are reported as skipped with a reason; they do
not make a run incomplete. Bodies under five lines are too small to judge and
never count as clear. A unit too large for one request is `needs-context`.
Clone candidates need two or more statements and 120 non-whitespace bytes;
at most 64 pairs per run and 8 per file are judged, and omissions are counted.
Probabilities are model judgments, not measured accuracy. Run linting,
formatting, tests and type checks separately.

Licensed under MIT OR Apache-2.0.
