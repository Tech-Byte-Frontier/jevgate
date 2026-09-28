# Supported languages and frameworks

Each language is supported or in preview. The ten languages with analyzers of their own are supported. The nine the [generic tier](#generic-support) reads are in preview until two projects JevGate was never tuned on meet the maturity bar: a rule and level right at least 80% of the time, over at least 20 labeled findings. A preview language's findings are reported like any other. [Support levels](#support-levels) lists each language's level and how often its findings were right.

✅ judged · ➖ not applicable

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
| C, C++, Kotlin, Swift, Bash, Dart, Scala, Elixir, Lua ([generic support](#generic-support), preview) | `.c` `.h` · `.cpp` `.cc` `.cxx` `.hpp` `.hh` `.hxx` · `.kt` `.kts` · `.swift` · `.sh` `.bash` · `.dart` · `.scala` · `.ex` `.exs` · `.lua` | ✅ function simplification, file organization, shared logic | ➖ test files are found by path and not judged yet | ➖ | ✅ comments |
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
| C, C++, Kotlin, Swift, Bash, Dart, Scala, Elixir, Lua | preview | none yet | not measured | not measured |

A finding is right when a person reading the code agrees with it; a debatable one counts as not right. The counts are 0.25.0's reviews and considers on the 25 projects JevGate was never tuned on (11 held out, 14 fresh), each labeled by hand from the code, for the four rules every language gets: function simplification, file organization, shared logic and comments. A percentage is shown from 20 labels on. Those projects hold no C#, Ruby or Bend 2 finding of these rules. A preview language is measured the same way, on projects never used for tuning, before it can become supported.

## Generic support

C, C++, Kotlin, Swift, Bash, Dart, Scala, Elixir and Lua are in preview. They are read through one tree-sitter tag query per language, written in the captures GitHub's code navigation uses (`@definition.function`, `@definition.class`, `@reference.call`): it finds functions, methods, types and the calls each makes, and a table per language names the nodes that hold statements, nest control flow and hold literals. These files get function simplification, file organization, shared logic and comments, and every request names the language. What they do not get:

- Hardcoded values and the security rules: those need a language's own sites, sources and sinks.
- Test rules: a test file is found by path and reported as not judged yet. Besides `test/`, `tests/`, `test_*` and `*_test.*`, that is a class named `…Test`, `…Tests`, `…Spec` or `…IT` (Kotlin, Swift, Scala; `…Suite` in Scala), a Kotlin source set such as `androidTest` or `commonTest`, a Swift test target such as `VaporTests`, busted's `spec/` and `*_spec.lua`, Dart's `integration_test/`, `*_unittest.cc` and `*.bats`.
- Callers from other files: no imports are resolved, so an outline shows the calls between its members but not which files use them, and a function's callees are found by name among the files of its own language.
- Idioms left out of copies: Go's error checks and Java's field initializers do not count as copies, and no such idiom is known for these languages yet. Their copies pair only within one language, or between C and C++, and take only the places of a run's 64 that the other languages leave.

`.sh` and `.bash` files are read as Bash; scripts of other shells (`.zsh`, `.fish`, `.ps1`, `.bat` and others) stay listed as operational scripts. A C++ header named `.h` is read as C, and skipped if that fails. Elixir's `@doc` and `@moduledoc` are strings, not comments, so the comments rule does not read them.
