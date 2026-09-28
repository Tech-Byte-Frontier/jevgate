# Code comments

{{#include ../../reference/_rules.md:documentation-comments}}

## When a finding is right

A finding says a comment only repeats its code, holds sentences that add nothing, narrates an edit instead of describing the code as it is, or is code turned off. It is right for `# Create skill directory` above `skill_dir.mkdir(…)`, or a docstring saying a module was split out to stay under a line budget. It is wrong when the comment heads a step of a long function, gives a reason, or, in a teaching project, is the lesson itself. A third of the findings labeled wrong or debatable outside Bend 2 code were step headings.

A comment still undecided once its kind is asked leans on the kind, and the comments of one definition that span fewer than three lines in all are a note.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: linkace 2f21b36ef74fec23543332a3db926bf88f046c6543a278faf57ecbf22f6d1ebb -->
### LinkAce: `config/auth.php`

- **Where:** [`config/auth.php:95`](https://github.com/Kovah/LinkAce/blob/d6821661fb5878850738dc5f3d3593799d89445f/config/auth.php#L95) in Kovah/LinkAce at `d682166`.
- **Finding (consider):** This file's top-level code has a comment to clean up: at lines 95–98 it repeats the code.
- **Why it was wrong:** The lines are the Laravel skeleton's own example of a `database` user provider beside the active `eloquent` one, under a header listing both drivers. The framework publishes the file with its documentation; removing its examples gains nothing.
- **Since:** cleared in 0.20.0, which no longer reads a Laravel application's `config/*.php` files for comments ([changelog](../../changelog.md#0200---2026-09-26)).

<!-- example: httpx c5deabd166b91a745d8a288ffec8c56da95ce3d765c4bb0fb186524de68e6481 -->
### httpx: `urlparse`

- **Where:** [`httpx/_urlparse.py:234`](https://github.com/encode/httpx/blob/b5addb64f0161ff6bfe94c124ef76f6a1fba5254/httpx/_urlparse.py#L234) in encode/httpx at `b5addb6`.
- **Finding (consider):** `urlparse` has 5 comments to clean up: at lines 234, 244, 250 and 284 they repeat the code; at line 239 it narrates an edit instead of the code as it is.
- **Why it was wrong:** The comments head the steps of a 130-line function divided by comment banners. Line 239, `# Replace "netloc" with "host and "port".`, says what the code does to its arguments when it runs, not a past edit.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.

<!-- example: nodegoat 9602f5e0cd67c3230e067adc98222724be0cca64bf55fae1b7ef02bb9f4e26a7 -->
### NodeGoat: the route table

- **Where:** [`app/routes/index.js:40`](https://github.com/OWASP/NodeGoat/blob/c5cb68a7084e4ae7dcc60e6a98768720a81841e8/app/routes/index.js#L40) in OWASP/NodeGoat at `c5cb68a`.
- **Finding (consider):** `index` has 3 comments to clean up: at lines 40 and 78 they repeat the code; at lines 57–60 it is code turned off.
- **Why it was wrong:** NodeGoat teaches web security. Lines 57–60 are the commented-out "Fix for A7", the routes with the missing role check, kept as the exercise's answer; lines 40 and 78 head entries of a route table like the others.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.
