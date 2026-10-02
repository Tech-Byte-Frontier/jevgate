# Function simplification

{{#include ../../reference/_rules.md:maintainability-function-simplification}}

## When a finding is right

A finding says a function could likely be made noticeably simpler to read or change: it may be long, deeply nested or repetitive, or mix separate jobs. Two kinds of question ask it. The split and flatten Scores ask whether splitting the function into named functions, or flattening control flow nested four deep or chains of four branches, would make it easier to understand; their reviews name the block that would be most useful as a function of its own, and they are the rule's measured level, which fails the check by default. Every other function is asked a look-here question, which flags it for the person or coding agent reading the finding to verify, and says `Not yet measured.`. A finding is right when a reader would understand the function faster after the change: a handler that parses, queries, formats and notifies in one body, the same five lines written out once per case, or a loop nested four deep around the one line that matters. It is wrong when the length is one job done step by step, or when the nesting follows the shape of the data the code walks; dismiss those with `jevgate baseline mark wrong PATH:LINE`.

A split of a function of 20 lines or fewer is never a review, so only the look-here question flags one. In Bend 2, proofs are not asked to be split, and flattening proposes nested patterns instead of guard clauses.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: vaultwarden 28cab11abbf4bd53bbe09a19bcd765d5b0f48d66d600b673bdf2231d2a9b051b -->
### vaultwarden: `schedule_jobs`

- **Where:** [`src/main.rs:661`](https://github.com/dani-garcia/vaultwarden/blob/061694d0cb3bbf5d4c7e920c892824f0020cff83/src/main.rs#L661) in dani-garcia/vaultwarden at `061694d`.
- **Finding (review):** `schedule_jobs` mixes separate jobs in long blocks; splitting it would make it easier to understand.
- **Why it was wrong:** The function has one job: register the configured cron jobs and run the scheduler. Its length is nine three-line registrations, most under a comment, and a function per registration would only scatter the list.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0, and by 0.28.0 with the default rules. Function-simplification reviews fail the check by default, so this one fails vaultwarden's check. With every rule, 0.28.0 asks it beside the other rules' questions about its functions, and that answer does not report it; no change aimed at it.
