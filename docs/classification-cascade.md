# Possible classification cascade

Status: first experiment available with `--classification-cascade --format json`.
One shared-evidence request includes overlapping test/production signals and a
conditional test specialist for up to 12 repeated fragments (at most 37 extra
questions). JSON retains raw answers, locations, model, composition version,
selected branches, omissions and conflicts under the shared-logic assessment's
`cascade` field. The shadow comparison leaves baseline findings unchanged;
uncertain routes retain the general assessment and conflicting applicable
judgments remain unresolved. Default behavior and review thresholds are unchanged.
Broader purpose/operation specialization and sequential requests remain proposals.
Improvement must be measured: an incorrect early route can hide a useful finding.

`--roles-only` evaluates the foundation independently: test scenarios, scenario
support, framework/tool implementations and application/library implementations
are separate overlapping Noul questions. Evidence sufficiency is independent of
role probability. Regions come from parser locations; repeated occurrences in the
same region share answers, and each occurrence pair retains all supported
test/test, test/implementation or implementation/implementation relationships.
Unknown or omitted regions leave relationships unresolved. The 32-region bound
and parser limits are reported explicitly. These roles remain disconnected from
specialist routing until human-reviewed role evaluation and a downstream
comparison establish benefit. Specialist prompts and maintainability thresholds
remain fixed during foundation evaluation.

## From basic evidence to specialized judgments

Use deterministic facts first, then semantic judgments where purpose matters.
The proposed stages are logical dependencies, not necessarily separate API calls.

| Layer | Evidence or question | How it helps |
| --- | --- | --- |
| Eligibility | Is this selected file supported source, generated output, data, or an unsupported format? | Use configuration, extensions and parser support to establish scope. Unsupported or skipped does not mean clear. |
| Language | Python, Rust, JavaScript, TypeScript, another language, or mixed/unknown? | Select parsers and language context in code. Avoid paying Jev to rediscover an unambiguous extension. Preserve parser failures and embedded-language limits. |
| Purpose | Does this region implement tests, UI behavior, service behavior, shared library logic, or tooling? | Use source semantics to choose relevant maintainability questions. Paths and imports provide evidence, not a maintainability verdict. |
| Operation role | Setup, action under test, assertion, rendering, transformation, resource lifecycle, or policy decision? | Distinguish the responsibilities inside a file before judging complexity or repetition. |
| Relationship | Are these locations implementing common mechanics, invoking an existing helper, repeating an intentional action, or expressing independent policies? | Focus shared-logic judgments on what should be maintained together. |
| Maintainability | Would a specific split, simplification, or extraction reduce maintenance while preserving the relevant responsibilities? | Produce localized findings, acceptable outcomes, uncertainty, or missing-context outcomes. |

JevGate already discovers source files by extension/configuration, assigns file
roles from paths/configuration, and chooses supported parsers by extension. The
proposed semantic layers would extend that evidence, rather than replace discovery.

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
