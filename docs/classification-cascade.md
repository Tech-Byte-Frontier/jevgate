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

1. **Eligibility and purpose.** Deterministic roles, generated-code headers,
   compiled or minified output (a trailing source map reference, or nine tenths
   of the file in lines of 1,000 bytes or more) and structural test markers
   decide which code the application rules and the test rules see. Only a test
   path without structural tests gets a file-purpose request. Go tests are
   `Test…`, `Benchmark…` and `Fuzz…` functions taking `*testing.T`, `.B` or `.F`.
   Astro, Vue and
   Svelte files are parsed as their scripts: Astro frontmatter and `<script>`
   contents, with every other byte a space, so lines stay the file's.
2. **Local analysis** (`src/analysis/`). Units with signatures, calls, references
   and control-flow nesting; callbacks registered through calls, including
   module-level route handlers named by their registration
   (`app.post('/pages')`); member groups by average linkage; callers that
   import the file (application code only, since tests calling a group do not
   make it a dependency); for file organization, each member's line count and
   the file's, and for a test file, its cases with their enclosing
   `describe`, class or module and the functions under test they call, grouped
   by shared suite, subject or helper. Test files get this outline without
   `--include-tests`; their finding is at most a consider. Who calls a group is
   evidence only: gating a split on callers of its own hid large files whose
   single caller is the rest of the program; Type-2 clone candidates grouped by overlapping copies, only
   within one package or packages linked by a local dependency (copies in side
   by side templates or example apps are separate projects); test cases with
   their subjects and similar pairs.
3. **First pass** (`src/units/`). One dispatch of every unit request. Functions
   are packed eight per request; tests are sent one per request, because unrelated
   tests in the same state left more answers undecided. State uses literal paths
   such as `functions[2].source`; group IDs are Choice options. Stage and freshness
   hashes stay in local `jevgate` metadata that is not uploaded.
4. **Follow-ups.** One recheck per uncertain unit, with callee signatures, the
   enclosing functions or the file's application source; a decisive recheck
   replaces the first answer and both are kept. A hardcoded-value unit is asked
   instead whether every value is of an acceptable kind; that check can only
   clear, since re-asking the concern per value added false findings. The
   unnamed-value check lists the kinds in its question: asked whether each
   value "explains itself", it cleared none, even of field names.
   A function or file-organization note whose middle and top levels both
   stay under 0.50 gets the same recheck, and a decisive answer replaces it.
   A test left undecided on whether it re-implements the code or checks only
   its mocks is asked again with the bodies of the functions it calls and its
   file's imports, mocks and setup hooks (a part too long is left out, never
   cut); each answer replaces the first unless only the first is decisive.
   The first pass names a literal worked out by hand, even with the
   arithmetic in a comment, as not re-implementing the code.
   Then one locate Choice per split finding picks the body block to extract,
   and one per hardcoded-value review or consider names the value it is about.
   Special-case findings in different files that name the same identity
   become one finding at the strongest site; the others are notes pointing at
   it. Numbers and paths are not grouped: `1000` meant metres per kilometre in
   one file and an image height in another.
   Security units whose presence answers are not clear get one trace before the
   rechecks: specific literal checks per kind, a Choice among the unit's sites,
   and the origin of its values (or whether it runs only in development). One
   broad "is every value bound, escaped or checked?" stayed undecided even for
   `eval` of model output; a check per kind decides and names the kind. An
   injection whose origin stays unclear or is the function's parameters is
   asked its origin and checks again with up to three callers; its answer
   replaces the traced one unless only the traced one is decisive.
   The markup check names text shown as a JSX child and CSS values or class
   names as escaped or inert; sending where each built string goes did not
   settle React units, the examples did. A sensitive-data trace lists the
   message argument of each error the function creates and asks which one,
   if any, carries another error's text: the response is often written by an
   error handler in another file, and adding the handler to every unit also
   cleared real leaks. Each registered error handler (`.onError(…)`,
   `.setErrorHandler(…)`, Express four-parameter `.use(…)` middleware, Flask
   and FastAPI decorators, NestJS `@Catch` filters, axum `IntoResponse` and
   actix-web `ResponseError` for an error type, Rocket catchers) is asked once
   whether it sends clients more than the program's own messages and codes,
   with the program's `…Error` classes (Rust enums with their `#[error]`
   messages) and the functions of its file that it calls. An injection trace
   also gets the definitions of enums its sites name (`ConfigKey.aiTag`), so
   a fixed choice does not read as a parameter.
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
   Staleness and duplication candidates come from code. Staleness candidates
   are paths and scripts a section names that the repository lacks, with what
   Git shows about each. Duplication candidates are section pairs where 30%
   of the smaller section's three-word sequences recur in the other, at most
   three per pair of documents. A document whose release is tagged, or whose
   named paths were deleted, is asked from its headings whether it is a plan;
   a plan with those facts is one finding, and its own candidates are not
   asked. The section check and the pair Nouls (does A state everything B
   states, and the reverse; do they disagree; is one a translation of the
   other?) are follow-ups for the other documents. A translation clears the
   repetition answers but not a disagreement. A Score on how two sections relate stayed on its middle level
   for almost every pair, so it is not asked.
   SQL files (`security/access-control`) are split into statements that honor
   comments, quotes and dollar quotes. Each project's files, grouped above
   their `supabase` or `migrations` directory, are read in path order, so a
   dropped or replaced policy or function is not judged. Each policy is sent
   with its table, the functions it calls, and the functions that set the
   token claims it reads, such as a custom access token hook: without the
   hook, 63 of one project's policies stayed undecided on whether users can
   change the claim. A SECURITY DEFINER function goes with its grants and
   revokes of EXECUTE; a grant with whether its table has row-level security.
   The criteria name role checks, service roles, restrictive policies and
   trigger functions, which a literal "other users' rows" question flagged.
   SpacetimeDB modules (TypeScript files that import `spacetimedb/server`,
   Rust files with `#[table]`, `#[reducer]` or `#[view]` attributes) are read
   for access control too, whatever the application: each public table with
   the columns that name users, and each view and reducer with up to six functions it calls (two
   deep, in the module's package). Lifecycle reducers are left out; a Rust
   reducer is sent with the table line that schedules it (`scheduled_by`), since
   the schedule is declared on the table. The framework version comes from the
   package's `package.json` or `Cargo.toml`, since scheduled reducers are
   private in 2.x and callable by clients in 1.x; with no version, both are
   stated. Each definition gets a Score whose two lower
   levels are acceptable (the caller's own data; data meant for every user)
   and concern Nouls: for a reducer, two literal checks (a row chosen by an
   argument without an ownership check; an admin-only change without checking
   the owner, an admin or a granting role). One broad Noul left most real definitions undecided and
   most broken reducers under 0.80. When access control is the only code
   rule, only the files of module packages are collected.
   Workflow jobs (`security/workflows`) are split by indentation. The parser
   lists the `${{ }}` expressions inside `run` scripts, and Jev is asked
   whether one can hold text outside people write: one question over the
   whole job scored obvious injections 0.57 to 0.79. Jobs of workflows that
   run on `pull_request_target` or `workflow_run` are asked whether they run
   pull request code with secrets.
5. **Composition** (`src/units/compose.rs`). Pure. On a Score whose top level is
   the actionable concern: review at 0.80 on the top level, consider at 0.80 on
   middle-or-top, clear when the top level is ruled out at 0.80, otherwise
   uncertain. Where the middle level says the code is fine as it is, a consider
   also needs the top level at 0.50; middle mass alone is an optional note. For hardcoded values and security, an answer still
   undecided after its follow-up is a note when it leans toward the concern
   (0.50, the leading probability) and stays uncertain otherwise: undecided
   answers leaning away were almost all acceptable code, and leaning toward
   held both real positives of the labeled set. When the own-messages check
   finds another error's text in an error message, an error-detail answer
   leaning toward a client is a consider. Instruction sections are
   cleanups, so their findings are at most a consider. Undecided answers do not
   lean into notes, because on the labeled set that added notes to kept sections.
   A large document's undecided history answer does lean into a note. Policies,
   grants and an open `search_path` are at most a consider, since a policy
   may cover data meant for everyone; an unchecked SECURITY DEFINER function
   and workflow findings can be reviews. A SpacetimeDB definition is a review
   when a concern Noul or the Score's top level reaches 0.80, and clear when
   the Score's acceptable levels do and nothing is at review; public tables
   and views are at most a consider, reducers can be reviews. A
   hardcoded-value review or consider whose value the locate Choice could not
   name is one level lower. Messages show the probability that set a finding's
   level (a consider shows the middle-or-top mass, not the top level); notes
   show none. Finished plans in one directory become one finding identified by
   the directory, and the others become notes pointing at it. On its
   labeled set, no living document leaned past 0.50. Questions ask whether a change would help a reader ("would splitting
   it make it easier to understand?"), not how many tasks or purposes there are:
   Jev does not count reliably and reads "tasks" literally. Copies inside test
   cases are one level lower. A test that checks several unrelated behaviors is at
   most a note: on labeled tests, tables of inputs and browser journeys rated
   as high as tests that really mix behaviors. A review always carries a finding.
6. **Gate.** `--fail-on`, `[[scope]]` levels per path and the baseline act on
   composed findings only. Baseline entries can carry a reason (`intended`,
   `later`, `wrong`) that survives rewrites; `baseline stats` counts them.

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
