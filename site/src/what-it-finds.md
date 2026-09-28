# What it finds

**Maintainability** (on by default, except hardcoded values)

| Rule | Example finding |
|---|---|
| File organization | This file holds several features that would be easier to find apart; the upload helpers would be most useful as their own module. Test files are judged too, at most as a consider. |
| Function simplification | `sync_accounts` mixes separate jobs in long blocks; lines 40–71 would be most useful as their own function. |
| Shared logic | `createInvoice` and `createReceipt` perform the same steps; one shared implementation would serve both. |
| Hardcoded values (opt-in: `--rule default --rule hardcoded-values`) | Module constants fix a value that differs between deployments; `apply_discount` special-cases one specific customer. On projects JevGate was never tuned on, 6 of its 37 labeled findings were right, against 47 of 85 on the projects it was tuned on, so it no longer runs by default. |

**Tests** (with `--include-tests`; file organization judges test files without it, and the laws of Bend 2 code are judged where they are)

| Rule | Example finding |
|---|---|
| Test value | `test_total` computes its expected value with the logic it tests. |
| Test redundancy | Three tests of `parse_date` check the same behavior; one parameterized test could hold them. |
| Laws (Bend 2) | The comment above law `body_after_blank` promises more than the law states: it says the text after a blank line is the body, and the law checks only texts that open with the blank line, so an `http_body` that returns a real response's headers could pass every proof. |

**Security** (opt-in with `--rule security`; each finding names a CWE)

| Rule | Covers |
|---|---|
| Injection | Variables reaching SQL, shell commands, evaluated code, HTML, file paths, outbound URLs or redirect targets without binding, escaping or checks; data from another party given to a deserializer that can build any object (`pickle`, `yaml.load`, `Marshal.load`, `ObjectInputStream`, node-serialize; in C#, types named by input or chosen by the data being deserialized; in PHP, `unserialize`) or to an XML parser that resolves external entities; in PHP, uploaded file names; in server templates, request, cookie, session or signed-in-user data written unescaped (`<%= raw cookies[:font] %>`), JSP scriptlets that query or run commands with request parameters, and inline scripts that write `location.hash` into the page |
| Sensitive data | Passwords, tokens or personal data written to logs; internal error details sent to clients, judged per error message and once per error handler (`app.onError`, `setErrorHandler`, Express error middleware, Flask and FastAPI handlers, Django error views and `process_exception` middleware, Django REST framework's `EXCEPTION_HANDLER`, NestJS filters, axum `IntoResponse` and actix-web `ResponseError` for error types, ASP.NET Core exception handlers, PHP `set_exception_handler`, Slim and Laravel handler classes); in Django code, also the server's environment or settings sent to clients (`request.META`) |
| Unsafe settings | Certificate checks turned off, passwords kept as plain text or hashed with a fast hash, non-cryptographic random secrets, permissive CORS, session cookies without `Secure`/`HttpOnly`, secrets in environment variables the build puts into browser code (`NEXT_PUBLIC_`, `VITE_`), HTML escaping turned off (`autoescape: false`), tokens accepted without checking their signature or expiry, and signing or encryption keys written in the code; in C#, also developer exception pages outside development and secrets derived from data others know; in Django code, also debug mode for the deployed site, `csrf_exempt` views and secret keys written in settings |
| Access control | SQL row-level policies that let every user reach other users' rows or trust `user_metadata`; SECURITY DEFINER functions without a fixed `search_path` or a caller check; grants that open writes to every user. SpacetimeDB modules (TypeScript and Rust, any kind of application): public tables of users' private data, views that return other users' rows, reducers that change rows their arguments choose or admin-only settings without checking the caller, and scheduled reducers clients can call in 1.x |
| Workflows | GitHub Actions `run` scripts that execute text outside people write (`${{ github.event.pull_request.title }}`); `pull_request_target` or `workflow_run` jobs that run pull request code with secrets |

**Documentation** (opt-in with `--rule documentation`)

| Rule | Example finding |
|---|---|
| Agent context | A section of `CLAUDE.md` only lists the scripts `package.json` already shows, and six harnesses load it at the start of every session. |
| Large docs | `docs/operations/runbook.md` holds several unrelated subjects; `docs/plans/v0.2-plan.md` mainly records finished work. |
| Staleness | `docs/plans/v0.4-auth.md` is a plan whose work is finished: the repository has a release tag v0.4.0, and 6 paths it names were since removed. |
| Duplication | Section `Release Workflow` of `CLAUDE.md` states everything section `Release` of `README.md` states. |
| Code comments | `save_skill` has 3 comments to clean up: at lines 214, 218 and 222 they repeat the code (`# Create skill directory` above `skill_dir.mkdir(…)`). A module docstring saying it was "split out of `portfolio.py` to stay under the 500-line budget" narrates an edit instead of the code as it is. |

The documentation rules read the instruction files that coding agents load (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and Claude, Cursor, Copilot, Windsurf, Cline, Kiro, Junie and Roo Code rules), even when hidden or gitignored. Each section is asked whether it only restates the stack, the manifest's commands, generic advice or a configured linter's rules, and whether text loaded in every session applies to only one directory. Project documentation in Markdown, MDX, reStructuredText or AsciiDoc of 300 or more lines is judged from its headings alone. Code finds staleness and duplication candidates: named paths or scripts that no longer exist, release tags, deleted files, and shared wording outside code examples. Jev then judges each candidate. Code comments and docstrings of application code are judged one at a time with the code they are about (the declaration they document, the lines below them or the line they end): whether they only repeat that code, hold sentences that add nothing, narrate an edit instead of the code as it is, or are code turned off. License headers, tool directives, type annotations, authorship tags and Sphinx version notes are left out; documentation that only repeats its declaration or says it at length (framework section banners included) is at most a `note`, as is every such comment in a project whose README says its code is written for learners; and the comments of one definition that span fewer than three lines in all are a `note`. Documentation findings are at most `consider`. The run also estimates the tokens each harness loads at session start; these estimates are evidence and never fail the gate.

`jevgate rules` prints every rule with its question, its default, and how often its reviews and considers were right on projects JevGate was never tuned on; by default only the levels right at least 80% of the time there fail the check. The [rules reference](reference/rules.md) lists them with what each one looks at.
