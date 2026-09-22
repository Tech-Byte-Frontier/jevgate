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

1. **Eligibility and purpose.** Deterministic roles and structural test markers
   decide which code the application rules and the test rules see. A test path
   that still contains other code gets one file-purpose request.
2. **Local analysis** (`src/analysis/`). Units with signatures, calls and
   references; member groups by average linkage over calls, owners and
   file-declared types; Type-2 clone candidates across the selected files and
   explicit context; test cases with their subjects and similar pairs.
3. **First pass** (`src/units/`). One dispatch of every unit request. State uses
   literal paths such as `functions[2].source`; group IDs are Choice options.
   Stage and freshness hashes stay in local `jevgate` metadata that is not uploaded.
4. **Recheck.** One request per uncertain unit, with callee signatures or the
   enclosing functions. A decisive recheck replaces the first answer; both are kept.
5. **Composition** (`src/units/compose.rs`). Pure: review at 0.80 on a Score's
   top level (or a Noul), clear at 0.80 on the bottom level, consider when the
   middle-or-top mass reaches 0.80, otherwise uncertain. A review always carries
   a finding, file-wide when no location is decisive.
6. **Gate.** `--fail-on` and the baseline act on composed findings only.

## Constraints

- Syntax supplies units, candidates and locations, never verdicts.
- Keep questions atomic and literal; no thresholds, hashes or self-descriptions in
  uploaded state or questions.
- Preserve raw answers, uncertainty and needs-context outcomes.
- Version question wording (`units::questions::VERSION`) and composition
  (`schema::COMPOSITION`); question changes invalidate the cache by content.
- Validate on small frozen sets through the CLI; keep results in ignored
  `.jevgate/evaluation/`.
