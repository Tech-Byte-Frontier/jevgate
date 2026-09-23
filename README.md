# JevGate

Code review with TypeSafe Jev over small evidence units. Six rules by default:

- **File organization:** would moving some members into a separate module make the file easier to understand?
- **Function simplification:** would splitting a function into named functions make it easier to understand, or, for deeply nested code, would flattening it help?
- **Shared logic:** do renamed or exact copies perform the same steps for the same purpose?
- **Hardcoded values:** does a value fixed in code need to change in another environment, need a descriptive name, or special-case one user, account or record?
- **Test value** (`--include-tests`): does a test only check its mocks, recompute its expected value, assert internal details or mix unrelated behaviors?
- **Test redundancy** (`--include-tests`): do similar tests of one function check the same behavior?

Three opt-in security rules (`--rule security`):

- **Injection:** does a variable another party controls reach SQL, a shell command, evaluated code, markup, a file path or a URL without being bound, escaped or checked?
- **Sensitive data:** does code log a password, token, key or personal data, or send internal error details to a remote client?
- **Unsafe settings:** does code turn off certificate verification, hash passwords weakly, make secrets with a non-cryptographic random generator, allow credentialed requests from any origin, or set session cookies without Secure or HttpOnly?

## Use

```sh
cargo install jevgate --locked
jevgate init          # writes jevgate.toml: detected sources, rule groups
jevgate auth login

# Review a directory and open a local dashboard
jevgate check src --report

# Include tests; judge changed files only; machine-readable output
jevgate check --include-tests --base origin/main --format json

# Inspect what would be uploaded, with planned requests and tokens per stage
jevgate check src --dry-run --show-requests
```

`jevgate rules` lists the rules and their groups (`--format json` for the
catalog). `--rule` and `--skip-rule` take a rule ID, key or group
(`maintainability`, `tests`, `default`, `all`); without them the `default` group
runs. `--context` adds a related file as evidence for shared logic, callers and
test subjects.

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

Security units are the application functions that call something, build text
or assign a field, plus each file's top-level setup statements for unsafe
settings. The parser lists each unit's statements as sites (calls, built text,
field assignments) only so a finding can be located. A packed first pass asks
whether the code places a variable into interpreted text or a path or URL,
logs secrets or exposes error details, or weakens a setting; a unit whose
answer is not clear gets one trace with specific checks per kind (SQL, shell,
code, markup, path, URL; TLS, hashing, randomness, CORS, cookies; logged
secrets, exception text in responses), the site, and where the values come
from or whether the code runs only in development. Values from another party
are a review; parameters of unknown origin are a consider for SQL, shell, code
and markup and a note for paths and URLs; their origin and checks are asked
again with up to three callers. Findings name a CWE in `category`.

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
`fail_on` in `jevgate.toml`) decides what fails the check; `--fail-on
TARGET=LEVEL` sets it for one rule or group, such as `tests=consider`. The
command line wins over the file, and a rule's own level over its group's. `consider` also fails
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

[rules]                      # level per group or rule; the rule's own entry wins
maintainability = "review"
tests = "consider"
"maintainability/hardcoded-values" = "off"   # or "report": judge, never fail
```

`rules = ["maintainability"]` (a list) selects rules without levels. Unknown keys are errors; configured budgets are ceilings. In CI, restore and
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
Security rules judge one function and at most one hop of callers: there is no
whole-program data flow, and SQL files, row-level policies and access control
are not judged. Secrets in source are out of scope; use a local secret
scanner, since judging them would upload them.
Probabilities are model judgments, not measured accuracy. Run linting,
formatting, tests, type checks and security scanners separately.

Licensed under MIT OR Apache-2.0.
