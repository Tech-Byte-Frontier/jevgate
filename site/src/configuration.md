# Configuration

`jevgate init` writes a commented `jevgate.toml` at the repository root. The command line wins over the file, except that upload patterns and budgets in the file are ceilings that flags can only narrow. Unknown keys are errors. Its first line points editors with TOML schema support (Even Better TOML, Taplo) to [`jevgate.schema.json`](https://github.com/Tech-Byte-Frontier/jevgate/blob/main/jevgate.schema.json), which completes keys, rule names and levels and flags mistakes as you type.

```toml
upload_allow = ["src/**", "tests/**"]   # only these paths may be uploaded
upload_deny = ["**/.env*", "**/*.pem", "**/*.key"]
include_tests = true
max_requests = 300

[rules]                                  # a level per group or rule
maintainability = "review"               # judge every rule of the group, and fail on its reviews
tests = "consider"
security = "mature"                      # opt-in group, enabled by naming it; fails only on levels measured mature
"maintainability/hardcoded-values" = "report"   # judge but never fail; "off" skips it

[[scope]]                                # levels for the files these paths match
paths = ["scripts/**", "tools/**"]
fail_on = ["report"]                     # every rule: judge, never fail
rules = { security = "consider" }        # except these
```

| Key | Default | Meaning |
|---|---|---|
| `upload_allow` | every path | Globs of the paths that may be uploaded, including instruction files and context |
| `upload_deny` | none | Globs never uploaded, even when allowed |
| `generated` | built-in names | Globs of generated files, which are skipped |
| `tests` | built-in conventions | Globs of additional test files |
| `context` | none | Files always sent as related evidence, like `--context` |
| `rules` | the `default` group | A list selects rules. A table gives each group or rule a level: `review`, `consider`, `mature`, `uncertain`, `report` (judge, never fail) or `off`; a level for a group judges every rule of it, opt-in ones included |
| `[[scope]]` | none | `paths` (globs), with `fail_on` for every rule and `rules` for rules or groups, as above; `off` is not accepted (use `upload_deny`). The last scope that matches a file and addresses a rule wins; flags win over scopes |
| `fail_on` | `["mature"]` | The level for rules without their own, like `--fail-on` |
| `include_tests` | `false` | Judge tests, like `--include-tests` |
| `model` | the key's provider's | The model, as the key's provider names it: `jev-1.13.0` for TypeSafe, `typesafe/jev-1.13` for OpenRouter, `typesafe-ai/jev` for Vercel AI Gateway. A pinned version keeps results repeatable; a repository that sets it for one provider needs `--model` with another provider's key |
| `cache_ttl_secs` | `3600` | Cache lifetime for an alias: a model name without an `x.y.z` version, such as `jev-latest` or `jev-1.13`. Pinned versions such as `jev-1.13.0` never expire |
| `max_requests` | unlimited | Ceiling on API attempts per invocation |
| `concurrency` | `6` | Ceiling on simultaneous requests (1–6). Requests also start at least 50 ms apart, TypeSafe's limit of 1,200 a minute |
| `max_file_bytes` | `262144` | Files larger than this are reported as needs-context, never truncated; generated and vendored files are skipped instead |
| `max_context_bytes` | `32768` | Ceiling on context bytes per request |

Rules are named by ID (`maintainability/shared-logic`), key (`shared_logic`) or group (`maintainability`, `tests`, `security`, `documentation`, `default`, `all`). The same names work in `--rule`, `--skip-rule` and `--fail-on TARGET=LEVEL`, and the most specific entry wins.

## What fails the check by default

The default level, `mature`, fails the check only on the rules and levels measured *mature*: their findings were right at least 80% of the time on projects JevGate was never tuned on, over at least 20 findings labeled by hand from the code. Today those are function-simplification reviews (20 of 23 right) and, when the documentation rules run, agent-context considers (22 of 24). Every other finding is reported and marked as still being measured, without failing the check. `jevgate rules` shows each rule's levels that fail by default and how often its reviews and considers were right.

Any level you set replaces the default exactly as it says, for the rules and paths it addresses: `fail_on = ["review"]` (or `--fail-on review`) fails on every review, as releases before 0.26 did; `--fail-on security=consider` sets one group and leaves the others at `mature`; `mature` itself can be set, such as for one group after a stricter `fail_on`. A later release can mark more levels mature as labels accumulate, or fewer; set `fail_on` to keep a fixed policy. Undecided answers never fail the check under `mature`.

A `jevgate.toml` written by `jevgate init` before 0.26 sets `maintainability = "review"` and `tests = "review"`: those lines keep every review of the two groups failing the check, and judge hardcoded values. Delete them for the default rules and gate.
## Keys and where requests go

`jevgate.toml` has no key for the provider or its address: the change under review can edit that file, so it must not be able to send your key elsewhere. The provider follows the key ([Install](install.md) lists the three kinds), and a key goes only to its own provider. `JEVGATE_BASE_URL`, read only from the environment, replaces the provider's API root for a self-hosted proxy or a test server: `https://` to any host, or `http://` only to `localhost`, `127.0.0.1` or `[::1]`. Each check says on stderr when it is set, and `jevgate auth status` checks the key against `<root>/v1/models` there.

The [configuration reference](reference/configuration.md) lists every key with its type, and the rule names and levels it accepts.
