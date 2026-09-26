# Supported languages and frameworks

✅ judged · ➖ not applicable

| Language or file | Extensions | Maintainability | Tests | Security | Documentation |
|---|---|:---:|:---:|:---:|:---:|
| Rust | `.rs` | ✅ | ✅ `#[test]`, `#[cfg(test)]` | ✅ | ✅ comments |
| Python | `.py` | ✅ | ✅ pytest, unittest | ✅ | ✅ comments |
| JavaScript | `.js` `.jsx` `.mjs` `.cjs` | ✅ | ✅ `describe`/`it`/`test` | ✅ | ✅ comments |
| TypeScript | `.ts` `.tsx` `.mts` `.cts` | ✅ | ✅ `describe`/`it`/`test` | ✅ | ✅ comments |
| Go | `.go` | ✅ | ✅ `Test…(t *testing.T)` | ✅ | ✅ comments |
| C# | `.cs` | ✅ | ✅ xUnit, NUnit, MSTest | ✅ | ✅ comments |
| Ruby | `.rb` | ✅ | ✅ RSpec, Minitest, Rails `test "…" do` | ✅ no Ruby framework handlers yet | ✅ comments |
| PHP | `.php` `.phtml` | ✅ | ✅ PHPUnit `…TestCase` classes, Pest `test`/`it` | ✅ | ✅ comments |
| Java | `.java` | ✅ | ✅ JUnit 4 and 5, TestNG: `@Test`, `@ParameterizedTest`, `@Nested`, JUnit 3 `TestCase` | ✅ | ✅ comments |
| Astro, Vue, Svelte | `.astro` `.vue` `.svelte` | ✅ scripts only | ➖ | ✅ scripts only | ✅ script comments |
| SQL (PostgreSQL, Supabase) | `.sql` | ➖ | ➖ | ✅ access control | ➖ |
| GitHub Actions | `.github/workflows/*.yml` | ➖ | ➖ | ✅ workflows | ➖ |
| Markdown, MDX | `.md` `.mdx` at the root, in `docs/` or `doc/`, READMEs and CONTRIBUTING files; agent instruction files | ➖ | ➖ | ➖ | ✅ |
| reStructuredText, AsciiDoc | `.rst` `.adoc` `.asciidoc`, in the same places | ➖ | ➖ | ➖ | ✅ |

| Framework or platform | What JevGate understands |
|---|---|
| Hono, Express, Fastify, Koa | Route handlers written inline (`app.post('/pages', async (c) => …)`); error handlers (`app.onError`, `setErrorHandler`, four-parameter Express middleware) |
| NestJS | Exception filters (`@Catch`) |
| Next.js (App Router and Pages Router) | Route handlers (`app/**/route.ts`), Server Actions (`'use server'` files and functions), `pages/api` routes, middleware, client components, error boundaries and pages are named to Jev with who calls them and where they run, so a Server Action's arguments read as client input and a client component's requests as the user's own; `dangerouslySetInnerHTML`, redirects to client-chosen URLs, raw Prisma and Drizzle queries (`$queryRawUnsafe`, `sql.raw`) as opposed to their binding tagged templates, `NEXT_PUBLIC_` secrets, and `next.config` headers |
| SvelteKit | Server load functions and form actions (`+page.server.js`, `export const actions = {…}`), endpoints (`+server.js`) and server hooks are named to Jev with who calls them, so their request, form data, URL and cookies read as client input, and `cookies.set` is read with its secure defaults |
| Flask, FastAPI | Error handlers (`@app.errorhandler`, `@app.exception_handler`) |
| Django, Django REST framework | Views and viewsets with the URL routes that reach them, the templates they render with `\|safe` or autoescaping off, and the module constants they use; settings modules, with secret literals redacted, the settings modules that import and override them, and the files that select them (`DJANGO_SETTINGS_MODULE`); management commands as run by hand; `handler500`-style error views, middleware `process_exception` and `EXCEPTION_HANDLER` as error handlers |
| PHP pages, Slim, Laravel | A file's top-level code is judged like a function, since a page script reads the request and writes the response; route closures (`$app->get('/users', function …)`, `Route::post(…)`) and configuration closures (`return function (App $app) {…}`); error handlers (`set_exception_handler`, subclasses of Slim's `ErrorHandler` and Laravel's `ExceptionHandler`) |
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
| Migrations | Directories named `migrations`, Rails' `db/migrate` and timestamped scripts under `db/`, and Alembic's `alembic/versions` are skipped as migrations; SQL migrations are still read for access control |

Other files, such as Kotlin, are listed as skipped with the reason and never fail the gate.
