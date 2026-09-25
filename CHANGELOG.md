# Changelog

Notable changes to JevGate. Versions follow [Semantic Versioning](https://semver.org); before 1.0, a minor version can change findings, flags and the report format.

## [Unreleased]

## [0.17.0] - 2026-09-25

- Release binaries for Linux (x86_64 and arm64, static), macOS (Apple silicon and Intel) and Windows (x86_64), with SHA-256 checksums and build provenance (`gh attestation verify`). `install.sh` installs a checked binary on Linux and macOS, and `cargo binstall jevgate` finds the archives on every platform.
- Windows: files are named with forward slashes, as on other platforms, so path rules (Next.js routes, Django modules, documentation roles) match there, and reports and baselines name a file the same way everywhere. Requests on Linux and macOS are unchanged, so cached answers stay valid.
- The project has a security policy with private reporting, a contributing guide, this changelog, and issue templates, including one for reporting a wrong finding.

## [0.16.0] - 2026-09-25

Checked against 40 open-source projects in every supported stack (Rust, Python, JavaScript/TypeScript, Go, C#, Java, PHP, Ruby, Svelte, Astro, Vue, Supabase SQL, GitHub Actions), with every review and consider labeled by hand. On the first 17, reviews went from 140 to 107 and considers from 935 to 293, mostly false positives and repeated findings removed; undecided units stayed near 2%.

- Test redundancy: two tests that check one behavior with different inputs are a note on their own; three or more linked by such pairs stay one consider. A review ("one adds nothing") needs both tests to share the input case and expected outcome, or to read the same apart from their names; tests in different groups whose setup is not sent are at most a consider. Parameterized tests are suggested in Rust only when the crate uses rstest, test-case or yare.
- Repeated findings are reported once: the pairs of a group of overlapping tests, and copies inside tests that a test-redundancy finding already names.
- Shared logic: copies of four lines or fewer are at most a consider, and short copies in test code lower still; docstrings, Go's `if err != nil` checks and `defer` cleanups, and lists of alike statements do not make a copy; variants of one example are not compared.
- Example code (`examples`, `demo`, `tutorial`, `docs_src`, top-level `samples`, `*.Examples.*` projects, Go `example_*_test.go`): findings are at most notes, injection and sensitive data at most considers, and hardcoded values are not judged.
- Skipped files: Rails (`db/migrate`) and Alembic (`alembic/versions`) migrations; scripts under `assets`, `static` or `vendor` that open with a whole license; shadcn components; files whose header says they were generated. SQL uninstall, teardown, rollback and down scripts are left out of the access-control state.
- SvelteKit: server loads, form actions, endpoints and hooks are named to Jev with who calls them and `cookies.set`'s secure defaults. Functions in an object literal (`export const actions = {…}`) and functions assigned to properties (`res.status = function…`, `Router.prototype.handle = …`) are units.
- Hardcoded values: protocol codes (HTTP status, file modes), environment variable and module names, hosts from configuration, and accounts the program creates are acceptable; module constants read from `require`, `process.env`, `os.getenv`, `ENV`, `env(…)` or `config(…)` are not values; a finding that names no value is a note.
- Comments: Sphinx version notes and `@author`/`@since` blocks are left out; framework section banners, doc blocks above declarations and PHP file headers read as documentation; optional code with "uncomment to enable" is not code turned off; comments in projects whose README says they are written for learners are at most notes.
- Follow-ups: an undecided cookie check is settled by what the cookie's flags come to; the mock-only recheck names errors around stubs, fields derived from them, expected mock calls, local test servers and hooks passed as options; Rust's random check names rand's secure generators; Ruby's exception check names Rails' 404 and 500 pages. SECURITY DEFINER functions are sent with the project functions they call.
- File organization: a consider that names no group is a note, and one whose choice leans toward two groups names both.
- The hardcoded-values, unsafe-settings, test-redundancy, file-organization and comments rule versions are bumped; the first run re-asks hardcoded-value requests and some follow-ups once. Other cached answers stay valid.

## [0.15.0] - 2026-09-25

- Functions, hardcoded values, security units and agent-instruction sections are packed into requests within runs of consecutive definitions, whose ends are chosen by each definition's name rather than its position, as code comments have been since 0.14.1 (#3). Adding, removing or resizing one function now re-asks only the requests of its own run: in a function-dense file, one request instead of every request after it (5 of 5 before, in `compose.rs`). Each edit re-sends 24–47% fewer tokens.
- Full runs send more, smaller requests: 25–106% more requests and 4–13% more billed input tokens on a first pass. With the answer cache kept between runs, as in CI, that is paid back after about 20–30 edits.
- The first run after upgrading re-asks most function, value, security and instruction requests once (37–88% of them, depending on the project), since their grouping changed. The function-simplification, hardcoded-values, injection, sensitive-data, unsafe-settings and agent-context rule versions are bumped. Comment and other requests are unchanged, so their cached answers stay valid.
- Shared logic no longer pairs two recursive tree walks as related steps when all they share is an early exit and the call to themselves (#4), and a function's call to itself is no longer listed as a difference between copies. Whole copies of small walks, and walks that do work around their recursion, are still reported. On eight projects, including four tree-sitter consumers, only the mistaken pair disappeared.

## [0.14.1] - 2026-09-25

- Code comments are packed into requests within runs of consecutive definitions, whose ends are chosen by each definition's name rather than its position. Adding or removing one comment now re-asks only the requests of its own run: in a comment-dense file, one request instead of every request after it (6 of 6 before, in `compose.rs`). Full runs send 12–53% more comment requests but only 1–2% more tokens.
- The first run after upgrading re-asks each comment once, since the grouping changed. No other rule's requests change, so their cached answers stay valid.

## [0.14.0] - 2026-09-25

- Code comments are judged: `documentation/comments` (opt-in with `--rule comments` or `--rule documentation`) asks whether each comment or docstring of application code only repeats the code next to it, holds sentences that add nothing, narrates an edit instead of describing the code as it is ("now uses…", "split out of… to stay under the budget"), or is code turned off. Each comment is sent with the declaration it documents, the lines below it or the line it ends; license headers, tool directives and type annotations are left out. An undecided comment is asked again with its whole definition, then what kind of comment it is.
- Comment findings are cleanups: at most a consider, documentation that only repeats its declaration at most a note, and one finding per function listing its comments by what is wrong with them, with an action to delete, shorten or rewrite. On seven projects, 44 of 57 considers were right when checked by hand, 11 debatable and 2 wrong; comment-rich codebases get none.
- `--rule documentation` now also reads code files for their comments. Configs that name `maintainability`, as `jevgate init` writes, are unchanged.
- Parsing, planning, composition and the question modules are split into smaller modules; every other rule sends byte-identical requests, so existing caches stay valid.

## [0.13.0] - 2026-09-24

- Java, C#, PHP and Ruby are judged. Java and C# get the maintainability, test and security rules; PHP gets security and tests (PHPUnit, Pest), with a page script's top-level code judged like a function; Ruby gets maintainability and tests (RSpec, Minitest), with each example's groups, hooks, `let`/`subject` and support helpers as evidence. JUnit, TestNG, xUnit, NUnit and MSTest tests are recognized.
- Frameworks: Django and Django REST framework (views with their routes and `|safe` templates, settings modules and the files that select them, CSRF exemptions, literal secrets, unsafe deserialization, error handlers), Next.js (route handlers, Server Actions, `pages/api`, middleware, client components, `dangerouslySetInnerHTML`, open redirects, raw Prisma and Drizzle queries, `NEXT_PUBLIC_` secrets, `next.config` headers), ASP.NET Core (controller actions, minimal APIs, exception handlers, EF Core raw SQL, JWT options and signing keys written in code), Slim and Laravel, and Spring MVC tests linked to the controller method they reach.
- Documentation rules also read MDX, reStructuredText and AsciiDoc. Undecided staleness, duplication and large-doc checks are settled by what a section treats its names as and how two sections relate, which removed most duplicate-section noise.
- Security checks that stay undecided get a narrow follow-up question (where redirect targets come from, how markup is rendered, which origins CORS allows, what logs write, where code runs), which can only clear a check.
- Tests: overlapping tests are grouped only when their pairs connect them; a test pair's subject skips getters, setters and fixtures most tests call; undecided pairs and mock-only checks are asked again with the code under test.
- Copies of three lines or fewer are at most a consider.

## [0.12.1] - 2026-09-24

- 0.12.0 did not build on Rust 1.90, its declared minimum (`if let` guards are not stable there). 0.12.1 builds on 1.90 and later; behavior is unchanged from 0.12.0.
- The test suite passes on macOS: test projects resolve their temp directory the way a check resolves the repository root.

## [0.12.0] - 2026-09-23 [YANKED]

Yanked: it did not build on Rust 1.90, its declared minimum. Use 0.12.1.

- Libraries copied into a repository (a versioned file name such as `jquery-3.6.0.js`, the readable build beside a `.min.js`, or a license banner naming a version) are skipped as vendored, whatever their size. A single vendored file no longer leaves a check incomplete.
- A test call counts only as its own statement, so a local `test()` inside library code no longer makes a file a test file, and a `Test*` class is a test only in files pytest collects.
- Security: SQL identifiers quoted by doubling embedded quotes count as handled, requests sent from a web page in the user's browser are excluded from the outbound-URL check, and an error-detail finding is first asked where the error text goes (a response, a terminal, a record) before being reported.
- Generated-code markers are read through a whole leading comment.

These changes come from running 0.11.0 on six open-source repositories it had never seen, where most security considers were wrong.

## [0.11.0] - 2026-09-23

- File organization judges large files and test files.
- An undecided file outline is asked what kind of file it is.
- Undecided URL and error-detail checks are settled by where values come from and where they go.
- Each error-handler registration is resolved in its own step.

## [0.10.0] - 2026-09-23

- Go code is judged.
- More kinds of codebases are judged, and findings that were wrong on fresh repositories are dropped.
- Headings are read when asking whether a section is a translation.

## [0.9.0] - 2026-09-23

- Findings say what to do next.
- SpacetimeDB modules are judged for access control.
- More undecided answers are settled with follow-ups.

## [0.8.0] - 2026-09-23

- `jevgate baseline --merge` accepts a partial check's findings and keeps the rest.
- Access-control checks see the claims a policy trusts and who may call a SECURITY DEFINER function.

## [0.7.0] - 2026-09-23

- SQL access control (row-level security, SECURITY DEFINER functions, grants) and GitHub Actions workflows are judged.
- Sharper test and hardcoded-value findings.
- Request bodies are sent as compact JSON.

## [0.6.0] - 2026-09-23

- The CLI is ready for CI: `--format github`, exit codes and the documentation for crates.io.

## [0.5.1] - 2026-09-23

- Notes no longer hide undecided answers or real findings.

## [0.5.0] - 2026-09-23

- Documentation rules (opt-in): agent instruction files, large project docs, finished plans, stale paths and repeated sections.

## [0.4.0] - 2026-09-23

- Security rules (opt-in): injection, sensitive data and unsafe settings.
- Rules are selected by group, gate levels are set per rule, and `jevgate init` writes a starting configuration.
- The HTML report shows finding categories.

## [0.3.0] - 2026-09-22

- Hardcoded values are judged: values that change between deployments, unnamed values and special cases.
- Optional notes are separate from considers, and split findings are located.
- Units that stay undecided are shown with the question they stayed undecided on.

## [0.2.1] - 2026-09-22

- No panic when the output pipe closes.

## [0.2.0] - 2026-09-22

- Focused evidence units replace one request per file: functions, file outlines, copy pairs and tests are asked about separately.
- Exit codes are the quality gate.
- Shared-logic findings need a located pair and a long shared span.

## [0.1.1] - 2026-09-18

- README usage commands explained.

## [0.1.0] - 2026-09-18

- First release: the maintainability CLI.

[Unreleased]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.17.0...HEAD
[0.17.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.14.1...v0.15.0
[0.14.1]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.14.0...v0.14.1
[0.14.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.12.1...v0.13.0
[0.12.1]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.12.0...v0.12.1
[0.12.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/Tech-Byte-Frontier/jevgate/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Tech-Byte-Frontier/jevgate/releases/tag/v0.1.0
