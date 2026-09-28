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

One question per file works too: `.jevgate/questions/no-body-logs.toml` holds the same keys, and its file name is its id.

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

`file` and `hunk` work in any language. Without `paths` they read the source and test files JevGate reads, a Kotlin or Swift file it cannot parse included. With `paths` they also read any other text file the globs name, such as `infra/**/*.tf` or `scripts/*.sh`. Generated files, binary or non-UTF-8 files, hidden paths, files larger than `max_file_bytes` and paths outside `upload_allow` and `upload_deny` are never read. A file skipped for syntax errors stays skipped.

A `hunk` question without `--base`, and a `test` question without `--include-tests`, are not asked, and the check says so on stderr.

## How it is asked

Each request tells Jev what the unit is, as the built-in requests do: the file's path and language, and the unit's literal place in the request, as in "For the function in `functions[2].source`: Does this function write a request body, or a field of one, to a log?". Background and guidance go beside the question as labeled keys. `jevgate check --dry-run --show-requests` prints every request without sending anything.

A unit whose source a built-in question already sends is asked in the same request: a function beside function simplification, a test beside test value, a comment beside the comments rule, an instruction section beside agent context. Its source goes up once. The other units are asked in requests of their own, stage `custom`: functions, comments and sections up to eight to a request, tests and files one to a request, and hunks up to eight of one file. Adding or rewording a question changes the requests it rides in, so their built-in questions are asked again once; after that, both are answered from the cache.

## Findings and the gate

At or above its threshold, an answer is a finding at the question's level, with the probability in the message:

```text
Review (1):
  src/api/orders.ts:41 [custom/no-body-logs] `createOrder`: Does this function write a request body, or a field of one, to a log? Yes (0.93).
    → Log the request id instead of the body.
```

At or below one minus the threshold, the unit is clear. Between the two it is undecided, listed with the question under `--verbose`, and never fails the gate unless you ask for that with `--fail-on custom/no-body-logs=uncertain`.

A question someone wrote and committed is a choice to enforce it, so it fails the gate at its own level: a `review` question on its reviews, a `consider` question on its considers, and a `note` question never. Any configured level replaces that, as for every rule: `--fail-on none` stays advisory, and `fail_on`, `[rules]` and `[[scope]]` apply as they say. Custom questions are named by their rule ID, `custom/<id>`, or all together as `custom`, wherever a rule is: `--rule`, `--skip-rule`, `--fail-on custom=consider`, `[rules]` (`"custom/no-body-logs" = "off"`), `[[scope]]`, `baseline mark --rule` and allow comments. They are in the `default` and `all` groups, so they run whenever the default rules do; a `rules` list or `--rule` that names others needs `custom` too. The id alone is not a name, since it could be a built-in rule's.

```ts
// jevgate: allow(custom/no-body-logs) the audit log keeps redacted bodies by design
function auditOrder(req: Request) {
  audit.log(redact(req.body));
}
```

A finding keeps its fingerprint through unrelated edits, as the built-in rules' do: a function's by its name and code, a hunk's by what it changes and where, a file's by its path.

## Writing a question

Ask about one thing a reader can see in the unit, and put what decides it in `guidance`: what counts as a violation, and what looks like one and is fine. Jev reads the guidance literally, so it decides the answers.

Three questions written from the instruction files of open-source projects, asked of their code:

- gin-realworld's `AGENTS.md` says "Count returns `int64`, handle overflow when converting to `uint`". Asked of its 95 functions whether they convert a GORM `Count` result to `uint` without checking that it fits, the question found `favoritesCount`, which returns `uint(count)`, at 0.98, and cleared 89. It left five undecided, among them `FindManyArticle`, which converts counts to `int`, a type the instruction does not name.
- ky's `AGENTS.md` says "Do not add special handling for `null`". The guidance named the `null` checks the platform forces (`Headers.get()` returns `null`; `typeof value === 'object'` holds for it) as fine. Of 103 functions, none reached 0.80 and 85 were clear; as a `hunk` question over its last 20 commits, 88 of 91 hunks were clear and none was a finding. A change adding a function that gives `null` a meaning of its own failed the gate at 0.92, as a function and as a hunk, and passed once fixed.
- bakerydemo's `AGENTS.md` prefers CSS `light-dark()` and `color-scheme` to a custom theme system. Its guidance named a `[data-theme='dark']` block that sets the palette again as a violation, and the question found exactly that in `main.css`, at 0.94. The project added that block the same day as the instruction, so its authors likely meant it for new theming work: the guidance, not the model, made this finding.

Start a new question as a `note`, or with `--fail-on custom/<id>=report`, run it on the code and on a change that breaks the rule, and raise its level once its findings are right.

## Cost

Beside the built-in questions a unit adds only its question: about 90 tokens for a one-line question, more with background and guidance (about 230 for the one measured below). On its own, a request also carries the unit's source and about 280 tokens of its own. Measured by dry run on eight open-source projects (3,773 functions), one function question with guidance took:

| | Requests | New input tokens | Cost |
|---|---|---|---|
| Alone (`--rule custom`) | 1,560 | 1.48 million | $0.06 |
| Beside the default rules | 889 of its own | 1.83 million once, when it is added | $0.08 |

Beside the default rules, 1,792 functions rode in function-simplification requests; the rest, mostly functions of fewer than five body lines, which the split question skips, were asked on their own. With `--base`, only the changed files are asked, and reruns are answered from the cache for free.

`paths` is the way to keep a question to the code it is about. A question also asks at most 2,000 units a run; the rest are counted as omitted, and the output says how many each question left unasked. `max_requests` still bounds the whole run.

## Where questions live

`.jevgate/questions/` is meant to be committed. JevGate's own `.jevgate/.gitignore` keeps it tracked and the cache ignored. A `.gitignore` entry that ignores `.jevgate/` as a whole hides the questions too, and every check then says so: ignore `/.jevgate/*` and keep `!/.jevgate/questions/` instead.

The question files are read with the repository's own `jevgate.toml`. `--config FILE` reads only that file's `[[question]]` tables, so a pull request cannot edit a question to pass a policy a workflow applies with `--config`.
