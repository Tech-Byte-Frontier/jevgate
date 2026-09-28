# Custom questions

A custom question turns a team's convention into a rule. It is a yes/no question whose yes is a violation, asked of every unit it names: each function, test, comment, documentation section, changed hunk, or the whole file. It becomes the rule `custom/<id>`, and its findings are gated, baselined and allowed like any rule's.

```toml
# jevgate.toml
[[question]]
id = "no-body-logs"
question = "Does this function write a request body, or a field of one, to a log?"
background = "Request bodies hold customers' personal data (AGENTS.md: 'Never log request bodies')."
guidance = "Logging the method, path, request id or status is fine. Logging `req.body`, a parsed payload or an object built from it is a violation."
unit = "function"
paths = ["src/api/**"]
level = "review"
next_step = "Log the request id instead of the body."
```

One question per file works too: `.jevgate/questions/no-body-logs.toml` holds the same keys, and its file name is its id. The [question gallery](question-gallery.md) has measured questions to start from, which `jevgate rules add NAME` writes there.

| Key | Default | Meaning |
|---|---|---|
| `id` | the file name, in a question file | Names the rule `custom/<id>`: lowercase letters, digits and single hyphens, starting with a letter, at most 48 characters |
| `question` | required | One yes/no question ending in `?`, at most 300 characters. Yes is a violation |
| `background` | none | Why the rule exists, sent with the question |
| `guidance` | none | How to decide: what counts as a violation and what does not, sent with the question |
| `unit` | required | `function`, `file`, `test`, `section`, `comment` or `hunk`, below |
| `paths` | every file the unit applies to | Globs of the files it applies to |
| `threshold` | `0.8` | The probability of yes at or above which a unit breaks the rule, 0.5 to 0.99 |
| `level` | `review` | The level of its findings: `review`, `consider` or `note` |
| `next_step` | fix it, or allow it with a reason | The action its findings show |

Every mistake is an error that names the question and its file. In `jevgate.toml`, `jevgate.schema.json` (see [Configuration](configuration.md)) also completes and checks the keys as you type. `jevgate rules` lists the questions after the built-in rules.

## Units

| `unit` | Asked about | Needs |
|---|---|---|
| `function` | Each function and method of application code outside tests, of any size | A [supported language](languages.md) |
| `test` | Each test case | `include_tests`, as the test rules do |
| `comment` | Each comment and docstring of application code, with the code it is about | A supported language |
| `section` | Each heading section with text of the agent instruction files and project documentation | |
| `file` | The whole file | |
| `hunk` | Each changed hunk since `--base`, with three lines of context (a new or untracked file is added throughout; a long hunk is asked in parts of 80 lines) | `--base` |

`file` and `hunk` work in any language. Without `paths` they read the source and test files JevGate reads, a Kotlin or Swift file it cannot parse included. With `paths` they also read any other text file the globs name that Git tracks, such as `infra/**/*.tf` or `scripts/*.sh`: `git add` a new one first. An untracked file in a CI workspace can be a credential another step wrote there, such as `google-github-actions/auth`'s `gha-creds-*.json`, which `paths = ["*.json"]` would otherwise send. Generated files, binary or non-UTF-8 files, hidden paths, files larger than `max_file_bytes` and paths outside `upload_allow` and `upload_deny` are never read. A file skipped for syntax errors stays skipped.

A `hunk` question without `--base`, and a `test` question without `--include-tests`, are not asked, and the check says so on stderr.

With `--base`, a check asks only about what the change touched, as it does for the built-in rules: the functions, tests, comments and sections on lines the change added or modified, or removed lines between. A team's rule is often about what the built-in rules leave to the code beside a unit, so a custom question also counts a decorator, attribute or doc comment removed right above a function or test, and the last lines removed from an indented body, as in Python; a function removed beside it, which Git removes with the blank lines after it, does not count. A `file` question asks about every file the change edits, at any line, or moves, and a file moved into a question's `paths` is asked about whole, since the question never read it before. Every hunk is part of the change, including one that only removes lines. A new or untracked file is judged whole, and `--whole-files` asks about every unit of the changed files.

## How it is asked

Each request tells Jev what the unit is, as the built-in requests do: the file's path and language, and the unit's literal place in the request, as in "For the function in `functions[2].source`: Does this function write a request body, or a field of one, to a log?". Background and guidance go beside the question as labeled keys. `jevgate check --dry-run --show-requests` prints every request without sending anything.

A unit whose source a built-in question already sends is asked in the same request: a function beside function simplification, a test beside test value, a comment beside the comments rule, an instruction section beside agent context. Its source goes up once. The other units are asked in requests of their own, stage `custom`: functions, comments and sections up to eight to a request, tests and files one to a request, and hunks up to eight of one file. Each question's answer is cached apart, so adding or rewording a question asks only that question: the requests it rides in are sent again with their units' source and that question alone, and their built-in questions are answered from the cache, including a cache an earlier version wrote.

## Findings and the gate

At or above its threshold, an answer is a finding at the question's level:

```text
Review (1):
  src/api/orders.ts:41 [custom/no-body-logs] (fails the gate) `createOrder`: Does this function write a request body, or a field of one, to a log? Yes. Not yet measured.
    → Log the request id instead of the body.
```

A built-in rule's finding ends with how often findings of its rule and level were right on projects JevGate was never tuned on; a team's own question was labeled on none, so its findings say `Not yet measured.` and carry `precision` of 0 labeled in the JSON report, the MCP results and SARIF, and never a built-in rule's. Its examples, below, are how you measure it. The JSON report keeps each answer's probability (`concern_probability`), and SARIF links a custom question to this page.

At or below one minus the threshold, the unit is clear. Between the two it is undecided, listed with the question under `--verbose`, and never fails the gate unless you ask for that with `--fail-on custom/no-body-logs=uncertain`.

A question someone wrote and committed is a choice to enforce it, so it fails the gate at its own level: a `review` question on its reviews, a `consider` question on its considers, and a `note` question never. JevGate's own rules fail the default gate only once their findings measure right on projects JevGate was never tuned on; a team's question cannot be measured there, so under the default level, `mature`, its own level is what counts (`fail_on_mature` in the JSON report says so), whether `mature` is left as the default or named in `fail_on`, `[rules]` or `[[scope]]`. Any other configured level replaces it, as for every rule: `--fail-on none` stays advisory, `fail_on = ["review"]` fails a `consider` question's findings no more than any rule's considers, and `[rules]` and `[[scope]]` apply as they say. Custom questions are named by their rule ID, `custom/<id>`, or all together as `custom`, wherever a rule is: `--rule`, `--skip-rule`, `--fail-on custom=consider`, `[rules]` (`"custom/no-body-logs" = "off"`), `[[scope]]`, `baseline mark --rule` and allow comments. They are in the `default` and `all` groups, so they run whenever the default rules do; a `rules` list or `--rule` that names others needs `custom` too, and a check, `rules add` and `rules accept` name each question a `rules` list in `jevgate.toml` leaves out. The id alone is not a name, since it could be a built-in rule's.

```ts
// jevgate: allow(custom/no-body-logs) the audit log keeps redacted bodies by design
function auditOrder(req: Request) {
  audit.log(redact(req.body));
}
```

A finding keeps its fingerprint through unrelated edits, as the built-in rules' do: a function's by its name and code, a hunk's by what it changes and where. A file's is its path and text, so a baselined finding of a `file` question covers the file as it was: once the file is edited, the question is asked of it again, as it is of an edited function.

## Writing a question

Ask about one thing a reader can see in the unit, and put what decides it in `guidance`: what counts as a violation, and what looks like one and is fine. Jev reads the guidance literally, so it decides the answers.

Three questions written from the instruction files of open-source projects, asked of their code:

- gin-realworld's `AGENTS.md` says "Count returns `int64`, handle overflow when converting to `uint`". Asked of its 95 functions whether they convert a GORM `Count` result to `uint` without checking that it fits, the question found `favoritesCount`, which returns `uint(count)`, at 0.98, and cleared 89. It left five undecided, among them `FindManyArticle`, which converts counts to `int`, a type the instruction does not name.
- ky's `AGENTS.md` says "Do not add special handling for `null`". The guidance named the `null` checks the platform forces (`Headers.get()` returns `null`; `typeof value === 'object'` holds for it) as fine. Of 103 functions, none reached 0.80 and 85 were clear; as a `hunk` question over its last 20 commits, 88 of 91 hunks were clear and none was a finding. A change adding a function that gives `null` a meaning of its own failed the gate at 0.92, as a function and as a hunk, and passed once fixed.
- bakerydemo's `AGENTS.md` prefers CSS `light-dark()` and `color-scheme` to a custom theme system. Its guidance named a `[data-theme='dark']` block that sets the palette again as a violation, and the question found exactly that in `main.css`, at 0.94. The project added that block the same day as the instruction, so its authors likely meant it for new theming work: the guidance, not the model, made this finding.

Start a new question as a `note`, whose findings are listed after the considers and never fail the gate, or with `--fail-on custom/<id>=report`; run it on the code and on a change that breaks the rule, and raise its level once its findings are right. Give it a failing and a passing example from your own code first, below, and keep `jevgate rules test` passing while you write its guidance.

## Examples and `jevgate rules test`

A question can carry examples: `failing` ones, code that breaks its rule, and `passing` ones, code that keeps it. `jevgate rules test` asks the question about each and fails when it no longer separates them: a failing example whose answer stays below the threshold, which a check would miss, or a passing one at or above it, which a check would report.

```toml
[[question.failing]]
path = "src/api/orders.ts"
code = '''
export function createOrder(req: Request) {
  logger.info("order", req.body);
  return save(req.body);
}
'''

[[question.passing]]
path = "src/api/orders.ts"
code = '''
export function createOrder(req: Request) {
  logger.info("order", { id: req.id });
  return save(req.body);
}
'''

[[question.passing]]
file = ".jevgate/questions/examples/audit.ts"
path = "src/api/audit.ts"
```

In a question file the tables are `[[failing]]` and `[[passing]]`.

| Key | Meaning |
|---|---|
| `code` | The example's text: a file's content, or for a `hunk` question the lines of a diff (`+` added, `-` removed, a space or an empty line unchanged; `@@` headers are optional). Use `code` or `file` |
| `file` | A file holding the example, relative to the repository root |
| `path` | The file the example stands for: its language, and the path Jev reads. Required with `code`; with `file`, the file's own path by default. It must match the question's `paths`, since a check never asks the question elsewhere |

Each example is asked as a check asks a file with that path and text when the question is the only rule selected: its functions, comments, test cases or sections, the whole file, or each hunk of the diff, in the same requests. An example with several units is found when any of them is, and its line shows the one that leans most to yes. A `function`, `comment` or `test` example needs a language JevGate parses, and an example without a unit of the question's kind is an error.

```text
$ jevgate rules test
JevGate: rules test · 1 of 5 examples wrong · 1 question · 0 API requests · 0 input tokens · ~$0.0000 · answered by jev-1.13.0

custom/jev-not-an-llm (comment, review at 0.70, src/**): 1 of 5 examples wrong
  ok     failing 1  yes 0.92  src/transport.rs: a comment in `send`
  ok     failing 2  yes 0.98  src/requests.rs: a comment in `cached`
  wrong  passing 1  yes 0.71  src/main.rs: a comment in `Cli` (a check reports it at 0.70 or more)
  ok     passing 2  yes 0.06  src/output.rs: a comment in `ask`
  ok     passing 3  yes 0.45  src/cache.rs: a comment in `keep`
```

It exits as `check` does: 0 when every example is right, 1 when a question gets one wrong, and 2 when an example could not be asked (no key, a file it cannot read, an example without a unit). `--rule custom/<id>` tests one question. `--format json` prints `complete` and `passed`, the cost as `estimated_usd` (null when unknown, priced as a check prices it), and for each question's examples their `result` (`right`, `wrong` or `error`), `yes`, `found`, `close`, `error` and every unit's answer. `--dry-run` counts the requests and new input tokens without a key or network and still reads every example, so a broken one exits 2 for free.

### Drift

Answers are cached like a check's, so a rerun costs nothing. A new model changes every request, and the examples are asked again: a new `model` pin, JevGate's default moving to a newer version, an alias's answers expiring after `cache_ttl_secs`, or `--model` to try a model before pinning it. So does rewording a question, its background or its guidance. A new threshold or level is judged from the answers already cached. Run `jevgate rules test` in CI next to `check` ([Continuous integration](ci.md)), and a question that stops separating its examples fails there, not in a pull request's findings.

Answers also move a little between asks of the same model. The three questions above, ky's asked of both functions and hunks, and one from JevGate's own instructions (never call Jev an LLM) separated all 23 of their examples, taken or adapted from the projects' code. Asked seven times (four `--refresh` runs and the `jev-latest` and `jev-preview` aliases of jev-1.13.0), their answers moved 0.01 at the median and at most 0.09; one example flipped once, from 0.84 to 0.78 against a threshold of 0.80. So an example closer than 0.10 to its threshold is marked `(within 0.10 of …)`: move it further from the line, or sharpen the guidance until it is.

### Example files

An example file is uploaded, so it is read as a checked file is: inside the repository, not hidden except under `.jevgate/questions/`, not a credential, within `upload_allow` and `upload_deny`, and never through a symbolic link. An `upload_allow` that lists only source directories needs `".jevgate/questions/**"` for examples kept there. Inline code is part of the question and is uploaded with it.

## Cost

When the built-in questions are asked too, as about new or changed code, a unit riding beside them adds only its question: about 90 tokens for a one-line question, more with background and guidance (about 200 for the one measured below). On its own, a request also carries the unit's source and about 280 tokens of its own. Measured by dry run on eight open-source projects (3,773 functions) whose caches answered the default rules, one function question with background and guidance took:

| | Requests | New input tokens | Cost |
|---|---|---|---|
| Alone (`--rule custom`) | 1,560 | 1.58 million | $0.07 |
| Beside the default rules, added to cached code | 889 of its own, and the 849 it rides in sent again with it alone | 1.59 million once | $0.07 |

Beside the default rules, 1,792 functions rode in function-simplification requests; the rest, mostly functions of fewer than five body lines, which the split question skips, were asked on their own. Added to code whose answers are cached, the question was the only one asked, 3,773 times, and none of the 3,137 cached built-in questions was asked again; sent with the functions' source again, it cost about what it costs alone, where asking the built-in questions again too would have taken 1.94 million. Riding saves when the code changes: its source then goes up once for both. With `--base`, only the units a change touched are asked, and reruns are answered from the cache for free.

`jevgate rules test` asks one request per example, or per eight of its units: about 280 tokens beyond the example's text and the question. The 23 examples above took 29,258 input tokens ($0.0012), and each rerun from the cache none.

`paths` is the way to keep a question to the code it is about. A question also asks at most 2,000 units a run; the rest are counted as omitted, and the output says how many each question left unasked. `max_requests` still bounds the whole run.

## Where questions live

`.jevgate/questions/` is meant to be committed. JevGate's own `.jevgate/.gitignore` keeps it tracked and the cache ignored; the one versions before 0.29 wrote, which ignores everything, is rewritten by the next check that is not a dry run. A `.gitignore` entry that ignores `.jevgate/` as a whole hides the questions too. Every command says when Git ignores a question file, which rule does, and how to keep it: for a root entry, ignore `/.jevgate/*` and keep `!/.jevgate/questions/` instead.

The question files are read with the repository's own `jevgate.toml`, once per run: `check --watch` stops when one changes, as it does for `jevgate.toml`, and the MCP server reads them afresh for each call, its rules tool listing them with their definitions. Within an agent's turn, [`jevgate hook`](coding-agents.md) reads them, and the `[[question]]` tables of `jevgate.toml`, as they were when the turn began, question files Git ignores included, so a turn that deletes a question, lowers it to a note or breaks it is still judged by it, and the person is told of the edit (a `question` [guard](output.md#guards)); a `hunk` question there asks about what the turn changed. A text file only a question's `paths` name is read once Git tracks it, in the hook as in a check: an agent's new shell script is asked about after `git add`. `--config FILE` reads only that file's `[[question]]` tables, so a pull request cannot edit a question to pass a policy a workflow applies with `--config`, and the check names the question files it left unread. `--questions DIR` reads question files from `DIR` instead, such as a copy of the base branch's; [Continuous integration](ci.md) has the recipe.

## Proposed from instruction files

Most teams have already written their conventions down for coding agents. `jevgate rules propose` reads the instruction files agents load (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, and Cursor, Copilot, Windsurf, Cline, Kiro, Junie and Roo Code rules) and drafts a question from each line that states one. Name files or directories to read only those; a file named is read whatever its name, such as `jevgate rules propose CONTRIBUTING.md`.

Each list item and paragraph is a candidate line. Code blocks, tables, headings, comments, `@path` imports and the block [`jevgate init --agent`](coding-agents.md) writes between its `<!-- jevgate:begin` and `<!-- jevgate:end -->` markers are left out, and a line ending in a colon that opens a list introduces its items instead of being one. Jev is asked two questions of every line: whether it states a rule for how the code is written that one piece of the code shows, and whether a reviewer would read a function, a test, a comment, a documentation section, a file or a change to check it. Commands, workflow steps, facts about the project, records of past work, how the agent should behave and advice too vague to break are not rules. A line it calls a rule at 0.80 is then asked what would check it, and one that a formatter, linter, compiler or a script measuring lines or coverage checks at 0.80 is left out: a question would repeat that check at a price.

Jev classifies; it writes nothing. Each proposal quotes its line, cites its file and line, and sends its section heading as background and the text that introduces it as guidance:

```toml
# Proposed by `jevgate rules propose` from AGENTS.md:3.
# Jev: a rule to check (0.94), on each function (0.76).
# Edit it, then accept it: jevgate rules accept prefer-undefined-for-absent-values
# It starts as a note, which never fails the gate. A rule quoted alone can answer close to
# the threshold: before raising level to "review", add guidance (what breaks the rule and
# what only looks like it) and a [[failing]] and a [[passing]] example, and run `jevgate rules test --rule custom/prefer-undefined-for-absent-values`.
# jevgate-proposal: 42fabc71fba2
question = 'Does this function break the project rule "Prefer `undefined` for absent values. Do not add special handling for `null`." (AGENTS.md:3)?'
background = 'The rule is from AGENTS.md, section "Conventions".'
unit = "function"
level = "note"
```

Proposals are written to `.jevgate/proposals/<id>.toml`, which Git ignores, never into the configuration. A rule in a nested file, such as `web/CLAUDE.md`, or in a Cursor rule with `globs`, gets those files as its `paths`. To accept one, read it, sharpen it (a `guidance` line saying what breaks the rule and what looks like it but does not), and run `jevgate rules accept <id>`: it checks the file as a question and moves it to `.jevgate/questions/`, where you commit it. Add a failing and a passing example from your code, run `jevgate rules test --rule custom/<id>`, and set its level once they pass; `rules accept` says what a question still lacks. Moving the file by hand works too; `accept` refuses a file that would not load, since a broken question file stops every check. `--format json` also prints every line with its answers, and `--format toml` prints the proposals as `[[question]]` tables to paste into `jevgate.toml` instead of writing them. `--cache-only` and `--max-requests` bound what a run asks, as they do for a check.

Answers are cached, so a second run asks only about lines that changed. It never replaces a proposal file, even one you are editing, and never proposes again a rule that is already a question: the `jevgate-proposal` comment marks both. A proposal you delete is proposed again on the next run; to set one aside, leave its file in `.jevgate/proposals/`, which is never asked. Translated copies of instruction files, under a locale directory such as `docs/i18n/ja/`, are read only when named: OmniRoute keeps its `CLAUDE.md` and `GEMINI.md` in 66 languages.

Measured on the instruction files of six open-source projects never used to write it (ComfyUI, dify, headroom, herdr, multica and rtk; 656 lines), it proposed 271 questions: 197 a reviewer would keep with small edits, 14 wrong (workflow steps, permissions nothing can break, rules that need other files) and 60 debatable: 28 point to another document or to code elsewhere ("follow the Button contract", "reuse the existing helpers"), which one unit cannot show, and 12 are too vague to break. The unit was right for 189 of the 197. Of the 50 lines between 0.65 and 0.80, 6 were rules worth keeping. JevGate's own `AGENTS.md`, release steps and measurement practice, gets no proposal. The run took 155 requests and 572,000 input tokens ($0.024); `--dry-run` prices a run first without a key.

A proposal quotes the rule as written, and a terse rule makes a borderline question. Accepted as proposed, at `review`, ky's rule above answered 0.78 against its threshold of 0.80 on a function that turns a `null` timeout off, so the change passed, and `rules test` found it missing one of six examples from ky's code (0.75). With one line of guidance saying what gives `null` a meaning of its own and which `null` checks the platform forces, the same change failed the gate at 0.89, the fixed function cleared at 0.12, and all six examples were right.
