# Output and exit codes

| Format | Use |
|---|---|
| `agent` (default) | Ranked findings with locations and next steps, for people and coding agents |
| `json` | The full report: every file, finding, raw answer and probability, gate and usage |
| `jsonl` | One compact report per line; one per evaluation with `--watch` |
| `github` | GitHub Actions annotations and job summary, then the agent text |
| `sarif` | A SARIF 2.1.0 log for [GitHub code scanning](https://docs.github.com/en/code-security/code-scanning/integrating-with-code-scanning/uploading-a-sarif-file-to-github) and other SARIF readers: the findings the annotations show, `error` when they fail the gate, with how the gate counted each as its `gate` property and how often its rule and level were right as `precision`; each rule's help links its page on this site |
| `gitlab` | A [GitLab Code Quality](https://docs.gitlab.com/ci/testing/code_quality/) report for merge requests: the same findings, `major` when they fail the gate and `minor` otherwise |

Agent output is colored on a terminal; `--color never`, or `NO_COLOR` set to any value, turns it off, and `--color always` or `CLICOLOR_FORCE` turns it on for pipes and logs.

The first line, also the title of the GitHub job summary, sums up the run: its status, the gate and why it failed, the files, what a `--base` check judged (`changed lines since 1a2b3c4`, or `whole files changed since 1a2b3c4` with `--whole-files`; the report's `scope` records it; `staged lines since 1a2b3c4` for `--staged`, whose report says `"staged": true`, and `changed lines from 1a2b3c4 to 9f8e7d6` for `--pre-push`, whose report names the pushed commit in `pushed_revision`), the API requests (`via OpenRouter` or `via Vercel AI Gateway` when a gateway answered), the input tokens and the estimated cost, which is `cost unknown` when a response reported no token usage or the model that answered has no known price.

Findings are `review` (act on it), `consider` (worth a look) or `note` (optional, shown with `--verbose`, never failing the gate; a [custom question](custom-questions.md)'s notes are always listed, since a team keeps a question a note while it tries it). A file whose answers stay undecided is `uncertain`, and one that cannot be judged without more evidence is `needs-context`; neither is hidden or counted as clear. A unit the parser could not read is listed after the findings as `path:line unit: reason` (the first 10 unless `--verbose`; with `--base`, only where the change touched the file), and in the JSON report under its file's `left_out`: the unit (a function, method, type, law or test by name, `outline`, or none for code outside every unit), its `start_line` and `end_line`, and the `reason`, such as "The Swift parser could not read line 4.". Before that list, the agent text names the files of the [preview languages](languages.md#support-levels) in the run, by language, with the rules that read them and their test files not judged yet. Each review and consider ends with how often findings of its rule and level were right on projects JevGate was never tuned on, counted from labels as `jevgate rules` counts them: `Right 87% of the time (23 labels).`, or `Not yet measured.` below 20 labels, as a custom question's findings always say; a finding in a preview language counts that language's own labels and names it (`Not yet measured in Kotlin.`); a law finding adds that it was labeled only on Bend 2 projects, which the table leaves out. That replaces the probability of the answer that set its level, which says how sure that one answer was, not how often such findings are right; the JSON report keeps it as `concern_probability`, beside `precision` (`right` of `labeled`; in a preview language's file that language's own counts, which `preview` names) and every raw answer. For each undecided unit, the JSON report also keeps where it is and each question it left open as it was asked, with what each answer means and the probabilities Jev gave them (`dimensions.*.undecided[].open`). Finished plans that share a directory are one finding. A hardcoded-value finding that cannot name its value is one level lower.

| Exit code | Meaning |
|---|---|
| 0 | Gate passed, or no supported file changed since `--base`; also a run that could not finish when `on_incomplete` passes it, as it does by default for `--staged` and `--pre-push`, with a line on stderr saying the change was not checked, and why |
| 1 | Gate failed |
| 2 | Run incomplete, invalid configuration or invalid usage |

`jevgate hook` is the exception: it exits 0 whatever happens, because agents read exit 2 as "block", and its JSON reply says what happened ([Coding agents](coding-agents.md)).

While a check runs on a terminal, one line on stderr says what it is doing, how many of that stage's requests are answered and for how long it has run: `JevGate · first pass · 312/768 answered · 23s`. It is erased before anything else is printed, and it is not drawn in CI, with `--watch` or `--format jsonl`, or when stderr is not a terminal.

After the findings, the agent text gives each reason files failed or were skipped, with how many files give it, such as `Failed 3: TypeSafe HTTP 402 (credits exhausted; …)`, so an incomplete run says why without `--verbose`, and so does the MCP server's `jevgate_check`, which returns this text.

`--fail-on review|consider|mature|uncertain|none` sets what fails the gate; `--fail-on security=consider` sets it for one group or rule. The default, `mature`, fails only on the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on, never on a finding in a [preview language](languages.md#support-levels), and on each [custom question](custom-questions.md)'s own level; `jevgate rules` lists them, and [configuration](configuration.md#what-fails-the-check-by-default) explains it. Baselined findings, findings allowed by a comment, and notes never fail the gate.

The agent text marks each finding that fails the gate with `(fails the gate)`, or `(would fail the gate)` in a run whose gate was not evaluated, such as one left incomplete, and says when reviews did not fail it because their rules are still being measured or their files' languages are in preview. The JSON report records how the gate counted each new finding in its `gate` field: `fails`, `measuring` (reported without failing: the level is `mature` and its rule and level are still being measured, or its file's language is in preview) or `advisory` (the level in force does not count it, as `review` does not count a consider). `fail_on_mature` says what `mature` stands for among the selected rules.

A single finding can also be accepted where it is, with a comment on its line or directly above it (doc comments and attributes may sit in between). The comment names a rule ID (`security/injection`), its name (`injection`), its key or a group (a [custom question](custom-questions.md) by its ID, `custom/no-body-logs`, or `custom`), and needs a reason; without one it is ignored and the finding says so:

```python
# jevgate: allow(hardcoded_values) the protocol fixes this port
PORT = 4222
```

The report keeps the finding with its reason, it never fails the gate, and `jevgate baseline` leaves it out, so deleting the comment brings it back.

`jevgate baseline` can record why each finding was accepted: `intended` (right, and meant to be so), `later` (right, to fix later) or `wrong` (mistaken), with `--reason` or `jevgate baseline mark`. Reasons survive later rewrites of the baseline, and `jevgate baseline stats` reports each rule's share of findings marked wrong: labels from daily use, not the model's own probabilities.

## Guards

A check with `--base`, and each check of the [agent hook](coding-agents.md#in-the-agents-loop-jevgate-hook), also reports what the change does to the checks around the code. Code finds most of them; Jev is asked only about the evidence code selected:

| Guard | When |
|---|---|
| `suppression` | a line the change adds turns off another tool: `# noqa`, `# type: ignore`, `eslint-disable`, `@ts-ignore`, `@ts-expect-error`, `#[allow(…)]`, `//nolint`, `@SuppressWarnings`, `rubocop:disable`, `# nosec`, `# pragma: no cover` and about 50 more |
| `allow` | a line the change adds is a `jevgate: allow` comment |
| `skipped-test`, `focused-test` | a test file gains `it.skip`, `xit`, `@pytest.mark.skip`, `#[ignore]`, `t.Skip`, `@Disabled`, `markTestSkipped` or RSpec's `skip`; or `.only` or `fit`, which skip every other test |
| `deleted-test` | a test is gone and no file of the change gained a test of that name, or a test file is deleted. Tests are located in the ten supported languages only: a test removed from a preview language's file, such as a Kotlin or Swift test, is not reported yet, though its skip markers are |
| `weaker-assertion` | a test whose assertion lines the change removed or rewrote, which Jev reads at 0.80 as checking less than before; it is asked with the test before and after and the functions of its file the new version newly calls |
| `configuration`, `baseline` | `jevgate.toml` or `jevgate-baseline.json` is added, deleted or edited: the keys that changed, the findings accepted, dropped or given another reason, or that it does not parse |
| `question` | a [custom question](custom-questions.md) file of `.jevgate/questions/`, or a `[[question]]` table of `jevgate.toml` by its `id`, is added, deleted or edited: the keys that changed, such as its `level`, `threshold`, `question` or `guidance`, or that it no longer loads |
| `skipped-file` | a file of code JevGate judged before the change and skips after it: it now reads as generated code (`// @generated`, `DO NOT EDIT` among its first comments) or a copied library, grew past `max_file_bytes`, is no longer UTF-8, or no longer parses |
| `cache` | files of `.jevgate/cache` the change commits: a check never reads a cache file Git tracks, since a change could commit answers that clear its own code |
| `steering` | a comment, string or document paragraph Jev reads at 0.80 as written to steer a reviewer, on any check; no unit asked in a request that sent it can clear. In documents only text addressed to a reviewer, a scanner or JevGate is asked about, since instruction files address AI agents throughout |

A moved or renamed line adds nothing, unless it is a comment, attribute or decorator that now sits above other code (a `jevgate: allow` comment or a skip marker moved or copied to another function applies to code it did not before), and neither does a marker quoted in a string, named in a comment (a skip marker; in Markdown, anything outside an HTML comment) or read by another language's tools (`# noqa` in Rust); generated code, type declarations, migrations and test data are left out. Guards follow the findings (`Guards (N):`, the first ten; `--verbose` shows all), and are `guards` in the JSON report (kind, path, line, text, message, probability, id), in the same shape in the MCP tools' results, and GitHub notices with a list in the job summary; SARIF and GitLab reports carry findings only. They never fail the gate, and neither the baseline nor an allow comment accepts them: they are facts for a person to look at, most of them legitimate.

On the last five commits of 142 corpus projects, 129 guards were reported (75 suppressions, 50 removed tests, 4 skipped tests), each checked against Git, and the scan adds 29 ms at the median to a check of a last commit.
