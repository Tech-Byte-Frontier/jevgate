# Git hooks

JevGate can stop a push, or a commit, whose findings fail the gate: the gate a [pull request check](ci.md) applies, answered from the same cache. A push is the cheaper moment. It runs once for all the commits it sends, and it judges them as committed, not as the working tree holds them after the work went on.

```sh
jevgate init --git-hook pre-push     # before each push: jevgate check --pre-push
jevgate init --git-hook pre-commit   # before each commit: jevgate check --staged
```

`init --git-hook` writes the hook where Git reads it (`.git/hooks/`), never over a hook it did not write, and `--remove` takes out only its own. When another tool keeps the repository's hooks, it says what to add there instead: the [recipes](#recipes) below cover pre-commit, prek, lefthook and husky. The hook it writes lets the push or commit through, with a line saying so, when `jevgate` is not on the PATH, as a Git client started from a desktop can have a shorter PATH than a terminal.

## What each hook judges

- **`check --pre-push`** reads the refs Git passes a pre-push hook. Each pushed commit is compared with the last commit on its first-parent line that a remote already has, as [Qlty](https://github.com/qltysh/qlty)'s pre-push check compares: a branch pushed before with its last push, a new branch with where it leaves the remote's history, and a rebased branch with where it leaves that history now, not with the commits the rebase replaced. A push that merges in commits a remote has, as after `git merge origin/main`, is compared with them merged in as Git merges them, conflict markers and all, so main's changes are not judged as the push's own, and how the push resolved a conflict is. Run on a terminal, it judges what a push of the current branch would send. A branch none of whose history is on a remote, as in a repository's first push, is not checked, and a line says so.
- **`check --staged`** compares the index with HEAD: what the commit records. A file staged in part with `git add -p` is judged as it will be committed, at the lines the commit holds, and untracked files are never judged. Git gives a pre-commit hook the index it commits, which for `git commit -a` or `git commit PATHS` is a temporary one, and `--staged` reads that one. A commit that concludes a merge is compared with the two sides merged as Git merges them, so what it judges is how the conflicts were resolved, not the merged branch's changes. Before the first commit, every staged file is new.

Both judge only what the change touches, as `--base` does, and read the files the change touched from Git; files read only as evidence, such as a copy elsewhere of a changed function, come from disk. The gate is the one configured for every check: by default only the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on stop the push ([what fails by default](configuration.md#what-fails-the-check-by-default)). A stopped push ends with what to do next, for the person and for a coding agent that ran `git push`:

```text
JevGate stopped this push: fix the findings above and push again. A person who judges a finding
acceptable can add a `jevgate: allow(RULE) reason` comment on its line, or run `jevgate baseline --merge`
and commit jevgate-baseline.json. Coding agents: fix the findings; never bypass this check with
--no-verify, an allow comment or the baseline, and if a finding looks wrong, tell the person.
```

## When the check cannot finish

A hook must not trap the change it guards. When the check cannot finish, for want of a key, after an HTTP 402, with a provider that stopped answering, or at a budget, the push or commit goes ahead, and the last lines of the output say it was not checked, and why:

```text
jevgate: this push was not checked: TypeSafe HTTP 402 (credits exhausted; add credits or turn on auto-refill at https://console.typesafe.ai); request was not retried.
jevgate: it goes ahead unchecked, as on_incomplete is "pass"; set on_incomplete = "fail" in jevgate.toml to stop it instead.
```

- **`on_incomplete`** decides: `"pass"` by default with `--staged` and `--pre-push`, `"fail"` (exit 2) for every other check, so CI never passes on partial evidence. Set `on_incomplete = "fail"` in `jevgate.toml`, or pass `--on-incomplete fail`, for a hook that stops the change instead. A configuration that does not load, or a Git failure, lets the change through the same way.
- **Budgets:** a hook's check asks for at most 60 seconds (`max_seconds` or `--max-seconds` changes it). No request starts past it, and an attempt under way gets only the time left, so a provider that stops answering holds a push for a minute at most, where a run of 100 requests otherwise waits out its retries for 8 minutes. The answers received are kept, so the next run asks only for the rest. After a provider failure that passes with time (an overload, a timeout, a dropped connection), the hooks' checks of the next five minutes ask nothing and use only cached answers, as [the agent hook](coding-agents.md#in-the-agents-loop-jevgate-hook) does, so an outage holds one push, not each one. `max_cost` or `--max-cost` stops the asking before the estimated spend passes a number of dollars, and says when 75% and 90% of it are spent.
- **Enter:** on a terminal, a check that has run for a second offers to skip itself. Enter lets the push or commit through, and a line says it was not checked. It is offered only when a person reads the output as it comes: a coding agent's shell reads it through a pipe, and so does pre-commit, which prints a hook's output when the hook ends. lefthook runs a job in a pseudo-terminal of its own, where the offer shows but Enter does not reach it, unless the job has `use_stdin: true`, as the pre-push recipe below does, or `interactive: true`.

## Recipes

The hooks need JevGate 0.31.0 or later on the PATH (or built by pre-commit), and a key: `jevgate auth login` saves one, or set `TYPESAFE_API_KEY`, `OPENROUTER_API_KEY` or `AI_GATEWAY_API_KEY`.

### pre-commit and prek

```yaml
# .pre-commit-config.yaml
default_install_hook_types: [pre-commit, pre-push]
repos:
  - repo: https://github.com/Tech-Byte-Frontier/jevgate
    rev: v0.31.0
    hooks:
      - id: jevgate-push-system   # before each push; jevgate-system before each commit
```

Then `pre-commit install`, or `prek install` with [prek](https://github.com/j178/prek), which reads the same file. The `-system` hooks run the `jevgate` on your PATH; `jevgate-push` and `jevgate` build it with Rust instead. They are `verbose`, so pre-commit prints JevGate's output even when the hook passes, as it must when a check that could not finish lets the change through. pre-commit hides unstaged changes while a commit hook runs, and passes the pushed ref to a push hook in `PRE_COMMIT_TO_REF`; JevGate reads the index, and that ref, either way. Before 0.31, the `jevgate` and `jevgate-system` hooks ran `check --base HEAD`, which judged the working tree and untracked files; they now run `check --staged`, only before a commit.

### lefthook

```yaml
# lefthook.yml
pre-push:
  jobs:
    - name: jevgate
      run: jevgate check --pre-push
      use_stdin: true   # the refs Git passes the hook
```

Before each commit instead, a `pre-commit:` job runs `jevgate check --staged`, without `use_stdin`. `lefthook install` writes the hooks.

### husky

```sh
# .husky/pre-push
jevgate check --pre-push
```

Or `.husky/pre-commit` with `jevgate check --staged`. husky puts `node_modules/.bin` on the PATH, so `npm install --save-dev @tech-byte-frontier/jevgate` gives every contributor the same JevGate.

### Any other hook runner

Run `jevgate check --pre-push` from the pre-push hook, with Git's input on its stdin, or `jevgate check --staged` from the pre-commit hook. The exit code is the gate's: 0 to go ahead, including a check that could not finish when `on_incomplete` passes it, 1 when findings fail the gate, and 2 when the check could not finish and `on_incomplete` is `"fail"`, or its usage is invalid.
