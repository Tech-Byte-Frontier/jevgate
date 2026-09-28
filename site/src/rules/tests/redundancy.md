# Test redundancy

{{#include ../../reference/_rules.md:tests-redundancy}}

## When a finding is right

A finding says two or more tests check the same behavior, with equivalent inputs (one of them adds nothing) or with different ones (one parameterized test could hold them). It is right when the tests run the same code with inputs that make no difference to it. It is wrong when each test checks something the other does not: another code path, such as the synchronous and asynchronous clients, a different public function, or another variant of a protocol. Of the 31 findings labeled wrong or debatable so far outside Bend 2 code, 8 covered distinct behavior and 7 different public functions.

A pair that would be a review is first asked whether each test checks something the other does not. A pair on its own is at most a note; three or more tests linked by such pairs are one consider.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: httpx eb7a4f7146e607bba5d799dd31ff9a05ab56ba274a9a1ec287248849b39e1a2f -->
### httpx: synchronous and asynchronous digest tests

- **Where:** [`tests/client/test_auth.py:596`](https://github.com/encode/httpx/blob/b5addb64f0161ff6bfe94c124ef76f6a1fba5254/tests/client/test_auth.py#L596) in encode/httpx at `b5addb6`.
- **Finding (review):** `test_async_digest_auth_raises_protocol_error_on_malformed_header` and `test_sync_digest_auth_raises_protocol_error_on_malformed_header` check the same behavior with equivalent inputs; one adds nothing.
- **Why it was wrong:** One test drives digest authentication through `httpx.AsyncClient` and the other through `httpx.Client`: different code paths. JevGate resolved both calls to the same function, which hid the difference.
- **Since:** a note since 0.20.0, which asks a pair that would be a review whether each test checks something the other does not ([changelog](../../changelog.md#0200---2026-09-26)); a pair on its own is at most a note.

<!-- example: httpx 5b83407a5252ba831d8afffb9f60ea774dea4c59609b1c4503054646384a83ad -->
### httpx: digest variants

- **Where:** [`tests/test_auth.py:44`](https://github.com/encode/httpx/blob/b5addb64f0161ff6bfe94c124ef76f6a1fba5254/tests/test_auth.py#L44) in encode/httpx at `b5addb6`.
- **Finding (consider):** 3 tests of `send` overlap: `test_digest_auth_rfc_2069`, `test_digest_auth_rfc_7616_md5`, `test_digest_auth_with_401`.
- **Why it was wrong:** The first two check different digest variants against their RFC test vectors, and the third the basic flow. JevGate grouped them under `send` because it read the generator's `flow.send(response)` as a call to `Client.send`.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.

<!-- example: linkace 8bbbc843f68d35e9c1d40554beebc119dca6e205287a3655ce60d694ff88dba9 -->
### LinkAce: search schemas

- **Where:** [`tests/Search/SearchableArrayTest.php:16`](https://github.com/Kovah/LinkAce/blob/d6821661fb5878850738dc5f3d3593799d89445f/tests/Search/SearchableArrayTest.php#L16) in Kovah/LinkAce at `d682166`.
- **Finding (consider):** 3 tests of `create` overlap: `test_link_searchable_metadata_and_array`, `test_list_searchable_metadata_and_array`, `test_tag_searchable_metadata_and_array`.
- **Why it was wrong:** Each test checks a different model's search schema: links with their tags, lists and counts, tags with their name, lists with their name and description. The expected arrays differ in shape, so one parameterized test would be harder to read.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.
