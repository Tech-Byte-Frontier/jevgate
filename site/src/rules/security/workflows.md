# Workflows

{{#include ../../reference/_rules.md:security-workflows}}

## When a finding is right

A finding says a GitHub Actions job can run text that people outside the repository write, or runs pull request code while it holds secrets or a write token. It is right for `${{ github.event.pull_request.title }}` inside a `run` script, or a `pull_request_target` job that checks out the pull request's head and runs it with secrets. It is wrong when outside text reaches only an action's input rather than a shell, or when the job runs only code from the base branch. Two findings have been labeled on the corpus, one right and one wrong: too few to measure the rule.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: cookiecutter-django 93985b7129a4c0252cc6a977c75bd859336df64317a24fb1a56fcdc3aa99f2e9 -->
### cookiecutter-django: `align-versions.yml`

- **Where:** [`.github/workflows/align-versions.yml:16`](https://github.com/cookiecutter/cookiecutter-django/blob/1ec1d82fa145375f407b01ccc44ba0a6db7d5ff2/.github/workflows/align-versions.yml#L16) in cookiecutter/cookiecutter-django at `1ec1d82`.
- **Finding (review):** Job `run` places text that people outside the repository write into a `run` script.
- **Why it was wrong:** The job's only script is `uv run ${{ matrix.job.script }}`, whose values are the workflow's own matrix. `${{ github.head_ref }}` appears only as the `ref:` input of `actions/checkout`, not in a shell, and the job runs on `pull_request` for the project's dependency bots or a manual run.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.
