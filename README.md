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
