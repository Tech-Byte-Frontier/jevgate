# Hardcoded values

{{#include ../../reference/_rules.md:maintainability-hardcoded-values}}

## When a finding is right

A finding says a function, or a file's module-level constants, may hold a fixed value worth a look: one that singles out one specific record, place or user, differs between environments, is likely to change, or is an unexplained number that should have a name. It is right for a production host, a customer id or a price written where configuration belongs, or for a number whose meaning a reader must guess. It is wrong when the value explains itself or belongs where it is: messages, keys, field names, formats, small counts, values in named constants or explained by the code around them, and test data. A module-level constant already names its value, so only one that differs between environments or singles out one record is flagged there.

The rule is opt-in: on projects JevGate was never tuned on, 6 of the 37 labeled findings of the questions it replaced were right.

## How it is measured

A look-here finding has not been labeled yet, so it says `Not yet measured.` and never fails the check by default. Each one a coding agent or person dismisses with a reason (`jevgate baseline mark wrong|intended|later PATH:LINE`) is counted by `jevgate baseline stats`, which is how its share of noise shows in daily use.
