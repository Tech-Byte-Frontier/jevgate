<!-- jevgate:begin (written by `jevgate init --agent`; run it again to update this block, or add --remove to take it out) -->
## JevGate

JevGate reviews code as you edit it, through hooks. When they run, a session starts with the line "JevGate's hooks run in this session". After an edit, its findings on what the turn changed in the edited files arrive as context, one line each: `- path:line level rule (fails the gate): why Right 87% of the time (23 labels). Next: step`, the mark only on findings that fail the gate, and the sentence before the next step saying how often findings of that rule and level were right on projects JevGate was never tuned on (`Not yet measured.` below 20 labels). At the end of a turn, it keeps you working while findings marked "(fails the gate)" remain, at most 3 times a turn.

If that line is not in this session, JevGate's hooks are not running for you (they may not be trusted yet, or this agent reads these instructions but not the hooks), and no silence from JevGate is a pass: before you finish, run `jevgate check --base HEAD` and act on its findings as below, or say that JevGate did not check your changes.

- Fix findings marked "(fails the gate)" before you finish. Weigh the others by how often findings like them were right: fix a `review` or `consider` finding when it is right, or leave the code and say why.
- If a finding is mistaken, keep the code as it is and say why in your reply; JevGate does not block again when nothing changed.
- Do not edit `jevgate-baseline.json`, `jevgate.toml` or the custom questions in `.jevgate/questions/`, add `jevgate: allow` comments, or delete or skip tests to clear a finding, unless the person asks: accepting a finding is their decision. Within a turn such edits do not unblock it: JevGate reads those files as they were when the turn began, and tells the person of each edit.
- "JevGate could not check …" means the code was not reviewed: say so in your reply, and do not treat it as a pass.
- `jevgate check --base HEAD` lists the findings in your uncommitted changes, and `.jevgate/latest.json` holds the last report.
<!-- jevgate:end -->
