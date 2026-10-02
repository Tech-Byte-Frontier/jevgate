# File organization

{{#include ../../reference/_rules.md:maintainability-file-organization}}

## When a finding is right

A finding says a file may do several separate kinds of work, such as separate features, layers or integrations, that a maintainer could keep in separate files; for a test file, that it may test several separate subjects. It is a look-here flag: Jev reads the file's outline (each member's signature, the names it calls and its size, without bodies, or a test file's cases with their suites and subjects), and the person or coding agent reading the finding opens the code before moving anything. It is right when a part is a feature or a job a reader would look for apart from the rest: a URL scraper inside a model, or a diff engine inside a renderer. It is wrong when the file holds one feature, type, resource, screen or job and its helpers, even when it is long, or when the same kind of code is written out for each case, such as one handler per message type.

A file of fewer than 100 non-blank lines is not asked, nor a Bend 2 file of fewer than 300.

## How it is measured

A look-here finding has not been labeled yet, so it says `Not yet measured.` and never fails the check by default. Each one a coding agent or person dismisses with a reason (`jevgate baseline mark wrong|intended|later PATH:LINE`) is counted by `jevgate baseline stats`, which is how its share of noise shows in daily use.
