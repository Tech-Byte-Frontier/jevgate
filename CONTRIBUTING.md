# Contributing to JevGate

Issues and pull requests are welcome. For anything larger than a fix, open an issue first so we can agree on the shape before you write it.

## Reporting

- **A finding that is wrong or missing:** use the [wrong finding](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml) template. The finding from `.jevgate/latest.json` and a small piece of the code are what make it fixable.
- **A bug in the tool:** use the [bug report](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=bug_report.yml) template.
- **A vulnerability:** see [SECURITY.md](SECURITY.md); don't open a public issue.

## Building and testing

JevGate builds on Rust 1.90, its declared minimum, and on stable.

```sh
cargo fmt
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo +1.90.0 check --locked    # new syntax must build on the minimum version
```

The tests run offline and need no API key. `jevgate check --dry-run --show-requests` shows the exact requests a change produces without a key or network access.

## Code conventions

- Unused code is deleted, not silenced, and long parameter lists are grouped into a type. `tests/lint_policy.rs` rejects `allow` or `expect` for `dead_code`, `unused`, `too_many_arguments` and `complexity`. Any other exception uses `#[expect(lint, reason = "…")]`.
- Commit subjects say what changed, in the imperative ("Report overlapping tests by groups").
- Messages and findings are plain sentences that name the code and say what to do next.

## Changing rules and questions

JevGate asks the model small questions and decides findings in code. When you change a question, what a unit sends, or how answers become findings:

- Validate on small, frozen sets of real code, and compare with the previous binary on the same code. A shared answer cache keeps unchanged requests free.
- Keep probabilities and uncertainty visible. Don't lower the number of `uncertain` results by skipping units or moving thresholds.
- Check each new `review` or `consider` finding by hand.
- Bump the rule's version in `src/catalog.rs`.

[docs/classification-cascade.md](docs/classification-cascade.md) describes the evidence units and composition rules.

## Pull requests

- Keep a pull request to one change, with tests.
- Add a line under `Unreleased` in [CHANGELOG.md](CHANGELOG.md) for anything users will notice.
- CI must pass on stable and on Rust 1.90.

By contributing, you agree that your contributions are licensed under MIT OR Apache-2.0, like the project. Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).
