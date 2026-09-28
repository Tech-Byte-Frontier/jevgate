# Access control

{{#include ../../reference/_rules.md:security-access-control}}

## When a finding is right

A finding says a SQL policy, SECURITY DEFINER function or grant lets users reach other users' rows, or that a SpacetimeDB table, view or reducer exposes or changes other users' data without checking the caller. It is right for a policy that trusts `user_metadata`, a SECURITY DEFINER function every role may call that deletes any stored file, or a grant that opens writes to every user. It is wrong when the rows are ones their owners chose to share, or when the function answers only what every user may read already. Of the 18 findings labeled wrong or debatable so far, 12 were policies showing rows their owners marked shared.

The rule has no labels on projects JevGate was never tuned on, so its levels cannot be measured yet.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: basejump c4106f81bfcee8dca28697faf8a1e409d6a134fe6b0011b3e6f4fefe3bfd0c37 -->
### Basejump: `accept_invitation`

- **Where:** [`supabase/migrations/20240414162100_basejump-invitations.sql:158`](https://github.com/usebasejump/basejump/blob/7a1f95ccef74eb2e638d5e4233b66b6cbbe175e6/supabase/migrations/20240414162100_basejump-invitations.sql#L158) in usebasejump/basejump at `7a1f95c`.
- **Finding (review):** SECURITY DEFINER function `accept_invitation` reads or changes other users' rows without checking the caller.
- **Why it was wrong:** The invitation token is the check: 30 random bytes, matched exactly and valid for a day. The function adds only the caller (`auth.uid()`) to the account, and only signed-in users may execute it.
- **Since:** cleared in 0.20.0: a SECURITY DEFINER function that acts only for whoever holds a secret token it looks up by value does not skip the caller check ([changelog](../../changelog.md#0200---2026-09-26)).

<!-- example: chatbot-ui 2cc59675deb0d24c686b0ef48f18a6abbd4554c70047bb2a9426ec5b958a0528 -->
### Chatbot UI: shared files

- **Where:** [`supabase/migrations/20240108234544_add_files.sql:44`](https://github.com/mckaywrigley/chatbot-ui/blob/81328b61d2a4ab597a7a057be70e785cf756d9f8/supabase/migrations/20240108234544_add_files.sql#L44) in mckaywrigley/chatbot-ui at `81328b6`.
- **Finding (consider):** Policy `allow view access to non-private files` on `files` likely lets every user it applies to read or change other users' rows.
- **Why it was wrong:** Others can read a file only once its owner changes `sharing` from the default `'private'`, and only the owner can, through the policy on their own files. This is the read side of sharing; tying the condition to the user's id would remove the feature.
- **Since:** 0.21.0 accepts a policy that lets others read rows their owners marked shared, and the same policy on `chats` is no longer reported. This one is still a consider, now for trusting a value users can change, which is wrong too: only the owner can set `sharing`. Not addressed.

<!-- example: chatbot-ui 12a2c9ccaa329d86d8e388d5d105704f206711f233572065a3a7c1240a028a4c -->
### Chatbot UI: `non_private_file_exists`

- **Where:** [`supabase/migrations/20240108234544_add_files.sql:92`](https://github.com/mckaywrigley/chatbot-ui/blob/81328b61d2a4ab597a7a057be70e785cf756d9f8/supabase/migrations/20240108234544_add_files.sql#L92) in mckaywrigley/chatbot-ui at `81328b6`.
- **Finding (review):** SECURITY DEFINER function `non_private_file_exists` reads or changes other users' rows without checking the caller.
- **Why it was wrong:** The function returns only whether a file with that id exists with `sharing <> 'private'`: exactly the rows the shared-files policy already shows every role. Private and missing files both give false. Its open `search_path` would be a fair consider; a missing caller check is not.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.
