# Supported languages and frameworks

Each language is supported or in preview. The ten languages with analyzers of their own are supported. The nine the [generic tier](#generic-support) reads are in preview: one becomes supported when, on each of two projects JevGate was never tuned on, one of its rules and levels is right at least 80% of the time over at least 20 labeled findings. None does yet. A preview language's findings are reported like any other and never fail the default gate: each says how often its rule and level were right in that language, not in the supported ones, and the agent text lists its files with the rules that read them. An explicit level counts them as it says (`--fail-on review` fails on every review), and a [custom question](custom-questions.md)'s findings fail at the question's own level in every language. [Support levels](#support-levels) gives each language's level and how often its findings were right.

✅ judged · ✗ not judged yet · ➖ not applicable

| Language or file | Extensions | Maintainability | Tests | Security | Documentation |
|---|---|:---:|:---:|:---:|:---:|
| Rust | `.rs` | ✅ | ✅ `#[test]`, `#[cfg(test)]` | ✅ | ✅ comments |
| Python | `.py` | ✅ | ✅ pytest, unittest | ✅ | ✅ comments |
| JavaScript | `.js` `.jsx` `.mjs` `.cjs` | ✅ | ✅ `describe`/`it`/`test`, `Deno.test` | ✅ | ✅ comments |
| TypeScript | `.ts` `.tsx` `.mts` `.cts` | ✅ | ✅ `describe`/`it`/`test`, `Deno.test` | ✅ | ✅ comments |
| Go | `.go` | ✅ | ✅ `Test…(t *testing.T)` | ✅ | ✅ comments |
| C# | `.cs` | ✅ | ✅ xUnit, NUnit, MSTest | ✅ | ✅ comments |
| Ruby | `.rb` | ✅ | ✅ RSpec, Minitest, Rails `test "…" do` | ✅ no Ruby framework handlers yet | ✅ comments |
| PHP | `.php` `.phtml` | ✅ | ✅ PHPUnit `…TestCase` classes, Pest `test`/`it` | ✅ | ✅ comments |
| Java | `.java` | ✅ | ✅ JUnit 4 and 5, TestNG: `@Test`, `@ParameterizedTest`, `@Nested`, JUnit 3 `TestCase` | ✅ | ✅ comments |
| Bend 2 ([bendlang/bend](https://github.com/bendlang/bend) 2.0.x) | `.bend` | ✅ | ✅ programs ending in the `#\|` lines their run must print, or defining `main` on a test path; laws (`tests/laws`) | ✅ defs that perform effects or build text | ✅ comments |
| C, C++, Kotlin, Swift, Bash, Dart, Scala, Elixir, Lua ([generic support](#generic-support), preview) | `.c` `.h` · `.cpp` `.cc` `.cxx` `.hpp` `.hh` `.hxx` · `.kt` `.kts` · `.swift` · `.sh` `.bash` · `.dart` · `.scala` · `.ex` `.exs` · `.lua` | ✅ function simplification, file organization, shared logic | ✗ test files are found by path and not judged yet | ✗ no sources or sinks known for these languages yet | ✅ comments |
| Astro, Vue, Svelte | `.astro` `.vue` `.svelte` | ✅ scripts only | ➖ | ✅ scripts only | ✅ script comments |
| Server templates: ERB, EJS, JSP, Handlebars, Mustache, Nunjucks, Twig, Jinja, Go | `.erb` `.ejs` `.jsp` `.hbs` `.mustache` `.njk` `.twig` `.jinja` `.j2` `.tmpl` `.gohtml`, and `.html` under `templates/`, `views/`, `layouts/`, `partials/` or `includes/` | ✅ inline scripts only | ➖ | ✅ inline scripts, as the page's code in the visitor's browser; and the code that reads the request, a cookie, the session or the signed-in user: tags that write it unescaped (`<%= raw … %>`, `.html_safe`, `<%== … %>`, `<%- … %>`, `{{{ … }}}`, `\|safe`, `\|raw`) and a JSP page's scriptlets | ✅ script comments |
| SQL (PostgreSQL, Supabase) | `.sql` | ➖ | ➖ | ✅ access control | ➖ |
| GitHub Actions | `.github/workflows/*.yml` | ➖ | ➖ | ✅ workflows | ➖ |
| Markdown, MDX | `.md` `.mdx` at the root, in `docs/` or `doc/`, READMEs and CONTRIBUTING files; agent instruction files; Claude Code skills, commands and subagents | ➖ | ➖ | ➖ | ✅ |
| reStructuredText, AsciiDoc | `.rst` `.adoc` `.asciidoc`, in the same places | ➖ | ➖ | ➖ | ✅ |

| Framework or platform | What JevGate understands |
|---|---|
| Hono, Express, Fastify, Koa | Route handlers written inline (`app.post('/pages', async (c) => …)`); error handlers (`app.onError`, `setErrorHandler`, four-parameter Express middleware); views rendered by name (`res.render('app/products')`), with the lines that write values unescaped (EJS `<%- … %>`, Handlebars `{{{ … }}}`, Pug `!=`, Nunjucks and Swig `\|safe`) |
| NestJS | Exception filters (`@Catch`) |
| Next.js (App Router and Pages Router) | Route handlers (`app/**/route.ts`), Server Actions (`'use server'` files and functions), `pages/api` routes, middleware, client components, error boundaries and pages are named to Jev with who calls them and where they run, so a Server Action's arguments read as client input and a client component's requests as the user's own; `dangerouslySetInnerHTML`, redirects to client-chosen URLs, raw Prisma and Drizzle queries (`$queryRawUnsafe`, `sql.raw`) as opposed to their binding tagged templates, `NEXT_PUBLIC_` secrets, and `next.config` headers |
| SvelteKit | Server load functions and form actions (`+page.server.js`, `export const actions = {…}`), endpoints (`+server.js`) and server hooks are named to Jev with who calls them, so their request, form data, URL and cookies read as client input, and `cookies.set` is read with its secure defaults |
| Flask, FastAPI | Error handlers (`@app.errorhandler`, `@app.exception_handler`) |
| Desktop, game and terminal programs | A package that depends on an interface toolkit (`ratatui`, `egui`, `iced`, `bevy`, `tauri`, `electron`, `spacetimedb-sdk` and others) is named to Jev as a client application, so the errors it shows its own screens go to its user, not to a remote client |
| GraphQL in Python (graphene, strawberry, ariadne) | A file that imports the library is named to Jev as GraphQL server code, so the arguments of its resolvers (`resolve_*`, `mutate`, strawberry fields and mutations, ariadne field functions) read as client input |
| Django, Django REST framework | Views and viewsets with the URL routes that reach them, the templates they render with `\|safe` or autoescaping off, and the module constants they use; settings modules, with secret literals redacted, the settings modules that import and override them, and the files that select them (`DJANGO_SETTINGS_MODULE`); management commands as run by hand; `handler500`-style error views, middleware `process_exception` and `EXCEPTION_HANDLER` as error handlers |
| PHP pages, Slim, Laravel | A file's top-level code is judged like a function, since a page script reads the request and writes the response; route closures (`$app->get('/users', function …)`, `Route::post(…)`) and configuration closures (`return function (App $app) {…}`); error handlers (`set_exception_handler`, subclasses of Slim's `ErrorHandler` and Laravel's `ExceptionHandler`); a Laravel app's `config/*.php` files, which the framework and its packages publish, are not read for comments |
| axum, actix-web, Rocket | Error responses (`IntoResponse` or `ResponseError` for an error type, `#[catch]`) |
| MDX sites (Next.js, Nextra, Docusaurus, Astro) | Imports, exports, comments and component markup are dropped, but the prose components carry (a `<Note>`'s text, a properties table's descriptions, with names and types as code) is read; frontmatter `title` names the page |
| Sphinx, AsciiDoc | Section titles by their adornment or `=` level; comments, attribute entries and options dropped; code directives, literal and listing blocks read as code; `:file:` and `include::` targets checked as paths, `:attr:` and `:class:` read as code, and `_build/` skipped |
| ASP.NET Core | Controller actions, minimal API route handlers (`app.MapGet("/orders", …)`) and inline middleware; exception handlers (`UseExceptionHandler` with a handler, `IExceptionFilter`, `IExceptionHandler`, middleware classes that catch what the pipeline throws); Entity Framework Core raw and interpolated SQL, CORS and cookie options, `UseDeveloperExceptionPage`, JWT validation options and the constants a setup names |
| .NET projects | Test projects named like `Shop.Tests` or `Shop.UnitTests`, and classes of `[Fact]`, `[Theory]`, `[Test]` or `[TestMethod]` methods anywhere; designer and source-generated files (`.Designer.cs`, `.g.cs`) are skipped as generated |
| Supabase and PostgreSQL | Row-level security policies, `SECURITY DEFINER` functions, grants, and the claims an access token hook sets |
| SpacetimeDB (TypeScript and Rust modules) | Public tables, views and reducers, checked against the caller (`ctx.sender`) and the framework version's scheduling rules |
| React | JSX components and hooks as functions; text shown as a JSX child is not treated as markup injection |
| RSpec, Minitest | Examples with the groups they are declared in, the `before` hooks and the `let`/`subject` definitions they read, and the helpers they call from support files such as `spec/support` and `test_helper.rb` |
| Sinatra and other Ruby DSLs | Methods of classes and modules (`def`, `def self.`, `class << self`, `define_method`); blocks passed at class or file level as units named by their call (`get('/invoices')`); constants as hardcoded values |
| Monorepos and examples | Copies are compared within a package and across packages linked by a local dependency, not across separate example apps, templates or variants of one example (`examples/login/raw` and `examples/login/sdk`); copies inside example code are notes; directories below a JVM source root (`src/main/java/com/example/demo`) are packages, not examples |
| Go packages | A file's package is its directory: the files of its package and the packages its imports name are its callers and callees, so an injection is judged with the handlers that call its query helper |
| Java classes | Methods and constructors belong to their class, interface, enum constant or record; `static` fields are constants; `equals` and `hashCode` overrides, constructors storing fields and setters given literals are boilerplate or data, never copies; initial capacities and a number a method returns whole are not values to name; a class of the same package counts as imported |
| Spring MVC | A MockMvc or RestTemplate test request reaches the controller method whose `@GetMapping`, `@PostMapping` or `@RequestMapping` route serves it, so the test is judged with that method as its code under test |
| Bundlers and compilers | Minified and compiled output (a source map reference, very long lines) is skipped as generated |
| Copied libraries | A library copied into the repository (a versioned file name such as `jquery-3.6.0.js`, the readable build beside a `.min.js`, a license banner naming a version, or a script under `assets`, `static` or `vendor` that opens with a whole license and copyright) is skipped as vendored, whatever its size |
| Project templates (cookiecutter, copier) | Files under a directory named with a `{{ … }}` placeholder are parsed without their Jinja tags, so the generated project's code is judged instead of skipped for syntax errors |
| Bend 2 | Defs, types and laws are units, named with their dots (`List.map`), and a call through an import alias (`Sort.sort` with `import ./main.bend as Sort`) reaches the def it names. A law is a claim when it states an equality, asks for a witness or applies a def that computes a type, and the def of its name is its proof; proofs (including every def of a `PROOF.bend`, a `*_proof.bend` or a file under `proofs/`) are not asked to be split, proofs and type-level defs are not asked about hardcoded values, and only defs that perform effects (`IO`) or join text with `++` are asked the security questions, since the rest are pure. `LAWS.bend`, `PROOF.bend` and files of fewer than 300 member lines are not asked to be split, a split of a file is at most a consider and a note in a file laid out in titled sections, a benchmark's values are not asked about, a program on a test path that defines `main` is a test, a zero-argument def returning a number names it, a `base.bend` copied from Bend's Base library is vendored, and Jev is told Bend 2's notation beside each file. Bend 1 files, a different language with the same `.bend` extension, are skipped with that reason |
| Migrations | Directories named `migrations`, Rails' `db/migrate` and timestamped scripts under `db/`, and Alembic's `alembic/versions` are skipped as migrations; SQL migrations are still read for access control |

Other files, such as Zig, are listed as skipped with the reason and never fail the gate.

## Support levels

| Language | Level | Unseen projects | Reviews right | Considers right |
|---|---|---:|---:|---:|
| Rust | supported | 7 | 69% (37 of 54) | 64% (129 of 202) |
| Python | supported | 4 | 51% (18 of 35) | 52% (43 of 82) |
| Go | supported | 3 | 8 of 10 | 57% (24 of 42) |
| TypeScript | supported | 4 | 2 of 4 | 58% (14 of 24) |
| PHP | supported | 2 | 1 of 6 | 11 of 18 |
| Java | supported | 2 | 1 of 3 | 4 of 12 |
| JavaScript | supported | 3 | 1 of 1 | 1 of 5 |
| C#, Ruby, Bend 2 | supported | none | not measured | not measured |
| C | preview | 4 | 64% (16 of 25) | 44% (15 of 34) |
| C++ | preview | 6 | 58% (23 of 40) | 42% (22 of 52) |
| Kotlin | preview | 3 | 8 of 9 | 9 of 11 |
| Swift | preview | 5 | 82% (28 of 34) | 70% (44 of 63) |
| Bash | preview | 8 | 45% (29 of 64) | 62% (56 of 90) |
| Dart | preview | 3 | 8 of 12 | 13 of 16 |
| Scala | preview | 4 | 3 of 5 | 43% (9 of 21) |
| Elixir | preview | 5 | 5 of 6 | 10 of 13 |
| Lua | preview | 4 | 90% (19 of 21) | 51% (19 of 37) |

A finding is right when a person reading the code agrees with it; a debatable one counts as not right. A percentage is shown from 20 labels on. The counts are for the four rules every language gets: function simplification, file organization, shared logic and comments.

- The supported languages' counts are 0.25.0's reviews and considers on the 25 projects JevGate was never tuned on (11 held out, 14 fresh), each labeled by hand from the code. Those projects hold no C#, Ruby or Bend 2 finding of these rules, so those three rest on the projects used for tuning.
- The preview languages' counts are 0.30's first run of the same four rules on 37 well-known projects chosen for them and never used for tuning, with all 598 of its findings labeled by hand. Shared-logic considers are counted as the same-steps threshold of [0.28](changelog.md) reports them, fitted on the supported languages: of the 104 that run reported, it makes 40 notes, 31 of them not right, and the other 64 were right 26 times, where all 104 were right 35 times. A language's projects are the ones holding a labeled finding in its files: dio's Flutter runners count for C++ and Swift, and leveldb's C++ headers, which that run read as C, count for C.

Maturity is judged per rule and level, which the pooled rows hide:

| Preview language | Function simplification | Shared logic | Comments | File organization |
|---|---|---|---|---|
| C | 10 of 11 · 13 of 19 | 6 of 14 · 2 of 6 | – · 0 of 8 | – · 0 of 1 |
| C++ | 11 of 11 · 19 of 37 | 12 of 29 · 1 of 6 | – · 1 of 6 | – · 1 of 3 |
| Kotlin | 1 of 1 · 6 of 7 | 6 of 7 · 2 of 2 | – · 1 of 2 | 1 of 1 · – |
| Swift | 10 of 10 · 22 of 30 | 17 of 23 · 16 of 27 | – · 4 of 4 | 1 of 1 · 2 of 2 |
| Bash | 25 of 28 · 34 of 50 | 4 of 34 · 0 of 10 | – · 22 of 28 | 0 of 2 · 0 of 2 |
| Dart | 5 of 5 · 9 of 10 | 3 of 7 · 1 of 1 | – · 2 of 4 | – · 1 of 1 |
| Scala | 2 of 3 · 5 of 11 | 1 of 2 · 1 of 2 | – · 3 of 5 | – · 0 of 3 |
| Elixir | – · 7 of 8 | 4 of 5 · 3 of 5 | – | 1 of 1 · – |
| Lua | 11 of 12 · 18 of 22 | 8 of 9 · 0 of 5 | – · 1 of 10 | – |

Each cell is reviews right, then considers right, and each finding in a preview language carries its own cell: a Kotlin function-simplification review says "Not yet measured in Kotlin.", a Swift function-simplification consider "Right 73% of the time in Swift (30 labels).". No preview language has two projects that meet the bar. The closest are Bash's function-simplification reviews, 25 of 28 over three projects (nvm 7 of 7, pi-hole 9 of 9, setup-ipsec-vpn 9 of 12) with none reaching 20 on its own; pi-hole's Bash comment considers (20 of 25) and Rectangle's Swift shared-logic reviews (17 of 21) meet the bar on one project each. Pooled over projects, Bash's function-simplification reviews and Lua's function-simplification considers (18 of 22) are above 80% over at least 20 labels.

The measurement's labels led to fixes that change findings on some of these projects, which count as tuned for those rules from now on: Bash copies pair across scripts only through `source` (setup-ipsec-vpn, tmux-resurrect), Flutter's platform runners are generated code (dio), C++ headers named `.h` are read as C++ (leveldb), C and C++ tests are found by name (beanstalkd, json11) and Kotest's `…Spec` classes only in test directories (kotlinconf-app), a comment of Lua language server annotations is not prose (nvim-cmp, which-key), and C++ members behind pointers and references, operators, Swift computed properties and Kotlin `init` blocks and accessors are units. The counts above are the run before these fixes.

## Generic support

C, C++, Kotlin, Swift, Bash, Dart, Scala, Elixir and Lua are in preview. They are read through one tree-sitter tag query per language, written in the captures GitHub's code navigation uses (`@definition.function`, `@definition.class`, `@reference.call`): it finds functions, methods, types and the calls each makes, and a table per language names the nodes that hold statements, nest control flow and hold literals. The units include C++ members defined outside their class (`ns::Cart::add`), those returning a reference or pointer, and operators; Swift computed properties (a SwiftUI view's `body`) and subscripts; and Kotlin `init` blocks, secondary constructors and property accessors. These files get function simplification, file organization, shared logic and comments, and every request names the language. [Custom questions](custom-questions.md) read them as they read the other languages: a `function` or `comment` question asks about their functions and comments, and a `file` or `hunk` question about any of their files. What they do not get:

- Hardcoded values and the security rules: those need a language's own sites, sources and sinks.
- Test rules, and custom `test` questions: a test file is found by path and reported as not judged yet. Besides `test/`, `tests/`, `test_*` and `*_test.*`, that is a class named `…Test`, `…Tests` or `…IT` (Kotlin, Swift, Scala), a C or C++ file whose name starts with `test` or ends in `-test` (`test.c`, `testheap.c`, `linenoise-test.c`) and `*_unittest.cc`, a Kotlin source set such as `androidTest` or `commonTest`, a Swift test target such as `VaporTests`, busted's `spec/` and `*_spec.lua`, Dart's `integration_test/`, and `*.bats`. Kotest's and ScalaTest's `…Spec` and `…Suite` classes are found by their test directory: outside one, production code takes those names.
- Callers from other files: no imports are resolved, so an outline shows the calls between its members but not which files use them, and a function's callees are found by name among the files of its own language.
- Idioms left out of copies: Go's error checks and Java's field initializers do not count as copies, and no such idiom is known for these languages yet. Their copies pair only within one language, or between C and C++, and take only the places of a run's 64 that the other languages leave. A Bash script runs on its own, so its copies pair with another script's only when one reads the other in (`source`, `.`) or both read in the same script of the project.

Copied dependencies (`Pods`, `Carthage`, `third_party`, `third-party`, `thirdparty`, `3rdparty`, `deps` and `external` directories) are skipped as vendored, and Flutter's platform runners (`windows/runner`, `linux/runner`, `macos/Runner`, `ios/Runner`) and Dart's generated files (`.g.dart`, `.freezed.dart`, `.gr.dart`, `.mocks.dart`, protoc's `.pb.dart`) as generated, as are files headed by build_runner's, SwiftGen's, flex's or Bison's generated-code marker. A library copied as one file beside the project's own, such as `uthash.h`, is still judged.

`.sh` and `.bash` files are read as Bash; scripts of other shells (`.zsh`, `.fish`, `.ps1`, `.bat` and others) stay listed as operational scripts. A `.h` header is read as C++ when its code is only C++ (`std::`, or a line opening a namespace, a template, a class or an access section), and as C otherwise. Elixir's `@doc` and `@moduledoc` are strings, not comments, so the comments rule does not read them.

The grammars miss some valid code, which is left out and listed as code the parser could not read, the rest of its file judged: Swift's `x as? T ?? fallback`; Kotlin 2's explicit backing fields, a Ktor `get("…") { }` right after a local `val`, and an assignment to a property named like a soft keyword (`inline = true`); Bash's base-prefixed arithmetic (`$((16#$hex))`), substring offsets (`${s:$i:1}`), a regex with groups after `=~`, and a lone `[` in a pattern expansion; Scala 3's `given … with` before an indented body; C's specifier and statement macros (`static JSON_INLINE int f`, `CHECK_AND_RETURN(p)` with no `;`) and a foreach macro before a block; and C++'s brace default arguments (`std::optional<T> x = {}`).
