<!-- jevgate:begin (written by `jevgate init --agent`; run it again to update this block, or add --remove to take it out) -->
## JevGate

JevGate reviews code as you edit it, through hooks. After an edit, its findings on what the turn changed in the edited files arrive as context, one line each: `path:line level rule: why Next: step`. At the end of a turn, it keeps you working while findings marked "(fails the gate)" remain, at most 3 times a turn.

- Fix findings marked "(fails the gate)" before you finish. Weigh the others: fix a `review` or `consider` finding when it is right, or leave the code and say why.
- If a finding is mistaken, keep the code as it is and say why in your reply; JevGate does not block again when nothing changed.
- Do not edit `jevgate-baseline.json` or `jevgate.toml`, add `jevgate: allow` comments, or delete or skip tests to clear a finding, unless the person asks: accepting a finding is their decision. Within a turn such edits do not unblock it: JevGate reads those files as they were when the turn began, and tells the person of each edit.
- "JevGate could not check …" means the code was not reviewed: say so in your reply, and do not treat it as a pass.
- `jevgate check --base HEAD` lists the findings in your uncommitted changes, and `.jevgate/latest.json` holds the last report.
<!-- jevgate:end -->
