# Evidence units

JevGate asks Jev short, literal questions about small units of evidence that code
has already built, then composes the answers in code.

## Why units

A single request per file carried the whole source, every rule's candidates and
several broad questions. Large states lowered decisiveness, and reconciling
file-wide answers with per-operation probes let decisive concerns disappear.
Units send only what one question needs: one function's source, a file's member
signatures, or one candidate pair.

## Pipeline

1. **Eligibility and purpose.** Deterministic roles, generated-code headers and
   structural test markers decide which code the application rules and the test
   rules see. Only a test path without structural tests gets a file-purpose request.
2. **Local analysis** (`src/analysis/`). Units with signatures, calls, references
   and control-flow nesting; callbacks registered through calls; member groups by
   average linkage; callers that import the file; Type-2 clone candidates grouped
   by overlapping copies; test cases with their subjects and similar pairs.
3. **First pass** (`src/units/`). One dispatch of every unit request. Functions
   are packed eight per request; tests are sent one per request, because unrelated
   tests in the same state left more answers undecided. State uses literal paths
   such as `functions[2].source`; group IDs are Choice options. Stage and freshness
   hashes stay in local `jevgate` metadata that is not uploaded.
4. **Follow-ups.** One recheck per uncertain unit, with callee signatures, the
   enclosing functions or the file's application source; a decisive recheck
   replaces the first answer and both are kept. A hardcoded-value unit is asked
   instead whether every value is of an acceptable kind; that check can only
   clear, since re-asking the concern per value added false findings.
   Then one locate Choice per split finding picks the body block to extract.
   Security units whose presence answers are not clear get one trace before the
   rechecks: specific literal checks per kind, a Choice among the unit's sites,
   and the origin of its values (or whether it runs only in development). One
   broad "is every value bound, escaped or checked?" stayed undecided even for
   `eval` of model output; a check per kind decides and names the kind. An
   injection whose origin stays unclear or is the function's parameters is
   asked its origin and checks again with up to three callers; a decisive
   answer replaces the traced one.
   Agent instruction files (`src/docs/`) are found by name even when hidden
   or ignored. Each file's heading sections, or the top-level blocks of a long
   section, are sent packed beside the nearest manifests, the configured
   linters and the directories. Code decides which harness loads each file and
   when, from its documented discovery rules. It also records loading facts:
   copies, unresolved imports, and files a harness skips or truncates. These
   are reported, never judged. Project Markdown of 300 or more lines is sent
   as its headings only, with `#` marks for nesting. A split finding is then
   located with one Choice among its top-level parts. Per-section questions on
   project docs were dropped: on a labeled sample they found almost nothing,
   and they cost about five times more than an outline.
5. **Composition** (`src/units/compose.rs`). Pure. On a Score whose top level is
   the actionable concern: review at 0.80 on the top level, consider at 0.80 on
   middle-or-top, clear when the top level is ruled out at 0.80, otherwise
   uncertain. Where the middle level says the code is fine as it is, a consider
   also needs the top level at 0.50; middle mass alone is an optional note. For hardcoded values and security, an answer still
   undecided after its follow-up is a note when it leans toward the concern
   (0.50, the leading probability) and stays uncertain otherwise: undecided
   answers leaning away were almost all acceptable code, and leaning toward
   held both real positives of the labeled set. Instruction sections are
   cleanups, so their findings are at most a consider. Undecided answers do not
   lean into notes, because on the labeled set that added notes to kept sections.
   A large document's undecided history answer does lean into a note. On its
   labeled set, no living document leaned past 0.50. Questions ask whether a change would help a reader ("would splitting
   it make it easier to understand?"), not how many tasks or purposes there are:
   Jev does not count reliably and reads "tasks" literally. Copies inside test
   cases are one level lower. A review always carries a finding.
6. **Gate.** `--fail-on` and the baseline act on composed findings only.

## Constraints

- Syntax supplies units, candidates, locations and eligibility (size, nesting),
  never verdicts; a unit below an eligibility floor is too small, never clear.
- Keep questions atomic and literal; no thresholds, hashes or self-descriptions in
  uploaded state or questions.
- Preserve raw answers, uncertainty and needs-context outcomes.
- Version question wording (`units::questions::VERSION`) and composition
  (`schema::COMPOSITION`); question changes invalidate the cache by content.
- Validate on small frozen sets through the CLI; keep results in ignored
  `.jevgate/evaluation/`.
