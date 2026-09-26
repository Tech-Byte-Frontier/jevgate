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
   C# tests are whole classes marked `[TestFixture]` or `[TestClass]` or
   holding a method marked `[Fact]`, `[Theory]`, `[Test]`, `[TestCase]`,
   `[TestCaseSource]`, `[TestMethod]` or `[DataTestMethod]`, and every C# file
   of a test project directory named like `Shop.Tests`; `.Designer.cs`,
   `.g.cs` and `.g.i.cs` files are generated.
   Ruby tests are RSpec groups and examples written as statements
   (`describe`, `context`, `it`, `specify`, `its`, titled by a string or not
   at all, so a Rakefile's `test(:unit) do` is not one) and classes whose
   superclass ends in `Test`, `TestCase` or `Spec` (`Minitest::Test`,
   `ActiveSupport::TestCase`) with `test_*` methods or `test "…" do` blocks;
   `*_spec.rb` files and Ruby files under `spec/` are test files.
   PHP tests are the `test…`, `@test` or `#[Test]` methods of a class
   extending a `…TestCase`, and Pest `test(…)`/`it(…)` calls; `…Test.php`
   files are test paths.
   A Java class is a test class, whole, when it holds a method annotated
   `@Test`, `@ParameterizedTest` or another test annotation (a composed one
   whose name ends in `Test` included), a JUnit lifecycle method such as
   `@BeforeEach`, a nested test class, or extends JUnit 3's `TestCase`;
   `…Test`, `…Tests`, `…TestCase` and `…IT` files are test paths. A test
   that sends a request to a literal path (MockMvc's `get("/owners/{id}")`,
   RestTemplate's `RequestEntity.get(…)`) calls the Spring controller method
   whose `@GetMapping`, `@PostMapping` or `@RequestMapping` route serves it,
   under its class's prefix, by full name, the most literal route winning:
   controllers share method names such as `initCreationForm`, and a request
   names no method, so such tests had no code under test.
   Astro, Vue and
   Svelte files are parsed as their scripts: Astro frontmatter and `<script>`
   contents, with every other byte a space, so lines stay the file's.
   A file whose parse holds syntax errors is not judged, unless they are few
   and small (at most three regions, an eighth of the source in all), since
   grammars miss some valid code: tree-sitter-typescript reads a call
   signature starting with `<T>` on the line after another as its
   continuation, which left four of zustand's source files unjudged. The
   definitions that hold an error are then left out. Generator templates
   (under `templates/`, or holding ERB tags or `//#if` conditions) keep the
   strict rule, since their placeholders are not the language's syntax.
2. **Local analysis** (`src/analysis/`). Units with signatures, calls, references
   and control-flow nesting; callbacks registered through calls, including
   module-level route handlers named by their registration
   (`app.post('/pages')`); member groups by average linkage; callers that
   import the file (application code only, since tests calling a group do not
   make it a dependency); for file organization, each member's line count and
   the file's, and for a test file, its cases with their enclosing
   `describe`, class or module and the functions under test they call, grouped
   by shared suite, subject or helper. A subject that one type in scope owns
   is named with it (`StringUtil::isBlank`); a Java test file's top-level
   class is not a suite, since every case would share it, while `@Nested`
   classes are. A pair of similar tests is about a function they share that
   is neither a camelCase getter or setter nor called by most of the file's
   tests (a fixture or client every test uses), and that the fewest tests
   call: the first shared name made `setBirthDate` the subject of validator
   tests and grouped unrelated pairs under a shared `create_user` fixture.
   Test files get this outline without
   `--include-tests`; their finding is at most a consider. Who calls a group is
   evidence only: gating a split on callers of its own hid large files whose
   single caller is the rest of the program; Type-2 clone candidates grouped by overlapping copies, only
   within one package or packages linked by a local dependency (copies in side
   by side templates or example apps are separate projects); test cases with
   their subjects and similar pairs. A PHP file's top-level statements
   outside functions and classes (and its `<?= … ?>` echoes) are one more
   unit, `top-level code`: a page script reads the request and writes the
   response there, so every security rule judges it like a function, while
   other languages' top-level statements are judged for unsafe settings
   only. Closures registered through calls (`$app->get('/users', …)`,
   `Route::post(…)`) or returned by a configuration file
   (`return function (App $app) {…}`) are functions of their own.
   A Java constructor's statements that store
   a parameter, another object's field or a literal in a field break a copy:
   two constructors filling different fields matched as copies and stayed
   undecided. So does a Java setter given one literal
   (`owner.setCity("Madison")`): a fixture built in a helper and an owner
   built inside a test matched as copies whose only differences were the
   values. Java `equals` and `hashCode` overrides offer no copies or
   values, and neither does an initial capacity (`new ArrayList<>(4)`) or the
   number a method returns whole (`int cost() { return 7; }`), which the
   method's name already names. Neither does a function or type marked
   deprecated (a `@deprecated` tag, annotation or decorator,
   `#[deprecated]`, `[Obsolete]`, or a `Deprecated:` comment above it): it
   goes with the next major version, and flysystem's deprecated phpseclib 2
   adapter was paired with the adapter replacing it in seven reviews.
3. **First pass** (`src/units/`). One dispatch of every unit request. Functions,
   for simplification, hardcoded values and security, are packed eight per
   request within runs of functions, a run ending after a function whose name
   hashes to one of four values, so a function added, removed or resized
   re-asks only its run: packed in file order, one added function re-sent
   every later pack of the file (5 of 5 simplification requests of
   `compose.rs`, against 1 now). Runs add requests (25% to 106%, and 33% to
   300% for instruction files), and every request is billed about 280 input
   tokens beyond its size, so a full first pass bills 4% to 7% more input
   (functions 6% to 13%, values 4% to 7%, security 3% to 5%). A function
   removed re-asks 24% to 47% fewer tokens, so the runs pay back after 19 to
   30 edits. One in eight names ending runs cost under half as much extra on
   a full pass but saved a half to two thirds as much per edit, and left
   whole files in one run (`clones.rs`, `literals.rs`); one in four
   overtakes it after 31 to 40 edits.
   Tests are sent one per request, because unrelated
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
   An outline whose recheck stays undecided is asked, in a request of its
   own, what kind of file it is: one algorithm, type, resource, component,
   set of definitions, helpers or coordination serves one feature; the same
   kind of code written out per feature, or several unrelated features,
   serves several. Kinds that serve one feature at 0.80 clear it, and the
   others at 0.80 raise a consider. Weighing a split stayed near a third per
   level on such files, while naming the kind was decisive; asked beside the
   recheck, the kind moved the recheck's own answers. A file too long to send
   whole gets no recheck, so its undecided first answer is asked the kind
   from the outline alone; large Java classes and their test files otherwise
   stayed uncertain.
   A test left undecided on whether it re-implements the code or checks only
   its mocks is asked again with the bodies of the functions it calls and its
   file's imports, mocks and setup hooks (a part too long is left out, never
   cut); each answer replaces the first unless only the first is decisive.
   A Ruby test is sent with the groups it is declared in, since an RSpec
   example reads as a sentence continuing them and the outer group often names
   the class under test. Its recheck shows, instead of every hook of the
   file, what runs for it: its groups' `before`, `around` and `setup` hooks,
   `let!`, and the `let` and `subject` definitions it reads (directly or
   through another), then the test helpers it and those hooks call, from its
   own file or the nearest support file (one without test cases sharing a
   directory with it; an RSpec group's methods stay in its file). The note
   says a value built there is input to the code under test unless a mock
   returns it: with the file's setup described as mocks, factory definitions
   and `mock_app` routes read as mocks, and a third of such tests stayed
   undecided. Ruby test pairs carry their groups and hooks when these differ,
   and are also asked whether each test checks something the other does not
   (another method, matcher, attribute, option or code path); "one adds
   nothing" is a review only when that is ruled out at 0.80. Pairs in other
   languages are asked it after the fact, only when they would be a review
   and do not read the same apart from their names: tests of two overloads
   (`writeTo(Path)` and `writeTo(File)`) and of two public methods were
   reviews, and six of ten labeled reviews were wrong. Copied RSpec
   examples for an alias and its original (`each` and `each_pair`) or for two
   predicates of one record were otherwise reviews.
   A controller method a test reaches through a request carries that route.
   The first pass names a literal worked out by hand, even with the
   arithmetic in a comment, as not re-implementing the code. Its "checks only
   its mocks" question names tests without any stub (a setter read back, a
   round trip, a benchmark), and tests checking which stub the code chose or
   the view and status a handler chose for stubbed data, as not hollow: those
   stayed near a third, as if every assertion were about the mocks.
   A pair of tests whose overlap spreads over the three levels is asked
   again with the body of the function both call: whether it throws before
   the rest of a test runs is in that body.
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
   cleared real leaks. The trace also lists the errors that the functions it
   calls create, two calls deep in its own file or files it imports, with
   their messages, and then asks the exception check about whose text a
   response carries rather than who raised it: FastAPI handlers returning
   `str(exc)` for the `LookupError` their service raised with the program's
   own text ("Imóvel não encontrado") were twelve reviews in one project,
   since the handler "did not raise it itself"; with the service's raise in
   view, ten became notes or considers, and tools that return the text of
   every exception they catch became reviews. The Choice over the messages
   it creates is told the same, since two of those considers remained
   because passing on the service's error read as "another error's text".
   Each registered error handler (`.onError(…)`,
   `.setErrorHandler(…)`, Express four-parameter `.use(…)` middleware, Flask
   and FastAPI decorators, NestJS `@Catch` filters, axum `IntoResponse` and
   actix-web `ResponseError` for an error type, Rocket catchers, ASP.NET Core
   `UseExceptionHandler` lambdas, exception filters, `IExceptionHandler` and
   middleware classes whose `Invoke` catches what the pipeline throws; a path
   given to `UseExceptionHandler` re-executes a page judged as its own code)
   is asked once
   whether it sends clients more than the program's own messages and codes,
   with the program's `…Error` classes (Rust enums with their `#[error]`
   messages) and the functions of its file that it calls. A registration
   inside a comment or string literal registers nothing. An injection trace
   also gets the definitions of enums its sites name (`ConfigKey.aiTag`), so
   a fixed choice does not read as a parameter.
   Questions about C# files carry ASP.NET Core's names for what they ask
   (`FromSqlInterpolated` binds, `ServerCertificateCustomValidationCallback`
   returning true skips certificates, `ValidateIssuer` checks claims, not
   certificates), and C# traces ask three more weak-setting checks (developer
   exception pages outside development, token signature or lifetime checks
   turned off, signing keys written in the code) and one injection check
   (types named by input or chosen by deserialized data). An unsafe-settings
   trace in C# also gets the `const` and `static readonly` fields the code
   names, often declared in another file, so a key written in the code does
   not read as configuration. Other languages keep their wording: the
   additions were measured on ASP.NET Core projects only. A broad weak-setting
   answer that none of the specific checks leans toward names no setting to
   change and is at most a note: on an action marked `[AllowAnonymous]` on
   purpose it was 0.85 while every check stayed at 0.30 or less.
   In a package that depends on `next`, a file's path names its role
   (`app/**/route.ts`, `pages/api/**`, `middleware.ts` or `proxy.ts`,
   `app/**/error.tsx`, pages and layouts, `next.config.*`), and in any
   package a leading `'use server'` or `'use client'` directive, or a
   function body that starts with `'use server'`, marks Server Actions or a
   client component. The role goes into the file state of security and
   hardcoded-value questions as `framework`, and every such question's note
   points to it: stated only in the state, a client component's role did not
   clear its browser requests. Questions about how code reads (splits,
   outlines, tests) get no role: there it moved split answers without
   informing them. With it, a
   Server Action's parameters read as client input (an injection that was a
   consider on its parameters became a review) and an error boundary as the
   browser's own page. The injection trace asks one more literal check,
   whether a redirect target from a variable is checked (CWE-601): without
   it, `redirect(next)` and `NextResponse.redirect(returnTo)` were notes about
   URLs "it requests". Only targets a request carries count: a link
   shortener's redirect to the destination its owner saved was a review.
   The SQL check names tagged templates that bind (`sql`, `$queryRaw`) as
   handled and `$queryRawUnsafe` and `sql.raw` as not, the
   markup check names `dangerouslySetInnerHTML` (also a site), and the unsafe
   settings ask whether a secret comes from a variable the build puts into
   browser code (`NEXT_PUBLIC_`). A `next.config` file's setup is every
   top-level statement that holds an object, with its innermost objects as
   sites, since `headers()` settings call nothing.
   Django code (Python that imports Django or Django REST framework, and
   settings modules) is asked Django's names in the checks that have them
   and three more; other code keeps the common ones, so its cached answers
   stay valid. They name raw SQL (`raw`, `extra`, `RawSQL`), `mark_safe` and
   templates that write values with `|safe`, `redirect()` to route names or
   the program's own paths, the storage API, Django's password hashers and
   validation errors, and add deserializers of request data (`pickle`,
   `yaml.load`), debug mode, `csrf_exempt` and literal secret keys, and
   `request.META` or the settings sent to a client. A view is sent with the
   URL routes that reach it (a `\d+` parameter holds digits), the templates
   it renders that write values unescaped, and the module constants it
   names; a management command is marked as run by hand, since its options
   came back as another party's. A settings module is one unit whose
   statements are its settings, with secret literals redacted to their
   length (a dotted path such as a secret-key getter is not a secret). It is
   sent with the lines that select it (`DJANGO_SETTINGS_MODULE` in a
   Dockerfile, CI or `manage.py`) and with the settings modules that import
   it, directly or through others, each with its own selections and its
   assignments of the settings it sets again; a `setdefault` in
   `manage.py` or `wsgi.py` is marked as only a default, since shown bare it
   read as the deployed choice and made shared CORS settings that production
   sets again a review. Its sites put security settings first, then
   assignments into a setting (`OPTIONS["ssl_cert_reqs"] = None`), which
   URLs joined with paths had crowded out. Presence alone found a
   development `DEBUG = True` or a signed webhook's `csrf_exempt` weak as
   surely as a deployed one, so a Django weak setting must be named by a
   specific check at the review threshold to be a consider or review;
   otherwise it is a note, and settings only development or tests run with
   are two levels lower. The secret, cookie and CORS checks ask about the
   deployed site, since asked about the module alone they flagged base
   settings that production sets again. The markup Choice asks a Django view
   what it sends back, since views that only redirect or render an escaping
   template split on the markup check. Since nearly every Django view places
   request values somewhere, an injection note that no check found (values
   from another party, every check undecided or clear) gets the settle
   Choices too: on django.nV, redirects to the view's own paths with ids in
   them left seven such notes, which the redirect-target Choice cleared. The
   CSRF check names forms for visitors who are not signed in, such as a
   password reset request, as not acting for a user. Django error views
   (`handler500 = …`), middleware `process_exception` and Django REST
   framework's `EXCEPTION_HANDLER` are error handlers, asked with the
   framework's errors written for the user named as acceptable.
   A security unit still uncertain after its trace and recheck is asked, per
   undecided check and in a request of its own, one literal Choice that can
   only clear that check: where its URLs come from (a host of the program's
   own at 0.80 clears an undecided URL check; a configured host sent another
   URL to fetch does not) and where the code runs (only in the user's
   browser clears it); where its redirect targets come from (written in the
   code, returned by its own server or what callers pass, checked, or no
   redirect); how its markup is rendered (escaped by JSX or a template, or
   shown as text; PHP units are asked what they join instead, see below); which sites may send credentialed requests (none, listed
   origins, or any origin without credentials); what its logs write (only
   messages, ids and caught errors); or where its text goes (anywhere but a
   remote client at 0.80 clears undecided error details). Offered beside
   "a whole URL handed to it", the browser lost for a client component's
   fetch helper, which is why where code runs is its own Choice. A consider
   or note that rests on an undecided error-detail or URL check gets the
   destination or runs-in question, since it claims the text likely reaches
   a client or the request leaves a server. On three Next.js apps these
   Choices took the uncertain files from 56 to 34, most of them client
   components that navigate to fixed paths or render values as attributes.
   A Choice about what a query builder joins into SQL (its own clauses,
   numbers, or values handed to it) was tried for considers on parameters
   and dropped: it cleared a sort column taken from the request as readily
   as clauses with placeholders. The same question about paths cleared real
   traversals, reading names stored in an index as the program's own, so path
   checks stay undecided until callers show more. The SQL check counts
   identifiers quoted by doubling embedded quotes as handled (identifiers
   cannot be bound), and the URL check excludes requests a web page sends from
   the user's browser; on fresh repositories both had flagged such code, while
   the SQL and SSRF advisory functions kept their answers. Code whose source
   names a deserializer that can build any object (Python's `pickle`,
   `marshal`, `shelve`, `jsonpickle` or `yaml.load`; Ruby's `Marshal.load`
   or `YAML.load`; Java's `ObjectInputStream`, `XMLDecoder`, XStream or
   SnakeYAML; node-serialize) is asked about loading data with it in the
   presence question, and its trace asks that language's deserialize check,
   as Django views and PHP pages naming `unserialize` are: a Flask route
   passing `pickle.loads(request.get_data())` was asked only about query,
   command, code and markup text, and was clear. Code that parses XML with a
   parser able to resolve external entities (it names lxml, SAX, pulldom,
   DocumentBuilderFactory, XmlDocument, SimpleXML, libxmljs or Nokogiri, or
   its file imports one and it calls a parse method) is asked the same way
   about XML with external entities (CWE-611): pygoat's lab calling
   `make_parser()` with external entities turned on, and a Spring controller
   parsing its body with a default DocumentBuilderFactory, were clear. Only
   the requests of such functions change.
   PHP units read the presence questions and checks in PHP's own terms
   (`src/units/questions/php.rs`), naming its functions (`echo`,
   `shell_exec` and backticks, `mysqli_real_escape_string`, `password_hash`,
   `CURLOPT_SSL_VERIFYPEER`); every other language keeps the general wording,
   so its requests and cached answers are unchanged. The PHP SQL check counts
   driver escaping inside quotes and numbers as handled: asked only about
   binding, DVWA's escaped and quoted guestbook inserts were reviews. Text a
   page writes with `echo` is its response, not a log; a page that only calls
   `generateSessionToken()` or a session helper is not judged for what that
   helper does; the message of an exception class the program defines,
   caught by name, is its own text (four BookStack upload controllers
   returning `FileUploadException` messages were error-detail reviews); and
   only variables placed into a query or path count for the origin, not an
   uploaded file's contents. `unserialize` and uploaded file names are PHP
   checks, asked only of source that names `unserialize` or an upload; a page
   that only showed an upload form stayed near 0.4 on whether it saves
   uploads.
   PHP units have three settle Choices of their own, asked whenever their
   check is not clear, even when it found a concern, since each can clear
   it: what the unit joins into HTML unescaped (request values, stored
   records and parameters holding text keep the check; escaped values and
   numbers, text the program produces such as errors and command output,
   HTML other code builds such as a page body an included file sets, and
   element data clear it), what its command lines hold (values each checked
   against a strict format, such as octets that pass `is_numeric`, clear it;
   values with some characters stripped do not), and where its paths come
   from (constants and a file name a `switch` picks clear it; names stored
   in a database or file are their own option). A page that reads a request
   also joins ids converted with `intval` and database errors, and the
   markup check found those at 0.9 while the origin question answered for
   the request. A markup check that found a variable which the Choice names
   as a request value or stored record is a review whatever the origin
   question said: an access log joining user names from the database stayed
   uncertain with the origin split. A page script's consider or note names
   values whose origin it does not show, not parameters. On DVWA the PHP
   Choices left 13 of 329 injection units undecided, from 46.
   Agent instruction files (`src/docs/`) are found by name even when hidden
   or ignored, including Kiro steering files, Junie guidelines and rules,
   and Roo Code rules, each loaded by its harness's documented rules (a Kiro
   `inclusion`, a Roo Code mode folder, Junie's precedence of its own
   `AGENTS.md`). Each file's heading sections, or the top-level blocks of a
   long section, are sent packed, within runs of sections that end after a
   heading hashing to one of four values (a long section's blocks share its
   heading and stay together; brstocks `CLAUDE.md` went from 1 request to
   4, billing 18% more input on a full run and 57% less when one section
   is removed), beside the nearest manifests (with the
   runtime versions they require: `engines`, `packageManager`,
   `requires-python`, `rust-version`), the configured linters and the
   directories. A section whose signals stay undecided is asked, alone,
   which kind of section it is (instructions, a description, a command
   list, generic advice or a record): its own kind at 0.80 raises the
   undecided signal, that kind at 0.20 or less clears it, and instructions
   clear an undecided "restates the repository". On vercel/ai, API tables and
   import maps stayed near the middle on "only describes", while naming the
   kind was decisive. The linter question asks whether a section is only
   style the listed tools check with their usual settings; asked whether it
   asked for such style, a list of `Do Not` rules with one import rule, or
   file naming no configured rule checks, stayed undecided.
   Code decides which harness loads each file and
   when, from its documented discovery rules. It also records loading facts:
   copies, unresolved imports, and files a harness skips or truncates. These
   are reported, never judged.
   Project documentation is Markdown, MDX, reStructuredText or AsciiDoc,
   read as Markdown with the file's own lines (`src/docs/format.rs`): MDX
   drops imports, exports, comments and component markup but keeps the prose
   components carry, reStructuredText titles become headings by the order of
   their adornment styles, AsciiDoc titles by their `=` level, comments and
   attribute entries are dropped, and code directives, literal and listing
   blocks are fenced with their language. A document of 300 or more lines is
   sent as its headings only, with `#` marks for nesting. A split finding is
   then located with one Choice among its top-level parts; a split that stays
   undecided is asked which kind of document it is (a guide, a reference, a
   migration guide, an introduction, or a collection of unrelated subjects).
   The kinds that serve one subject at 0.80 clear it and a collection at
   0.80 raises a consider: a quickstart, a migration guide and a package
   README each stayed near a third per level on the split.
   Per-section questions on
   project docs were dropped: on a labeled sample they found almost nothing,
   and they cost about five times more than an outline.
   Staleness and duplication candidates come from code. Staleness candidates
   are paths and scripts a section names that the repository lacks, with what
   Git shows about each. A span written with a code role (`:attr:`,
   `:class:`) is not a path, nor is a link that climbs above the repository
   (a README badge's `../../actions/...`); a path the ignore files cover
   (a bare name also as a directory, so `backend/app/frontend/` covers a
   build output), or one the section writes out for a code block (its
   `filename=`, or the one path the paragraph before it names), is the
   reader's own; a script where no command starts, such as "make sure", is
   not one, and dependencies count as scripts, since `pnpm tsx` runs one. A
   missing name whose one tracked namesake sits under the document's
   directory, such as `.tsx` for `.ts`, is named beside it. A check that
   stays undecided is asked, apart, what the section treats the names as (a
   current part of the repository, the reader's own project, an example,
   not a file at all, or something removed); the repository at 0.20 or less
   clears it. A protocol method (`tools/call`), skill-relative example
   paths and a migration guide's `pnpm drizzle-kit` stayed near 0.25 to
   0.50 on the check and were decisive as a kind.
   Duplication candidates are section pairs where 30%
   of the smaller section's three-word sequences recur in the other, at most
   three per pair of documents. Sequences come from the prose; a section
   with too little prose is compared on its prose, commands and settings,
   and program code is never compared: pairing on code sent hundreds of
   vercel/ai pages that shared a `streamText` call and nothing else. Project
   documents of separate packages are not paired, since each package's
   README is read alone. A section that pairs with two or more others heads
   a family, and its members are asked against it alone. A document whose release is tagged, or whose
   named paths were deleted, is asked from its headings whether it is a plan;
   a plan with those facts is one finding, and its own candidates are not
   asked. The section check and the pair questions (does A state everything
   B states, and the reverse, as Scores whose middle "mostly" is acceptable;
   do they disagree, a Score whose middle "only in detail" is acceptable; are
   they about one subject; is one a translation of the other?) are
   follow-ups for the other documents, sent with each document's title. A
   translation clears the repetition answers but not a disagreement.
   Different subjects settle what stays undecided, never a decided answer.
   A pair still undecided is asked, apart, how the two sections relate
   (one repeats the other, they overlap, they are written alike for
   different subjects, they contradict each other, or they describe
   different things); a repetition or a contradiction at 0.20 or less
   clears that check. Weighing coverage stayed near the middle for one step
   of two quickstarts or one option of guide and reference, while naming
   the relation was decisive: 18 of 654 pairs undecided on vercel/ai became
   1. The repetition findings that share a section are one finding at the
   section most of them name, listing the others. A Score on how much two
   sections overlap, asked of every pair, stayed on its middle level for
   almost every pair, so it is not asked.
   Code comments (`documentation/comments`) are collected from the parse
   tree of application code, outside tests: runs of line comments on
   consecutive lines are one comment, and Python docstrings are comments of
   the definition or module they open. License headers, tool directives
   (`eslint-disable`, `# noqa`, `//go:`), JSDoc type annotations, shebangs
   and comments without letters are left out. Each comment is sent with the
   code it is about: the declaration it documents (its signature when
   longer than 40 lines), the lines below it up to a blank line, another
   comment or the end of its block, the line it ends, or for a file's own
   documentation the signatures of its definitions; with where it sits and
   the signature of the definition it sits in. Comments are packed eight per
   request within runs of definitions, a run ending after a definition
   whose name hashes to one of four values, so a comment added or removed
   re-asks only its run: packed in file order, one added comment re-sent
   every later pack of the file (6 of 6 requests of `compose.rs`, against 1
   now), and a pack per definition doubled the requests. They are asked
   whether they only repeat that code (a Score whose
   middle level holds headings over a group of lines), whether sentences
   could go without losing anything (only comments of 20 or more words),
   whether they describe an edit instead of the code as it is, and whether
   they are code turned off (only comments whose lines read like
   statements). Asked of every comment, those two stayed near 0.5 on
   two-word trailing comments and on docstrings holding usage examples;
   asked only where they apply, psf/requests' undecided comments went from
   78 of 519 to 63 with the code check and to 35 with the wordiness one. The first
   wording of the wordiness question ("could it say the same in far fewer
   words?") called every multi-line JSDoc block that explains a rounding
   rule or a matching strategy wordy; asked whether sentences add nothing,
   with parameter entries named by whether the signature writes their
   types, the Sphinx `:param` entries of untyped functions stopped reading
   as filler while `numerator: The numerator value.` still does. A flag's
   meaning (`"-x",  # Extract audio`), a unit (`// 16px` beside
   `1rem`) and a category heading (`// Fixed` above fixed costs) are
   named as acceptable. A comment left undecided is asked again with the
   whole definition it sits in, then, alone, what kind of comment it is (a
   reason, caveat, reference, usage, summary or heading, against repeating
   the code, narrating steps, an edit or code turned off).
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
   pull request code with secrets. A job left undecided is asked, apart,
   which expression holds outside text and what code it runs (the base
   branch's, the pull request's, or none), as Choices that can only clear:
   a release job's tag names and a job uploading a pull request's coverage
   report stayed between 0.2 and 0.6, and 7 of 14 such jobs were settled.
5. **Composition** (`src/units/compose.rs`). Pure. On a Score whose top level is
   the actionable concern: review at 0.80 on the top level, consider at 0.80 on
   middle-or-top, clear when the top level is ruled out at 0.80, otherwise
   uncertain. Where the middle level says the code is fine as it is, a consider
   also needs the top level at 0.50; middle mass alone is an optional note. For hardcoded values and security, an answer still
   undecided after its follow-up is a note when it leans toward the concern
   (0.50, the leading probability) and stays uncertain otherwise: undecided
   answers leaning away were almost all acceptable code, and leaning toward
   held both real positives of the labeled set. So is an error-detail
   answer leaning toward a client when the own-messages check finds another
   error's text in an error message: as a consider, 1 of 28 such findings
   outside example code was right, since a central handler replaced the
   text with a generic message, the error was one written for users, or no
   remote client read it. Instruction sections are
   cleanups, so their findings are at most a consider. Comments are cleanups
   too: at most a consider, and documentation that only repeats the
   declaration it documents is at most a note, since documentation tools and docstring
   linters expect a summary even when it says what the name says. A
   definition's comments that reach a consider are one finding, listing
   each comment with what is wrong with it, and when they span fewer than
   three lines in all they are a note. Undecided answers do not
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
   cases are one level lower, and copies in their fixtures, helpers and setup
   at most a consider. Copies of three lines or fewer are at most a
   consider: in Java such a copy was as often an idiom, a pooled builder
   borrowed and released around one call, as a missing helper. A test that
   checks several unrelated behaviors is at most a note: on labeled tests,
   tables of inputs and browser journeys rated as high as tests that really
   mix behaviors. Overlapping test pairs of one subject become one consider
   for three or more tests only when the pairs connect them: two pairs that
   share no test stay two pairs (a pair of redirect tests and a pair of deny
   tests of `get` are not four overlapping tests). A review always carries a
   finding. A file-organization finding on a file of fewer than 250 lines is
   a note: of 32 such findings labeled by hand on 25 projects, 3 were right,
   and splitting a 138-line module or a 175-line test helper file only
   scatters it, while 21 of 29 on longer files were right. A group that
   holds three quarters or more of the outline's members is not named:
   moving 14 of a file's 15 members, or 9 of its 11 tests, moves the file
   rather than splitting it, so a consider left naming no group is a note
   and a review says to split the whole file.
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
