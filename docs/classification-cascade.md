# Classification cascade

Every shared-logic check includes this in the same request.
One shared-evidence request includes four overlapping semantic roles for up to 32
deduplicated regions containing repeated occurrences (160 role/evidence questions)
and a conditional test specialist
for up to 12 repeated fragments. If that body would exceed the provider context
limit, trailing occurrence regions are omitted and counted in `regions_omitted`;
the general maintainability questions stay in the same request.
An uncertain file-organization, function-simplification, or shared-logic result
triggers one follow-up whose state is only the undecided operation, the
repeated lines, or the file without the other evidence. The same question is
asked again. A follow-up replaces the uncertain status only when review or
clear reaches the existing 0.80 threshold. JSON retains raw answers, locations, model,
composition version, occurrence relationships, omissions and conflicts. Region
roles are under `files[].role_assessment`; routing and specialist comparisons are
under the shared-logic assessment's `cascade` field.

`--roles-only` evaluates the foundation independently: test scenarios, scenario
support, framework/tool implementations and application/library implementations.
Evidence sufficiency is independent of role probability. A parameterized template
that owns the tested operation and its expected-result checks defines scenarios;
generic runners and assertion APIs accept arbitrary supplied tests or values.
Parsers supply locations, not semantic verdicts. Repeated occurrences in the same
region share answers. Each occurrence pair retains every supported test/test,
test/implementation or implementation/implementation relationship.

Only fully resolved test–test groups select the specialist. Mixed groups,
unsupported evidence, omitted occurrences and uncertain roles retain the general
assessment. Conflicts in an applicable test branch remain unresolved. The
specialist comparison does not replace the general verdict. Specialist criteria
use the same structured review boundary as the other judgments, and the
maintainability thresholds are unchanged. File eligibility, including which
test code reaches those gates, is applied on every check. Broader specialization
remains a proposal.

## From basic evidence to specialized judgments

Use deterministic facts first, then semantic judgments where purpose matters.
The proposed stages are logical dependencies, not necessarily separate API calls.

| Layer | Evidence or question | How it helps |
| --- | --- | --- |
| Eligibility | Is this selected file supported source, generated output, data, or an unsupported format? | Shell scripts and `.d.ts` declarations are skipped before any model call. Skipped is not clear. |
| Language | Python, Rust, JavaScript, TypeScript, another language, or mixed/unknown? | Select parsers and language context in code. Avoid paying Jev to rediscover an unambiguous extension. Preserve parser failures and embedded-language limits. |
| Purpose | Does this region implement tests, UI behavior, service behavior, shared library logic, or tooling? | Use source semantics to choose relevant maintainability questions. Paths and imports provide evidence, not a maintainability verdict. |
| Operation role | Setup, action under test, assertion, rendering, transformation, resource lifecycle, or policy decision? | Distinguish the responsibilities inside a file before judging complexity or repetition. |
| Relationship | Are these locations implementing common mechanics, invoking an existing helper, repeating an intentional action, or expressing independent policies? | Focus shared-logic judgments on what should be maintained together. |
| Maintainability | Would a specific split, simplification, or extraction reduce maintenance while preserving the relevant responsibilities? | Produce localized findings, acceptable outcomes, uncertainty, or missing-context outcomes. |

JevGate discovers source files by extension and configuration, then applies
eligibility before the gates. Shell scripts and TypeScript declaration files are
outside the three gates. Structural test markers — Rust `#[cfg(test)]`, `#[test]`,
and the `not(test)` exception, plus JavaScript `describe` / `it` — are removed
from the source the gates read, while line numbers stay aligned. The gate request
includes that base classification.

A path that already means tests is not judged unless `--include-tests` is set.
When that file still contains other code, one purpose question classifies it as
tests, application, or mixed before any gate request. Portion questions are used
only when the file is confidently mixed. An uncertain purpose does not drop the
file: the gates still judge it, and only confident test regions are removed.
A confident test result skips the gates unless `--include-tests` is set. Mixed
files always judge the application portion; the flag also judges the test portion
as its own request.

Language, application area and test role are separate axes. A Python file can
contain backend tests; a Rust module can contain production functions and tests;
a UI component can contain shared transformation logic. Classify relevant regions
as well as files, and permit overlapping roles. For overlapping yes/no signals,
consider separate [Noul questions](https://docs.typesafe.ai/primitives/noul);
use Choice for genuinely competing alternatives, with unknown/context outcomes.

## Where specialization could help

| Context | Focused judgment | Boundary to preserve |
| --- | --- | --- |
| Tests | Is repetition reusable fixture setup, or a sequence needed to demonstrate a behavior? | Repeated assertions or retry actions are not automatically extraction opportunities; duplicated setup can still be worth sharing. |
| Frontend | Are rendering, state transitions and data transformations separate responsibilities? | Similar markup or lifecycle calls alone do not establish shared implementation. |
| Backend | Do handlers repeat validation, mapping or cleanup mechanics? | Similar expressions with different permission rules or transaction boundaries may need to remain independent. |
| Shared libraries/tooling | Do operations implement one common transformation or resource protocol? | Language idioms and calls into existing abstractions are not duplicated implementations. |

These specialize file organization, function simplification and shared logic.
They do not introduce security/correctness enforcement or exempt an entire role
from review. Language-specific evidence can clarify ownership, decorators or
asynchronous boundaries without turning syntax into a verdict.

## Request strategy

Start with one shared-evidence request per selected file. Ask a small set of role
questions alongside conditional specialist questions, with each premise stated
explicitly. Code selects applicable answers afterward. TypeSafe calls this
[speculative fan-out](https://docs.typesafe.ai/patterns/fan-out). Questions in the
same request cannot read each other's answers: a specialist must judge its stated
premise against the source, not an unavailable routing result.

A true sequential cascade becomes useful when an answer determines which explicit
context to retrieve, which candidates to construct, or a question set too large to
send speculatively. Compare it against the single-request baseline before adoption.
Extra stages repeat evidence and add latency; fan-out still adds question tokens.
Bound questions, candidates, context and follow-up requests, and report omissions.
Any added context must remain within the user's authorized scope.

For a larger taxonomy, [hierarchical classification](https://docs.typesafe.ai/cookbooks/hierarchical_classification)
can retain several plausible branches instead of taking one irreversible early
choice. Start with a shallow routing graph; a deep language-by-framework-by-role
tree adds complexity before its value is demonstrated.

## Uncertainty and observability

- Keep raw routing and specialist probabilities, evidence locations, model and
  rubric versions, selected branches and composition reasons.
- An ambiguous route must retain plausible branches or use the general assessment;
  it must not silently suppress a finding or report clear. Distinguish ambiguity
  from missing evidence, unsupported syntax and provider failure.
- Conflicting applicable branches remain visible and unresolved. Unused branches
  do not affect the final result. Test classification alone cannot establish that
  repetition is intentional.
- Do not multiply independent answer probabilities and present the product as a
  calibrated final probability. Keep routing certainty separate from concern
  probability; [Choice confidence](https://docs.typesafe.ai/confidence) describes
  distribution concentration, not end-to-end correctness.
- Version routing, specialist questions and composition; invalidate affected cache
  entries when evidence or semantics change. Keep general checks available when
  routing fails, and preserve the existing review thresholds during comparisons.

## First experiment and adoption criteria

Start with test-role and repeated-fragment-purpose judgments. Compare the existing
shared-logic classifier with one-request specialization on the same small frozen
set. Only then compare an additional request where a concrete failure justifies it.

Include useful refactors, legitimate repetition, mixed test/production files,
unusual paths, overlapping UI/service roles and missing-context cases. Keep labels
outside model inputs, preserve first live CLI results, and reserve fresh cases for
independent evaluation. Replay and unit tests validate composition, not accuracy.

Measure routing errors, missed useful findings, false positives, uncertainty,
coverage and calibration by role, alongside attempts, input/output tokens,
estimated batch cost and end-to-end latency. Adopt a layer only if it improves the
agreed quality/cost tradeoff without hiding difficult cases. Record detailed
evaluation evidence privately; keep this document focused on the design.
