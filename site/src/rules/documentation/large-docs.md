# Large docs

{{#include ../../reference/_rules.md:documentation-large-docs}}

## When a finding is right

A finding says a long document would be easier to find and maintain split by subject, or that it mainly records past work. It is right for a runbook that holds unrelated subjects, or a finished plan kept among the living documents. It is wrong for one long guide or reference written for one reader, such as a contributing guide. More than a third of the findings labeled wrong were plans covering one release, which read as several subjects from their headings.

A document is judged from its headings alone, and a split finding is asked what kind of document it is: a kind that serves one subject clears it.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: debug-toolbar 7a5c4e801e625d6648dd38ac99fd036c50459937d18e0c4d7d5f6ece7dcf4423 -->
### Django Debug Toolbar: `contributing.rst`

- **Where:** [`docs/contributing.rst:1`](https://github.com/django-commons/django-debug-toolbar/blob/dfc69d9b8f15e36c776ec54f50c7b4e2e6082cbb/docs/contributing.rst#L1) in django-commons/django-debug-toolbar at `dfc69d9`.
- **Finding (consider):** `docs/contributing.rst` holds several unrelated subjects.
- **Why it was wrong:** It is a 301-line contributing guide for one reader, the contributor: bug reports, code, architecture, tests, style, patches, translations, releases and building the docs are the usual sections of such a guide. Splitting it would scatter one guide.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.
