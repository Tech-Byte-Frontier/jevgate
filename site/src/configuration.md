# Configuration

`jevgate init` writes a commented `jevgate.toml` at the repository root. The command line wins over the file, except that upload patterns and budgets in the file are ceilings that flags can only narrow. Unknown keys are errors. Its first line points editors with TOML schema support (Even Better TOML, Taplo) to [`jevgate.schema.json`](https://github.com/Tech-Byte-Frontier/jevgate/blob/main/jevgate.schema.json), which completes keys, rule names and levels and flags mistakes as you type.

```toml
upload_allow = ["src/**", "tests/**"]   # only these paths may be uploaded
upload_deny = ["**/.env*", "**/*.pem", "**/*.key"]
include_tests = true
max_requests = 300

[rules]                                  # a level per group or rule
maintainability = "review"               # judge, and fail the gate on review findings
tests = "consider"
security = "consider"                    # opt-in group, enabled by naming it
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
| `rules` | the `default` group | A list selects rules. A table gives each group or rule a level: `review`, `consider`, `uncertain`, `report` (judge, never fail) or `off` |
| `[[scope]]` | none | `paths` (globs), with `fail_on` for every rule and `rules` for rules or groups, as above; `off` is not accepted (use `upload_deny`). The last scope that matches a file and addresses a rule wins; flags win over scopes |
| `fail_on` | `["review"]` | The level for rules without their own, like `--fail-on` |
| `include_tests` | `false` | Judge tests, like `--include-tests` |
| `model` | `jev-1.13.0` | TypeSafe model; a pinned version keeps results repeatable |
| `cache_ttl_secs` | `3600` | Cache lifetime for the `jev-latest` and `jev-preview` aliases; pinned versions never expire |
| `max_requests` | unlimited | Ceiling on API attempts per invocation |
| `concurrency` | `6` | Ceiling on simultaneous requests (1–8) |
| `max_file_bytes` | `262144` | Files larger than this are reported as needs-context, never truncated; generated and vendored files are skipped instead |
| `max_context_bytes` | `32768` | Ceiling on context bytes per request |

Rules are named by ID (`maintainability/shared-logic`), key (`shared_logic`) or group (`maintainability`, `tests`, `security`, `documentation`, `default`, `all`). The same names work in `--rule`, `--skip-rule` and `--fail-on TARGET=LEVEL`, and the most specific entry wins.

The [configuration reference](reference/configuration.md) lists every key with its type, and the rule names and levels it accepts.
