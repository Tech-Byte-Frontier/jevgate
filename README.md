# JevGate

File-scoped code review using TypeSafe Jev. Three classifications:

- **File organization:** would separating independently useful capabilities help?
- **Function simplification:** is there a useful task to extract or control flow to simplify?
- **Shared logic:** should repeated implementations share a helper?

## Use

```sh
# Install the CLI
cargo install jevgate --locked

# Save your TypeSafe API key
jevgate auth login

# Review a file and open the results in your browser
jevgate check src/example.ts --report

# Recheck the file after edits and refresh the report
jevgate check src/example.ts --watch --report

# Include a related file to help interpret the selected file
jevgate check src/example.ts --context src/peer.ts

# Review files changed since the latest commit and output JSON
jevgate check --base HEAD --format json
```

`--report` opens a local HTML dashboard with classifications, locations and raw
probabilities. Watch refreshes it after edits. Opening the report makes no API calls.

Each application file uses one request containing the three verdicts and their
conditional locations, plus focused function judgments and judgments of repeated
fragments when found. The file-wide answer keeps the status. A function score
is a place to inspect, and a follow-up asks whether that function has a
validation, parsing, or delivery task to extract. `none` means the length is
the work itself.
Operational scripts and TypeScript declaration files are reported and not judged.
Test files are not judged unless you pass `--include-tests`. A file that mixes
application code and tests is judged on the application portion; with that flag,
the test portion is judged separately. A test path that still contains other
code is classified before the gates, and that classification is part of the
gate request.

Only that file and explicit context are uploaded; no automatic
repository retrieval. Directory arguments select files recursively. `--base` selects
changed files and reviews their current organization, not behavioral regressions.

Use `--dry-run --show-requests` to inspect uploads, `--cache-only` for offline replay,
and `--refresh` for fresh inference. `jevgate check --help` lists all options.
`jevgate rules` describes the three checks. Use `--rule` to select a subset.

Shared logic includes the role cascade in the same request. Inspect
`dimensions.shared_logic.refactoring_assessment.cascade` for raw overlapping
occurrence relationships, specialist judgments, selected branches and unresolved
conflicts; `files[].role_assessment` retains the four overlapping region roles.
Only resolved test–test groups select the test specialist. Mixed, omitted or
ambiguous relationships retain the general assessment. The specialist comparison
does not replace that general verdict. It adds at most 160 role/evidence
questions and 12 specialist questions per file in the same request. Only regions
containing repeated occurrences receive role questions. Trailing regions are
omitted when the combined request would exceed the provider context limit, and
that omission stays visible. An uncertain result triggers one follow-up on only
the undecided operation or repeated lines; it replaces that status only at the
existing 0.80 threshold.

Use `jevgate check src/example.rs --roles-only` to evaluate semantic roles without
maintainability judgments. It defaults to JSON and reports overlapping test
scenario, test support, framework/tool, and application/library roles under
`files[].role_assessment`. Each of up to 32 deduplicated regions has raw yes/no
probabilities and a separate evidence-sufficiency answer. Repeated occurrences
reference their region and retain ambiguous or overlapping relationships.
Complete region excerpts share a 16 KiB budget; omitted excerpts are reported
and the complete selected source remains available. Test support means a fixture
or helper provider; arranging inputs inside a scenario remains scenario behavior.
Parameterized templates that own the tested operation and expected-result checks
are scenarios even when reused.
Omissions and unsupported parsers stay visible; `classified` means roles were
resolved, not that code quality passed. `--roles-only` cannot be combined with
`--rule` or `--report`.

For frozen role evaluations, `scripts/roles_eval.py` provides `freeze`, `run`, and
`summarize` commands (`--help`). Keep source copies, manifests, human-reviewed
labels and results in ignored `.jevgate/evaluation/`; labels never enter requests.
The report separates false positives, missed roles, abstentions, calibration,
language/repository groups and identical-source path comparisons. Reserve whole
repositories for holdout evaluation. The shared-logic cascade uses these roles.

## Configuration and CI

The nearest `jevgate.toml` or Git root defines the project boundary. For example:

```toml
upload_allow = ["src/**", "tests/**"]
upload_deny = ["src/private/**"]
generated = ["**/*.generated.*"]
max_file_bytes = 131072
```

Unknown configuration keys are errors. Upload boundaries and configured ceilings
also apply to explicit context. For CI, check changed files with `--base origin/main` and restore/save
`.jevgate/latest.json` together with `.jevgate/cache`. An unchanged file is
reused from the last report, so a repeat check does not call the API.
`--refresh` judges those files again. The cache still answers a repeated
request for one hour (`--cache-ttl-secs`). Inject `TYPESAFE_API_KEY`; local credentials
can use `jevgate auth` or an explicit `--env-file`. Never commit credentials.

Process exit is the quality gate: `0` is clear or not-applicable, `1` is review,
`2` is uncertain, needs-context or incomplete. Findings stay advisory in the
report; CI can treat `1` as the block and `2` as a failed gate run.

## Limits

Results are `review`, `clear`, `uncertain`, `needs-context`, or `not-applicable`.
Probabilities are model judgments, not measured accuracy. A conditional location
does not establish a concern. Syntax supplies candidates, never semantic verdicts.
Candidates are capped at 64 operations and 240 pairs; omissions are reported.
A shared-logic finding quotes the repeated text from each site. A match shorter
than 120 bytes stays a probability and is not printed as a finding.

The default read cap is 128 KiB. A file above it, or whose complete source does
not fit one request beside the maintainability questions, is `needs-context`.
The report includes its byte size and the operation names the parser found. That
result is not a file-organization finding, the source is not truncated, and the
other files are still judged. A lower `max_file_bytes` only narrows the cap.

Shared logic can miss repeated sections inside larger files. Run linting,
formatting, tests and type checks separately.

Licensed under MIT OR Apache-2.0.
