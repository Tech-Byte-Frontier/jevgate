# Unsafe settings

{{#include ../../reference/_rules.md:security-unsafe-settings}}

## When a finding is right

A finding says code turns off a security check or chooses a weak setting. It is right when a real connection accepts any certificate, passwords are kept with a fast hash, a session cookie is sent without its flags, or a secret is built into browser code. It is wrong when the weak setting is an option a caller or operator must ask for, when the value is no secret, or when the check it names protects nothing there, such as a CSRF exemption on a view that changes no data. The commonest causes of findings labeled wrong or debatable were CSRF exemptions on views that change nothing and plain connections inside a cluster by design.

A password or token finding stays a review only when the function itself hashes with a fast hash or turns verification off; otherwise, since a callee or the platform may do it, it is a consider.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: httpx 3d0df78be51ec97d3b9a41f85bacb39eb93afadc0592d2c83b94218bab0b2f1b -->
### httpx: `create_ssl_context`

- **Where:** [`httpx/_config.py:43`](https://github.com/encode/httpx/blob/b5addb64f0161ff6bfe94c124ef76f6a1fba5254/httpx/_config.py#L43) in encode/httpx at `b5addb6`.
- **Finding (review):** `create_ssl_context` turns off certificate or signature verification.
- **Why it was wrong:** The branch that skips verification runs only when the caller passes `verify=False`; the default builds a verifying context. An HTTP client library offering a documented, explicit opt-out is doing its job.
- **Since:** a note since 0.21.0, whose TLS question counts verification skipped only when a caller or the operator asks for it as not turning it off ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: wp-two-factor 778ec8f5c23edb69c27314ef08f1387c52cc602e7a4abd9f1811dbc98969e57b -->
### Two-Factor: `get_code`

- **Where:** [`providers/class-two-factor-provider.php:161`](https://github.com/WordPress/two-factor/blob/72effa59d85970ccadd2b8fc420dab471e299cd5/providers/class-two-factor-provider.php#L161) in WordPress/two-factor at `72effa5`.
- **Finding (review):** `Two_Factor_Provider::get_code` makes secret tokens or identifiers that can be guessed, with a non-cryptographic random generator or from known data.
- **Why it was wrong:** `get_code` picks each character with `wp_rand()`, which calls PHP's cryptographic `random_int()` on every PHP version the plugin supports.
- **Since:** cleared in 0.21.0, which knows WordPress's `wp_rand` is cryptographic ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: pygoat ec189b71be34be1941d71d36d8ed249ddeca3372093cff70fce60c5a413f9810 -->
### PyGoat: `A7_disscussion_api`

- **Where:** [`introduction/apis.py:93`](https://github.com/adeyosemanputra/pygoat/blob/19d17cc8874861142b330636d068bbde54e86b85/introduction/apis.py#L93) in adeyosemanputra/pygoat at `19d17cc`.
- **Finding (review):** `A7_disscussion_api` turns off cross-site request forgery protection for requests that change data.
- **Why it was wrong:** The view only checks whether the posted code contains a snippet and answers success or failure. It changes no data, so a forged request can do nothing through its CSRF exemption.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.
