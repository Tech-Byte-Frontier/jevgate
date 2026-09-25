# Versions and stability

JevGate follows [Semantic Versioning](https://semver.org). This page says what the version number promises, so a CI job, a script or an agent knows what an upgrade can change.

## Before 1.0

Until 1.0, a minor release (0.18, 0.19, …) can change commands, flags, configuration and output formats, and a patch release (0.18.1) only fixes. Every change is in the [changelog](changelog.md), with what an upgrade re-asks. Pin a version in CI (`version:` in the action, `--version` for `cargo install`) and upgrade on purpose.

## From 1.0

A major release is needed to remove or change the meaning of:

- **Commands and flags**, and the values they accept.
- **Exit codes**: 0 gate passed, 1 gate failed, 2 run incomplete or invalid.
- **`jevgate.toml` keys, levels and rule names.** Unknown keys are errors, so removing a key or a rule name would break configurations.
- **The JSON report** (`--format json`, `.jevgate/latest.json`): its fields keep their names and meanings, and new fields can be added in any release. `schema_version` changes when a field is removed or changes meaning.
- **`jevgate-baseline.json`** and finding fingerprints: an upgrade must not make accepted findings new. A release that changes how findings are fingerprinted carries the baseline over.
- **SARIF, GitLab Code Quality and GitHub annotation output**, within what those formats define.

Deprecated flags and keys keep working for at least one minor release, with a warning on stderr, before a major release removes them.

## Not covered

These are judgments or presentation, and any release can change them; the changelog says how:

- **Which findings a rule reports**, their levels, wording, probabilities and next steps. Findings are model judgments composed by code, and improving them is most of what releases do. A rule's `version` changes when its questions or composition change, and its cached answers are asked again.
- **The agent text** (`--format agent`): it is written for people and coding agents to read. Scripts should read JSON.
- **Request bodies and the answer cache**: the cache is safe to delete or restore at any version; unmatched entries are simply not used.
- **The default model**: a release can pin a newer model version, which re-asks every unit once. Set `model` in `jevgate.toml` to keep one.

## Releases

Releases are batched: a minor release collects features and rule changes, and a patch release ships fixes without waiting. Each release publishes binaries, the crate, the GitHub Action's inputs and the Homebrew formula together, and this site is published from the same tag.
