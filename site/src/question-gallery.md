# Question gallery

Ready-made [custom questions](custom-questions.md) for conventions teams ask for and linters cannot check, each measured before it was shipped: asked of the code of real projects, with every finding labeled right or wrong by reading that code.

```sh
jevgate rules add swallowed-errors resource-leak   # writes .jevgate/questions/<name>.toml
jevgate check --rule custom --dry-run               # what they would ask and cost, offline
jevgate check --fail-on custom=report               # ask them without failing the gate
```

`jevgate rules add` writes each question's file into `.jevgate/questions/`, where it becomes a file the project owns: adapt its guidance and paths to the code, and commit it. It fails the gate at its level, as any custom question does, so try it with `--fail-on custom=report` first. `jevgate rules add` is offline and writes the wording the installed version measured; `--force` restores it over an edited file. Each file is also shown below, and kept in the repository's [`gallery/`](https://github.com/Tech-Byte-Frontier/jevgate/tree/main/gallery) directory.

| Question | Asked of each | Level | Projects | Units | Findings | Right |
|---|---|---|---|---|---|---|
| [todo-without-owner](#todo-without-owner) | comment | review at 0.95 | 7 | 973 | 31 | 31 (100%) |
| [swallowed-errors](#swallowed-errors) | function | review at 0.80 | 6 | 1,550 | 17 | 14 (82%) |
| [resource-leak](#resource-leak) | function | review at 0.80 | 6 | 1,212 | 14 | 12 (86%) |
| [thin-handlers](#thin-handlers) | request handler | review at 0.80 | 12 | 645 | 13 | 12 (92%) |
| [n-plus-one](#n-plus-one) | function | consider at 0.80 | 11 | 1,864 | 7 | 5 (71%) |

## How they were measured

Each question was asked of 6 to 12 projects where it applies, without the built-in questions, with jev-1.13.0 in September 2026. Every finding at the question's threshold was labeled from the code: right, wrong, or debatable when competent maintainers would disagree. A debatable finding counts as not right. The numbers are for the files as shipped: replayed from the cached answers, the files ask exactly what the measured runs asked.

A question ships at `review` when at least 80% of its findings were right over at least 10 findings, and at `consider` from 60%, or at 80% over fewer. A question's first wording was revised at most once, and a project whose findings informed the wording or threshold is counted as tuned; the sections say which. Of the 29 projects, 24 are open source and 5 are the maintainer's own. Four of them (microblog, laravel-realworld, nest-realworld and bakerydemo) were asked last, of the files as shipped: they added one right and one debatable n-plus-one finding, none in 79 more handlers, and two wrong findings that took a sixth question, global-state, out of the gallery.

The counts are small. They say how often a question is right when it fires, not how much it finds: no one labeled the units it cleared.

## Cost

Each unit carries the question's own text: its question, background and guidance, about 160 to 300 tokens for these. A unit whose source a built-in question already sends, such as a function beside function simplification, adds only that. Asked alone, a function question took 540 to 750 new input tokens per function and the comment question about 450 per comment (by dry run on mdbook and flysystem), so a first run over 1,000 functions costs about $0.03. Reruns, and units that did not change, are answered from the cache.

## todo-without-owner

Is a comment a TODO, FIXME, HACK or XXX that names neither a person or team to do the work nor an issue that tracks it? ruff's TD002 and TD003 check one format of owner in Python; this question reads free-form owners and links, such as `@maria` or `see #123`, in every language JevGate parses.

On 7 projects (973 comments), all 31 findings were right: gson's `// TODO: strip wildcards?`, shiori's `// FIXME: This only works in local filesystem`, fd's `// TODO: support writing raw bytes on unix?`, refined-github's `// TODO: Add support for PRs by detecting deferred-content wrappers`. The threshold is 0.95, chosen on the first four projects: at 0.80 the question also flagged gson's `// OK: will assume everything is accessible`, which holds no marker, and 26 of refined-github's dated TODOs, such as `// TODO [2027-01-01]: Drop after legacy PR files view is removed`, which a lint rule there fails once the date passes. They are tracked by that project's convention, so they were labeled debatable; 0.95 dropped all 27 and 4 right findings. On the three projects not used to choose it (fd, Online Boutique, refined-github), 14 of 14 were right.

At 0.95 a comment is clear only at 0.05 or below, so most comments stay undecided (562 of 973): they never fail the gate, and `--verbose` lists them. At 0.90, 91 stay undecided and 35 of 43 findings were right, the other 8 being refined-github's dated TODOs and the comment without a marker. If your project dates its TODOs, add dated TODOs to what guidance calls tracked. Five of the projects were asked only about the files that hold a TODO or FIXME marker.

```toml
{{#include ../../gallery/todo-without-owner.toml}}
```

## swallowed-errors

Does a function catch or receive an error that means something went wrong, and drop it without logging, returning, rethrowing or reporting it? A linter sees an empty `except` or `_ = err`; it cannot tell a failure hidden from an expected case handled on purpose, such as a division by zero that returns 0 or a missing optional file that gives defaults.

On 6 projects (1,550 functions), 14 of 17 findings were right. shiori's `UserConfig::Scan` ignores `json.Unmarshal`'s error and returns `nil`, so a corrupt stored config loads as defaults; `GenerateEbook` drops `AddImage`'s error, so a cover that fails to load leaves an ebook without one; Online Boutique's `getProductByID` returns on an error with neither a response nor a log. Wrong: shiori's `importHandler`, where every failure is printed and the ignored `fmt.Scanln` error only keeps the default answer. Debatable: Online Boutique's `placeOrderHandler`, which turns unparsable numbers into zeros that validation then rejects, and a webhook sender that returns only whether any webhook succeeded. Nine of the right findings are one of the maintainer's projects, which catches `Exception` and continues in its data fetchers.

The first wording also flagged expected cases handled on purpose: 7 of its 17 findings on shiori, django-debug-toolbar and one of the maintainer's projects were wrong. The guidance now names those cases, and those three projects count as tuned: 4 of 5 right there, 10 of 12 on the others.

```toml
{{#include ../../gallery/swallowed-errors.toml}}
```

## resource-leak

Does a function open a file, connection, cursor, stream or lock that it does not close on every path, errors included, and does not hand to its caller?

On 6 projects (1,212 functions), 12 of 14 findings were right, 9 of them in javavulnlab, an intentionally vulnerable Java application whose servlets never close their JDBC connections. The others: pgweb's `Tunnel::handleConnection`, which never closes the remote connection and leaves the local one open when the dial fails; Online Boutique's email client, which creates a gRPC channel per call and never closes it, and its `chatBotHandler`, which reads a response body without closing it. Wrong: Online Boutique's two `initTracing` functions, which hand the collector connection to the trace exporter for the life of the process. sqlite-utils, websocket and chi had no finding.

```toml
{{#include ../../gallery/resource-leak.toml}}
```

## thin-handlers

Does a request handler do business work itself, such as calculations, rules or several data changes, instead of reading the request, calling a service and building the response? It is asked of the functions in files its `paths` match: controllers, handlers, routes and views under common names. Change `paths` to where your handlers live.

On 12 projects (645 handlers), 12 of 13 findings were right, 11 of them in lobsters, whose Rails controllers hold the login rules, moderation records and karma changes (`LoginController::login`, `StoriesController::destroy`), and one in linkace, whose single sign-on callback links accounts and sets defaults for new users. Debatable: linkace's `saveAppSettings`, mostly input copied onto settings. The Django, Wagtail, ASP.NET, Express, Symfony, Laravel and NestJS projects had none.

```toml
{{#include ../../gallery/thin-handlers.toml}}
```

## n-plus-one

Does a function run a database query or a network call once per item of a loop, where one query or call could handle all the items?

On 11 projects (1,864 functions), 5 of 7 findings were right: linkace's HTML and CSV exports, which query each link's tags (and lists) over all of a user's links; lobsters' `MessagesController::batch_delete`, a query and a save per selected message; spring-realworld's `createNew` and laravel-realworld's `ArticleController::store`, a lookup and an insert per tag of an article. Debatable: linkace's `getOldTaxonomyItems`, one lookup per item of a form shown again after a validation error, a handful at most, and bakerydemo's random-data command, one insert per item, where `bulk_create` would skip the `save()` its models may rely on. It is a consider: 71% right.

```toml
{{#include ../../gallery/n-plus-one.toml}}
```

## Measured and left out

Ten more questions were measured the same way and are not shipped: under 60% right, or too few findings to measure.

| Question | Asked | Findings | Right | Why it is left out |
|---|---|---|---|---|
| global-state | Does this function both read and change a global, module-level or static variable? | 6 on 11 | 3 | Two Laravel model factories that count a static timestamp offset down on purpose, in seed data, and a `main` that only assigns a start time. The three right: functions that replace a module's database engine through `global`, and a loop that changes six module globals. |
| log-and-rethrow | Does this function log an error and then also return or rethrow it? | 7 on 7 projects | 4 | A gRPC handler that logs and passes the error to its callback, which answers the client; two logging decorators whose job is to log what passes through (debatable). |
| flaky-test | Can this test pass or fail from one run to the next with no code change? | 11 on 6 | 6 | Tests asserting that many random draws are not all equal, which fail with a negligible chance, and timing margins of seconds. |
| test-name-mismatch | Does this test's name promise what its assertions do not check? | 17 on 6 | 7 | Go test names that name the unit under test, type-level tests, a test whose check is the race detector; 0 of 5 right on projects it was not tuned on. |
| debug-output | Does this function print debugging output left over from investigating a bug? | 13 on 7 | 5 | Call traces of a service whose only logging is `Console.WriteLine`. Linters also catch most stray prints (`no-console`, ruff's T201, clippy's `dbg_macro`). |
| untranslated-text | Does this function put user-facing text into the interface as a literal instead of through the translation function? | 18 on 2 | 4 | Console-command output and example placeholders such as `R$` and `L, kg` read as interface text. Its first wording was right 25 times in 42 on four other projects, with 15 debatable demo screens. |
| money-in-float | Does this function store or compute money as a binary float? | 50 on 9 | 6 | Percentages, quantities, fee rates, simulations and display code read as money. |
| stale-comment | Does a comment in this function say something the code does not do? | 4 on 6 | 1 | Comments that describe what a callee or an override does. |
| undocumented-return | Does this public function return a special value for failure or not found without saying so? | 1 on 5 | 1 | One finding in 1,565 functions of five libraries: too few to measure. |
| hidden-side-effects | Does this function's name promise a lookup, check or conversion while it writes or sends? | 0 on 6 | | No finding in 916 functions once HTTP handlers named `get` were excluded; the one right finding of its first wording was lost. |
