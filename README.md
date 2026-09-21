# JevGate

File-scoped code review using TypeSafe Jev. Three classifications:

- **File organization:** would separating unrelated responsibilities help?
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

Each selected file uses one request containing the three verdicts and their
conditional locations, plus focused function judgments and judgments of repeated
fragments when found. Direct function concerns retain the separate file-wide judgment.
Only that file and explicit context are uploaded; no automatic
repository retrieval. Directory arguments select files recursively. `--base` selects
changed files and reviews their current organization, not behavioral regressions.

Use `--dry-run --show-requests` to inspect uploads, `--cache-only` for offline replay,
and `--refresh` for fresh inference. `jevgate check --help` lists all options.
`jevgate rules` describes the three checks. Use `--rule` to select a subset.

`--classification-cascade --format json` enables an experimental test-role
comparison for shared logic in the same request. Inspect
`dimensions.shared_logic.refactoring_assessment.cascade` for raw overlapping
occurrence relationships, specialist judgments, selected branches and unresolved
conflicts; `files[].role_assessment` retains the four overlapping region roles.
Only resolved test–test groups select the test specialist. Mixed, omitted or
ambiguous relationships retain the general assessment. This shadow comparison
does not change findings or establish measured accuracy. It adds at most 160
role/evidence questions and 12 specialist questions per file in the same request.
Only regions containing repeated occurrences receive role questions in this mode.
Trailing regions are omitted when the combined request would exceed the provider
context limit, and that omission stays visible. An uncertain result can trigger
one follow-up on only the undecided operation or repeated lines; it replaces
that status only at the existing 0.80 threshold. The default classifier remains unchanged.

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
`--classification-cascade`, `--rule`, or `--report`.

For frozen role evaluations, `scripts/roles_eval.py` provides `freeze`, `run`, and
`summarize` commands (`--help`). Keep source copies, manifests, human-reviewed
labels and results in ignored `.jevgate/evaluation/`; labels never enter requests.
The report separates false positives, missed roles, abstentions, calibration,
language/repository groups and identical-source path comparisons. Reserve whole
repositories for holdout evaluation. The opt-in cascade uses these roles; adoption as the default requires measured
improvement in findings as well as role reliability.

## Configuration and CI

The nearest `jevgate.toml` or Git root defines the project boundary. For example:

```toml
upload_allow = ["src/**", "tests/**"]
upload_deny = ["src/private/**"]
generated = ["**/*.generated.*"]
max_file_bytes = 65536
```

Unknown configuration keys are errors. Upload boundaries and configured ceilings
also apply to explicit context. For CI, check changed files with `--base origin/main` and restore/save
`.jevgate/cache`. Set `--cache-ttl-secs` for the desired cache lifetime (default:
one hour). Inject `TYPESAFE_API_KEY`; local credentials
can use `jevgate auth` or an explicit `--env-file`. Never commit credentials.

Findings currently remain advisory; operational or incomplete runs exit 2.
Whether semantic findings should block CI belongs to the configuring user;
a built-in enforcement policy is not implemented yet.

## Limits

Results are `review`, `clear`, `uncertain`, `needs-context`, or `not-applicable`.
Probabilities are model judgments, not measured accuracy. A conditional location
does not establish a concern. Syntax supplies candidates, never semantic verdicts.
Candidates are capped at 64 operations and 240 pairs; omissions are reported.

Shared logic can miss repeated sections inside larger files. Run linting,
formatting, tests and type checks separately.

Licensed under MIT OR Apache-2.0.
