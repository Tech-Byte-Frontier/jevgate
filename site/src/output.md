# Output and exit codes

| Format | Use |
|---|---|
| `agent` (default) | Ranked findings with locations and next steps, for people and coding agents |
| `json` | The full report: every file, finding, raw answer and probability, gate and usage |
| `jsonl` | One compact report per line; one per evaluation with `--watch` |
| `github` | GitHub Actions annotations and job summary, then the agent text |
| `sarif` | A SARIF 2.1.0 log for [GitHub code scanning](https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/uploading-a-sarif-file-to-github) and other SARIF readers: the findings the annotations show, `error` when they fail the gate |
| `gitlab` | A [GitLab Code Quality](https://docs.gitlab.com/ci/testing/code_quality/) report for merge requests: the same findings, `major` when they fail the gate and `minor` otherwise |

Agent output is colored on a terminal; `--color never`, or `NO_COLOR` set to any value, turns it off, and `--color always` or `CLICOLOR_FORCE` turns it on for pipes and logs.

Findings are `review` (act on it), `consider` (worth a look) or `note` (optional, shown with `--verbose`, never failing the gate). A file whose answers stay undecided is `uncertain`, and one that cannot be judged without more evidence is `needs-context`; neither is hidden or counted as clear. A finding's message shows the probability that set its level; a note shows none, and the JSON report keeps every raw value. Finished plans that share a directory are one finding. A hardcoded-value finding that cannot name its value is one level lower.

| Exit code | Meaning |
|---|---|
| 0 | Gate passed, or no supported file changed since `--base` |
| 1 | Gate failed |
| 2 | Run incomplete, invalid configuration or invalid usage |

`--fail-on review|consider|uncertain|none` sets what fails the gate; `--fail-on security=consider` sets it for one group or rule. Baselined findings, findings allowed by a comment, and notes never fail it.

A single finding can also be accepted where it is, with a comment on its line or directly above it (doc comments and attributes may sit in between). The comment names a rule ID, key or group, and needs a reason; without one it is ignored and the finding says so:

```python
# jevgate: allow(hardcoded_values) the protocol fixes this port
PORT = 4222
```

The report keeps the finding with its reason, it never fails the gate, and `jevgate baseline` leaves it out, so deleting the comment brings it back.

`jevgate baseline` can record why each finding was accepted: `intended` (right, and meant to be so), `later` (right, to fix later) or `wrong` (mistaken), with `--reason` or `jevgate baseline mark`. Reasons survive later rewrites of the baseline, and `jevgate baseline stats` reports each rule's share of findings marked wrong: labels from daily use, not the model's own probabilities.
