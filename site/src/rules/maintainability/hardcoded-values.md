# Hardcoded values

{{#include ../../reference/_rules.md:maintainability-hardcoded-values}}

## When a finding is right

A finding says a value written in the code changes between deployments, needs a descriptive name, or special-cases one identity. It is right for a production host, a customer id or a price written where configuration belongs, or for a number whose meaning a reader must guess. It is wrong when the code around the value already says what it is: the argument it fills, the function or variable it is assigned to, or a comment beside it. A third of the findings labeled wrong or debatable outside Bend 2 code were values whose meaning was clear from their context.

The rule is opt-in since 0.26: on projects JevGate was never tuned on, 6 of its 37 labeled findings were right. A value that only needs a name is at most a consider, and a note when its file writes it once. A value said to change between deployments is asked where it would differ, and one that is the same wherever the program runs is a note.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: nanogpt c187a670fde224af824f3b2eec203007dc6a55baf65fce14c806c425461e823e -->
### nanoGPT: `312e12`

- **Where:** [`model.py:289`](https://github.com/karpathy/nanoGPT/blob/3adf61e154c3fe3fca428ad6bc3818b27a3b8291/model.py#L289) in karpathy/nanoGPT at `3adf61e`.
- **Finding (consider):** `GPT::estimate_mfu` likely uses a value whose meaning a reader must guess. The value is 312e12.
- **Why it was wrong:** The value is assigned to `flops_promised` beside the comment "A100 GPU bfloat16 peak flops is 312 TFLOPS", and the docstring defines the result in those units. Nothing is left to guess.
- **Since:** a note since 0.21.0, which made a value that only needs a name a note when its file writes it once ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: t3-turbo 1168b49036823f11ac69126077ed11216a53daf074e193c7b08b29a691fd86fb -->
### create-t3-turbo: the auth CLI configuration

- **Where:** [`packages/auth/script/auth-cli.ts:21`](https://github.com/t3-oss/create-t3-turbo/blob/8f945b7bb3bfb3ca8358d48b1ff0214079bc11ee/packages/auth/script/auth-cli.ts#L21) in t3-oss/create-t3-turbo at `8f945b7`.
- **Finding (review):** One of this file's constants fixes a value that differs between deployments.
- **Why it was wrong:** The file says it is used only by the Better Auth CLI to generate the database schema and is "NOT intended for runtime use". Its `http://localhost:3000`, `"secret"` and `"1234567890"` are placeholders that never reach a deployment.
- **Since:** a note since 0.21.0, which asks such a finding where its value would differ; code no deployment runs makes it a note ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: dvga 25050fb06dde46631c19b795a8c8e1212b141e19256de7bc8a540bd0f58a562e -->
### Damn Vulnerable GraphQL Application: `'DVGAUser'`

- **Where:** [`core/views.py:118`](https://github.com/dolevf/Damn-Vulnerable-GraphQL-Application/blob/a961308c02d1fb462b192681c336b0739e432da7/core/views.py#L118) in dolevf/Damn-Vulnerable-GraphQL-Application at `a961308`.
- **Finding (review):** `CreatePaste::mutate` fixes a value that differs between deployments. The value is 'DVGAUser'.
- **Why it was wrong:** `'DVGAUser'` is the name of the owner row that `setup.py` creates on every install, and every paste is attributed to it. It is the same in every deployment; reading it from configuration would change nothing.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.
