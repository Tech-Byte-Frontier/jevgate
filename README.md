# JevGate

Code review with TypeSafe Jev over small evidence units. Six rules:

- **File organization:** would moving some members into a separate module make the file easier to understand?
- **Function simplification:** would splitting a function into named functions make it easier to understand, or, for deeply nested code, would flattening it help?
- **Shared logic:** do renamed or exact copies perform the same steps for the same purpose?
- **Hardcoded values:** does a value fixed in code need to change in another environment, need a descriptive name, or special-case one user, account or record?
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

Local analysis runs first and uploads nothing: parsers find functions, methods,
types and callbacks registered through calls (`const view = db.view(opts, (ctx) => …)`);
measure control-flow nesting; group a file's members by calls, owners and shared
types; list callers that import each file; find Type-2 clone candidates
(identifiers and literals normalized, whole statements, consistent renaming) and
merge overlapping copies into one group; and map tests to the functions they call.
Generated files (`@generated`, "DO NOT EDIT" headers) are skipped.

Each request asks short questions about one unit: a pack of up to eight
functions, one file outline (signatures, callers and groups, no bodies), one clone
group's representative pair, one test, or one pair of tests. Hardcoded values are
asked per function with the literal values it uses (0, 1, 2, one-character strings,
documentation and attributes are skipped) and per file for its module-level
constants; they get no recheck, because per-value rechecks added false findings. Flattening is asked
only for control flow nested four deep or four-branch chains. A unit that stays
uncertain gets one recheck with more evidence (callee signatures, the enclosing
functions, or the file's source for an outline).
Code composes the answers at a 0.80 threshold into `review` (top level),
`consider` (middle-or-top mass), `clear` (top level ruled out) or `uncertain`,
and ranks findings by probability × ln(1 + lines). Where the middle level says
the code reads well as it is (splitting, flattening, moving members), a
`consider` also needs the top level at 0.50 or more; middle mass alone is an
optional `note`. A split function finding gets one follow-up Choice among the
body's top-level blocks, and the chosen block becomes its first location.
Every `review` carries a finding. Raw answers are kept under `files[].judgments`.

Test files are judged only with `--include-tests`. A file that mixes code and
tests keeps them apart: application rules judge the code, test rules the tests.
A test path with structural tests (including Python `unittest` classes and pytest
functions) is a test file; one without any gets one file-purpose question first.
Copies whose every site is inside test cases are one level lower (review becomes
consider, consider becomes note): spelling out each case is idiomatic in tests.
A file split is suggested only when the proposed group has callers of its own in
other selected files; when no member has known callers (an entry point, or a
partial scope), the answer stands.

## Gate, baseline and exit codes

`--fail-on review|consider|uncertain|none` (repeatable, default `review`, or
`fail_on` in `jevgate.toml`) decides what fails the check. `consider` also fails
on review findings. Notes never fail the gate; the agent output counts them and
`--verbose` lists them. Exit codes: `0` pass, `1` gate failed, `2` incomplete or error.

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
backoff (at most four attempts); account rejections stop further uploads. A
request blocked by the provider's edge firewall fails alone; three blocks in a
row stop further uploads, and a rerun resumes from the cache.

## Limits

Parsers: Rust, Python, JavaScript and TypeScript. Files in other languages,
with syntax errors, or not UTF-8 are reported as skipped with a reason; they do
not make a run incomplete. Bodies under five lines are too small to judge and
never count as clear; files under 100 lines of member code are too small to
split. A unit too large for one request is `needs-context`. Clone candidates
need three or more statements and 120 non-whitespace bytes; at most 64 groups
per run and 8 per file are judged, and omissions are counted.
Probabilities are model judgments, not measured accuracy. Run linting,
formatting, tests and type checks separately.

Licensed under MIT OR Apache-2.0.
