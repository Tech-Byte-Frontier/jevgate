---
name: findings
description: How to act on JevGate findings. Use when JevGate reports findings after an edit, blocks the end of a turn, says it could not check something, or when asked to run JevGate or read its report.
---

# Acting on JevGate findings

JevGate is a code-review gate. It asks TypeSafe Jev short, typed questions about units of code (a function, a file outline, a pair of copies, a test, a comment), and code composes the answers into findings. This plugin runs it at three points:

- **After each edit**, the findings on what the turn changed in the edited files arrive as context, one line each: `- path:line review rule (fails the gate): why Right 87% of the time (23 labels). Next: step`. A finding is given once a turn; later edits only count the ones already given.
- **At the end of a turn**, while a finding in what the turn changed is neither fixed nor dismissed, JevGate keeps you working with those findings as the reason: at most 3 times a turn, and not again when nothing changed since its last block.
- **When you ask**: the `jevgate_check` tool runs a check (`base: "HEAD"` covers your uncommitted changes), `jevgate_findings` reads the last report without running anything, and `jevgate_rules` says what each rule asks.

## What to do with a finding

Every finding is a `review`: a place worth a look, flagged broadly so that you verify it rather than trust it.

1. Read the code at `path:line` before changing anything. The why says what the finding rests on, and the sentence after it how often findings of its rule were right on projects JevGate was never tuned on ("Not yet measured." for broad look-here findings and when fewer than 20 were labeled); `Next` is a suggested step, not the only fix.
2. **Right**: fix it. A finding marked "(fails the gate)" also fails the person's CI.
3. **Mistaken, intended or for later**: dismiss it with a reason, `jevgate baseline mark wrong PATH:LINE` (or `intended`, or `later` with the issue that will fix it: `--note "#123"`). The dismissal counts at once, holds while the code it names is unchanged, and the person audits dismissals with `jevgate baseline stats`; `jevgate baseline list --reason later --format md` exports the `later` ones as a checklist. JevGate does not block again when nothing changed.

## Never, unless the person asks

- Edit `jevgate-baseline.json` by hand, `jevgate.toml` or the custom questions in `.jevgate/questions/`, or run `jevgate baseline` without `mark`: accepting findings wholesale is the person's decision.
- Add `jevgate: allow(RULE)` comments, delete or skip tests, or reword code only to change the answer.

Within a turn, none of these unblocks a finding: JevGate reads `jevgate.toml`, custom questions, the baseline and allow comments as they were when the turn began, except a dismissal with a reason, and reports each such edit to the person.

## When JevGate could not check

"JevGate could not check …" means nothing was reviewed: no API key (`jevgate auth login`), exhausted credits (HTTP 402), an outage, a time limit, another JevGate process in the repository, a directory outside Git, or no `jevgate` 0.27 or later on the PATH Claude Code runs hooks with. It never blocks you. Say so in your reply, and do not treat it as a pass.
