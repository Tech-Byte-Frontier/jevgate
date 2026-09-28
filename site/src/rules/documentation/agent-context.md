# Agent context

{{#include ../../reference/_rules.md:documentation-agent-context}}

## When a finding is right

A finding says a section of an instruction file, which coding agents load at the start of every session, restates what the repository's files show, gives generic advice, repeats what a configured linter checks, or records past work. It is right when an agent would learn the same from the code: the stack, the manifest's commands, a tour of the directories, a changelog kept in `CLAUDE.md`. It is wrong when the section tells agents something the files do not show, or when it is a pointer to a detailed document that costs a few tokens. Most of the findings labeled wrong or debatable were sections so short they cost almost nothing.

A section of fewer than 15 tokens is a note. Agent-context considers are the one documentation level that fails the check by default once these rules run; 23 of their 24 labels on unseen projects come from the maintainer's own repositories.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: shiori 6b92d13ad766484d98e928c4d45c425dd7c3f9ed97aca8cc1f70c13fc022e604 -->
### shiori: `.cursorrules`

- **Where:** [`.cursorrules:3`](https://github.com/go-shiori/shiori/blob/9a9a426acaca0e57e205bf20266a44954aaa8264/.cursorrules#L3) in go-shiori/shiori at `9a9a426`.
- **Finding (consider):** Section `Run the entire test suite` restates what the repository's files show; only lists commands the manifests already show. Cursor and Cline load it at the start of every session (about 4 tokens).
- **Why it was wrong:** The section is one line, `make unittest`, about 4 tokens. The target is in the Makefile, but the line sends agents to the target that adds the race detector and the right build tags instead of a bare `go test`; removing it saves nothing.
- **Since:** a note since 0.21.0, which makes a section of fewer than 15 tokens a note ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: cookiecutter-django 701c9e7239970bac5bdb5a4dd21f5ecc27896b213de1daae869792800ad7dca3 -->
### cookiecutter-django: `What This Project Is`

- **Where:** [`AGENTS.md:5`](https://github.com/cookiecutter/cookiecutter-django/blob/1ec1d82fa145375f407b01ccc44ba0a6db7d5ff2/AGENTS.md#L5) in cookiecutter/cookiecutter-django at `1ec1d82`.
- **Finding (consider):** Section `What This Project Is` only describes the project, which agents read from its files. Codex, GitHub Copilot, Cursor, Windsurf, Cline and Claude Code load it at the start of every session (about 79 tokens).
- **Why it was wrong:** The section says the repository is not a Django application but a Jinja2 template whose `{{cookiecutter.project_slug}}/` files Cookiecutter renders. That framing keeps agents from running Django commands at the root or "fixing" the template tags in `.py` files, and at 79 tokens it is worth keeping.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.
