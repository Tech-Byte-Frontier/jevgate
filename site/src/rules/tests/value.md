# Test value

{{#include ../../reference/_rules.md:tests-value}}

## When a finding is right

A finding says a test checks only its mocks, computes its expected value with the logic it tests, asserts internal details instead of observable results, or mixes unrelated behaviors. It is right when the test would pass whatever the code did: it asserts the value its mock returns, or builds its expected value by calling the code under test. It is wrong when what the test reads is behavior a caller can observe: a panel's recorded output, a framework's documented hook, or the state the program acts on next. Of the 81 findings labeled wrong or debatable so far outside Bend 2 code, 37 read state that is the observable behavior and 15 checked a callback that is part of the public interface.

A test said to assert internal details is asked, with the bodies of the functions it calls, what its assertions read: results, state the program shows or acts on next, or effects a caller observes clear it.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: debug-toolbar 0117af9b6ddfc1409cfd6755add7f04b024aba5e135a92faa64f4ef25de33a73 -->
### Django Debug Toolbar: `test_recording`

- **Where:** [`tests/panels/test_sql.py:91`](https://github.com/django-commons/django-debug-toolbar/blob/dfc69d9b8f15e36c776ec54f50c7b4e2e6082cbb/tests/panels/test_sql.py#L91) in django-commons/django-debug-toolbar at `dfc69d9`.
- **Finding (consider):** `test_recording` asserts internal details instead of observable results.
- **Why it was wrong:** The test checks that a query is recorded once with its alias, SQL, duration and stack trace in `panel._queries`. That list is what the panel saves as its statistics and renders, so the assertions read the panel's recorded output.
- **Since:** cleared in 0.21.0, which asks such a test what its assertions read, with the bodies of the functions it calls ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: wp-two-factor e411c879204fcde7194306044a4eb6549ba043157db5bfa45e69658ed39c513a -->
### Two-Factor: `test_get_user_time_delay`

- **Where:** [`tests/class-two-factor-core.php:1192`](https://github.com/WordPress/two-factor/blob/72effa59d85970ccadd2b8fc420dab471e299cd5/tests/class-two-factor-core.php#L1192) in WordPress/two-factor at `72effa5`.
- **Finding (review):** `test_get_user_time_delay` computes its expected value with the logic it tests.
- **Why it was wrong:** The expected values are the one-second default, the 15-minute cap and `pow( 2, 5 ) * $rate_limit`, the documented doubling after five failed attempts written out. Nothing calls the code under test to compute them, so a changed base, default or cap would fail the test.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.

<!-- example: linkace 1a891547c401db573bc0573c612b88865652d4cda0b34afbd0ba1b7984a711b0 -->
### LinkAce: `test_successful_check`

- **Where:** [`tests/Helper/UpdateCheckTest.php:19`](https://github.com/Kovah/LinkAce/blob/d6821661fb5878850738dc5f3d3593799d89445f/tests/Helper/UpdateCheckTest.php#L19) in Kovah/LinkAce at `d682166`.
- **Finding (review):** `test_successful_check` only checks values its mocks were set to return.
- **Why it was wrong:** `checkForUpdates` returns the fetched version only when it is newer than the installed one, and `true` otherwise. The test fakes `v100.0.0` to take the first branch, and its sibling fakes `v0.0.0` and expects `true`: together they check the comparison, not the mock.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.
