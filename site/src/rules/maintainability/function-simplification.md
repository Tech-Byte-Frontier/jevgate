# Function simplification

{{#include ../../reference/_rules.md:maintainability-function-simplification}}

## When a finding is right

A finding says a function mixes separate jobs in long blocks, or that its branching hides its main path, and it names the block that would be most useful as a function of its own. It is right when a reader would understand the function faster with that block named: a handler that parses, queries, formats and notifies in one body, or a loop nested four deep around the one line that matters. It is wrong when the length is one job done step by step, or when the nesting follows the shape of the data the code walks. More than a third of the findings labeled wrong or debatable outside Bend 2 code were linear code that a split would only scatter.

Splitting a function of 20 lines or fewer is at most a note. In Bend 2, proofs are not asked to be split, and flattening proposes nested patterns instead of guard clauses.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: vaultwarden 28cab11abbf4bd53bbe09a19bcd765d5b0f48d66d600b673bdf2231d2a9b051b -->
### vaultwarden: `schedule_jobs`

- **Where:** [`src/main.rs:661`](https://github.com/dani-garcia/vaultwarden/blob/061694d0cb3bbf5d4c7e920c892824f0020cff83/src/main.rs#L661) in dani-garcia/vaultwarden at `061694d`.
- **Finding (review):** `schedule_jobs` mixes separate jobs in long blocks; splitting it would make it easier to understand.
- **Why it was wrong:** The function has one job: register the configured cron jobs and run the scheduler. Its length is nine three-line registrations, most under a comment, and a function per registration would only scatter the list.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0, and by 0.28.0 with the default rules. Function-simplification reviews fail the check by default, so this one fails vaultwarden's check. With every rule, 0.28.0 asks it beside the other rules' questions about its functions, and that answer does not report it; no change aimed at it.

<!-- example: cookiecutter-django 8c12544ae3a1e980f96efd1bf0843af32641166efa422c7e33d838c675832ea4 -->
### cookiecutter-django: `update_package_version`

- **Where:** [`scripts/python_dependency_version.py:51`](https://github.com/cookiecutter/cookiecutter-django/blob/1ec1d82fa145375f407b01ccc44ba0a6db7d5ff2/scripts/python_dependency_version.py#L51) in cookiecutter/cookiecutter-django at `1ec1d82`.
- **Finding (consider):** `update_package_version` likely mixes separate jobs; splitting it may make it easier to understand.
- **Why it was wrong:** It is 19 lines with one job, bumping one package's version everywhere: the pin in `pyproject.toml`, then the `rev:` in two pre-commit configurations, each step already under a comment. Two helpers would only turn those comments into function names.
- **Since:** a note since 0.23.0, which made splitting a function of 20 lines or fewer at most a note ([changelog](../../changelog.md#0230---2026-09-27)).

<!-- example: nodegoat 7d7158b633a1c1c82e4d565481936fba407469d9e2a0d05bde6f49eb63a60676 -->
### NodeGoat: `AllocationsDAO`

- **Where:** [`app/data/allocations-dao.js:4`](https://github.com/OWASP/NodeGoat/blob/c5cb68a7084e4ae7dcc60e6a98768720a81841e8/app/data/allocations-dao.js#L4) in OWASP/NodeGoat at `c5cb68a`.
- **Finding (consider):** `AllocationsDAO` likely mixes separate jobs; splitting it may make it easier to understand.
- **Why it was wrong:** `AllocationsDAO` is a constructor function whose body defines its two methods, `this.update` and `this.getByUserIdAndThreshold`. Those are the separate jobs, and they already have names.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.
