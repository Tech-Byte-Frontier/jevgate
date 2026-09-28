# Versions and stability

JevGate follows [Semantic Versioning](https://semver.org). This page says what the version number promises, so a CI job, a script or an agent knows what an upgrade can change.

## Before 1.0

Until 1.0, a minor release (0.18, 0.19, …) can change commands, flags, configuration and output formats, and a patch release (0.18.1) only fixes. Every change is in the [changelog](changelog.md), with what an upgrade re-asks. Pin a version in CI (`version:` in the action, `--version` for `cargo install`) and upgrade on purpose.

## From 1.0

A major release is needed to remove or change the meaning of:

- **Commands and flags**, and the values they accept.
- **Exit codes**: 0 gate passed, 1 gate failed, 2 run incomplete or invalid; for `rules test`, 0 every example right, 1 a question got one wrong, 2 incomplete or invalid.
- **`jevgate.toml` keys, levels and rule names**, and the keys of question files in `.jevgate/questions/`. Unknown keys are errors, so removing a key or a rule name would break configurations.
- **The JSON report** (`--format json`, `.jevgate/latest.json`): its fields keep their names and meanings, and new fields can be added in any release. `schema_version` changes when a field is removed or changes meaning.
- **`jevgate-baseline.json`** and finding fingerprints: an upgrade must not make accepted findings new. A release that changes how findings are fingerprinted carries the baseline over.
- **SARIF, GitLab Code Quality and GitHub annotation output**, within what those formats define.
- **The MCP tools' structured results**: the fields each tool's output schema declares keep their names and meanings, and new fields can be added in any release.

Deprecated flags and keys keep working for at least one minor release, with a warning on stderr, before a major release removes them.

## Not covered

These are judgments or presentation, and any release can change them; the changelog says how:

- **Which findings a rule reports**, their levels, wording, probabilities, measured precision and next steps. Findings are model judgments composed by code, and improving them is most of what releases do. They can also depend on the other rules selected: a function's questions are asked with every selected rule's questions about it, so selecting hardcoded values or a security rule can move a function-simplification finding near a threshold. A rule's `version` changes when its questions or composition change; a changed question is asked again, and the rule's other answers still come from the cache.
- **Which rules and levels fail the check by default.** The default level, `mature`, follows the labeled findings: a release marks a rule's reviews or considers mature once they are right at least 80% of the time on projects JevGate was never tuned on, over at least 20 labels, and can drop one that stops measuring up. The changelog gives the numbers, and [accuracy](accuracy.md) the table in force. Set `fail_on`, or a level per rule, to keep a fixed policy.
- **Which rules run by default.** A rule can leave the default group, as hardcoded values did in 0.26, or join it; `--rule` and `[rules]` keep an explicit selection.
- **The agent text** (`--format agent`): it is written for people and coding agents to read. Scripts should read JSON.
- **The questions an undecided unit left open**, as the JSON report and the MCP verify items quote them: their wording, answers and the state paths they name change when a rule's questions do.
- **Request bodies and the answer cache**: the cache is safe to delete or restore at any version; unmatched entries are simply not used.
- **The default model**: a release can pin a newer model version, which re-asks every unit once. Set `model` in `jevgate.toml` to keep one.
- **The [question gallery](question-gallery.md)**: which questions it holds, and their wording, levels and thresholds, change as they are measured. A question file `jevgate rules add` wrote keeps its wording until `--force` replaces it.

## Reruns of an unchanged commit

A check asks Jev only what its answer cache cannot answer. The cache keeps each answer under a hash of what it was asked about and of its question: the unit's source and evidence, the question and the model. With a TypeSafe key the default model is a pinned version, `jev-1.13.0`, whose answers never expire, and code, not the model, turns the answers into findings. So with the same version and cache, a rerun of an unchanged commit sends no request, costs nothing and reports the same findings, down to each answer's probabilities.

[`rerun.sh`](rerun.sh) shows it on your repository. It runs `jevgate check` twice with the arguments you give it, prints each run's headline, and compares the two reports: whether each run finished, the gate, and each file's status, findings and raw answers. It needs `jq`, and exits 0 when the rerun sent no request and matched, 1 when it sent requests or differed, and 2 when a check did not finish.

```sh
curl -fsSLO https://tech-byte-frontier.github.io/jevgate/rerun.sh
sh rerun.sh --rule all --include-tests
```

On [zoxide](https://github.com/ajeetdsouza/zoxide/tree/09a18b4424b3f1033094ffd97da6d47585e38259), whose answers an earlier check had cached:

```text
JevGate: review · gate failed: 1 new review finding · 33 files · 0 API requests · 0 input tokens · ~$0.0000
JevGate: review · gate failed: 1 new review finding · 33 files · 0 API requests · 0 input tokens · ~$0.0000
The rerun sent no request and matched: 40 findings (4 reviews, 8 considers, 28 notes) and 1472 answers in 33 files, with the same levels, lines and probabilities.
```

On 14 open-source projects in 9 languages (2,839 files, 3,870 findings, 129,403 answers), every rerun sent no request and matched, and a check answered from the cache took 0.3 to 3.3 seconds.

A rerun asks Jev again, and its findings can change, when:

- **A release changes a rule's questions or composition.** The [changelog](changelog.md) says what an upgrade asks again. `--refresh` skips the cache on purpose.
- **`model` names an alias**, such as `jev-latest`, rather than a version, as the default models of OpenRouter and Vercel AI Gateway keys do: its answers expire after `cache_ttl_secs`, an hour by default.
- **The cache is missing**, in a fresh clone or a CI job without the cache step. [Continuous integration](ci.md) keeps `.jevgate/cache` between runs.
- **The first run did not finish.** What it could not ask is asked on the rerun.
- **A unit is at the edge of the provider's size limit and the token calibration moved.** Each run that sends requests updates `.jevgate/token-budget.json`, which planning reads to tell whether a unit fits in one request. On the 14 projects above, checks with the default calibration and with the saved one matched.

## Releases

Releases are batched: a minor release collects features and rule changes, and a patch release ships fixes without waiting. Each release publishes binaries, the crate, the GitHub Action's inputs and the Homebrew formula together, and this site is published from the same tag.
