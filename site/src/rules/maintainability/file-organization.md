# File organization

{{#include ../../reference/_rules.md:maintainability-file-organization}}

## When a finding is right

A finding says a file holds parts that would be easier to find in modules of their own, and it names the group of members to move. It is right when the named group is a feature or a job a reader would look for apart from the rest: a URL scraper inside a model, or a diff engine inside a renderer. It is wrong when the file is small and holds one subject, when the named group cuts a feature in half, or when its members cannot move, such as a class's own methods. About a fifth of the findings labeled wrong or debatable outside Bend 2 code named a group that was no coherent part, and small files about one subject were the next commonest cause.

A file of fewer than 250 lines gets at most a note, a group holding three quarters or more of a file's members is not named, and a test file's split is at most a consider.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: vaultwarden da084b5f559d93e1ca7b566c5c5ac2402f2f03f153d6a8a555da2aecb8d9db6e -->
### vaultwarden: the mailer

- **Where:** [`src/mail.rs:25`](https://github.com/dani-garcia/vaultwarden/blob/061694d0cb3bbf5d4c7e920c892824f0020cff83/src/mail.rs#L25) in dani-garcia/vaultwarden at `061694d`.
- **Finding (consider):** This file writes out the same kind of code for several features; each feature's part would be easier to find in its own module. It named two groups: the mail transports with most of the `send_*` functions, and the template helpers with `send_password_hint`.
- **Why it was wrong:** `mail.rs` is the mailer: one short `send_*` function per email template, beside the transport and rendering helpers. Neither group is a feature, and splitting the one-function-per-template list across modules would scatter it.
- **Since:** cleared in 0.21.0: a file that writes out the same kind of code for each of several features is one job ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: httpx edc801c2a9263709c0abe04bef51bb00b8199447ed73ca24d1f1bf130e2b6670 -->
### httpx: `_utils.py`

- **Where:** [`httpx/_utils.py:162`](https://github.com/encode/httpx/blob/b5addb64f0161ff6bfe94c124ef76f6a1fba5254/httpx/_utils.py#L162) in encode/httpx at `b5addb6`.
- **Finding (consider):** Some members of this file could move to a separate module: `URLPattern` or the text helpers `to_bytes`, `to_str` and `unquote`.
- **Why it was wrong:** `_utils.py` is a 242-line module of small helpers. Moving `URLPattern` or one-line helpers into modules of their own would scatter a file that is already easy to navigate.
- **Since:** a note since 0.20.0, which gives a file of fewer than 250 lines at most a note ([changelog](../../changelog.md#0200---2026-09-26)).

<!-- example: microblog 9e7ea7bfbd3108bddeb4f620a2c21db26b3e79962335530e15f9f186dcd76512 -->
### microblog: `models.py`

- **Where:** [`app/models.py:131`](https://github.com/miguelgrinberg/microblog/blob/a975ef64864354867c88e0ed3a17ba7d17dca752/app/models.py#L131) in miguelgrinberg/microblog at `a975ef6`.
- **Finding (review):** This file holds several features that would be easier to find apart. It named two dozen `User` methods, or `SearchableMixin`, as the group to move.
- **Why it was wrong:** `models.py` is the application's one SQLAlchemy models module, 356 lines, whose classes refer to one another. The `User` methods cannot leave their class; moving `SearchableMixin` next to the search helpers is optional tidying at this size, not a review.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.
