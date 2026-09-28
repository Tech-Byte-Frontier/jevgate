# Shared logic

{{#include ../../reference/_rules.md:maintainability-shared-logic}}

## When a finding is right

A finding says two or more places perform the same steps for the same purpose, so one shared implementation would serve them. It is right when the copies would change together: the same validation in two handlers, or a parser written twice, where a fix to one belongs in the other. It is wrong when the copies are setup that a framework or a test needs in each place, when their differences are the point, or when the shared lines are too few to be worth a function. Of the 443 findings labeled wrong or debatable so far outside Bend 2 code, 89 were spans too small to share and 48 were steps each test case writes out.

Short copies between test cases are notes, copies in test fixtures and helpers are at most a consider, and copies in code marked deprecated are not compared.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: vaultwarden dabef680a4d1e474557feac4ad9b01ab08bd58ccc930dcc53ceaa52aa564f591 -->
### vaultwarden: two error serializers

- **Where:** [`src/error.rs:253`](https://github.com/dani-garcia/vaultwarden/blob/061694d0cb3bbf5d4c7e920c892824f0020cff83/src/error.rs#L253) in dani-garcia/vaultwarden at `061694d`.
- **Finding (review):** `ApiErrorResponse::serialize` and `CompactApiErrorResponse::serialize` perform the same steps for the same purpose.
- **Why it was wrong:** The two hand-written `Serialize` implementations lay out different wire formats, one with nine fields and one with six. They share three null exception fields, the `object` tag and `state.end()`; a helper for those calls would split each format across two places.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.

<!-- example: shiori 16ef905810be62d7f59c8ed0ffa2ed6c85b526798c90e9556a40b8210ecda9e1 -->
### shiori: test setup around a fixture

- **Where:** [`internal/domains/auth_test.go:17`](https://github.com/go-shiori/shiori/blob/9a9a426acaca0e57e205bf20266a44954aaa8264/internal/domains/auth_test.go#L17) in go-shiori/shiori at `9a9a426`.
- **Finding (consider):** `TestAuthDomainCheckToken`, `TestAuthDomainCheckTokenInvalidMethod` and 2 more copies repeat the same steps across test cases.
- **Why it was wrong:** The shared lines are four lines of setup: a context, a logger, the project's fixture `testutil.GetTestConfigurationAndDependencies` and one constructor. The fixture already is the shared helper; wrapping it again would save three lines per test.
- **Since:** a note since 0.23.0, which made copies of up to twelve lines between test cases in different files notes, like short copies inside test cases ([changelog](../../changelog.md#0230---2026-09-27)).

<!-- example: microblog d74d3030486933b9af6521b6fe291d7e8815eab1d9576ac9e4607bafcc6fd092 -->
### microblog: blueprint registrations

- **Where:** [`app/__init__.py:46`](https://github.com/miguelgrinberg/microblog/blob/a975ef64864354867c88e0ed3a17ba7d17dca752/app/__init__.py#L46) in miguelgrinberg/microblog at `a975ef6`.
- **Finding (consider):** Lines 46 and 55 of `create_app` repeat related steps; a person should decide whether they belong together.
- **Why it was wrong:** The repeated lines are Flask's two-line blueprint registration, a local import and `app.register_blueprint`, written out for each of five blueprints with its own module and prefix. A loop over module and prefix pairs would hide the wiring and save nothing.
- **Since:** a note from 0.28.0. A shared-logic consider now needs its same-steps answer at 0.90 (it was 0.87 here): below it, 25 of 54 such considers were right on the projects used for tuning, and 9 of 29 on the unseen ones.
