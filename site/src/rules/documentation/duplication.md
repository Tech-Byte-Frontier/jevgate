# Duplication

{{#include ../../reference/_rules.md:documentation-duplication}}

## When a finding is right

A finding says one section states everything another states, or that two sections give different values or instructions for the same thing. It is right when two documents disagree about a command or a version, or when a copy will drift from the page it repeats. It is wrong when the repetition is what the reader needs where they are: an index entry summarizing the page it links, a pointer to the canonical page, or the template every API reference page follows. Of the 34 findings labeled wrong or debatable so far, 15 were READMEs that repeat the documentation's home page.

A translation is not a duplicate: two documents in different languages are asked only whether they disagree.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: ktor-samples 91c3da3e1e17a489aa75a04f8fcfa294f6ce000541455b8892f571386ebe00a4 -->
### Ktor samples: the sample index

- **Where:** [`README.md:16`](https://github.com/ktorio/ktor-samples/blob/1c9df7cf102d638eadaf545fcce4c0ec5ccad334/README.md#L16) in ktorio/ktor-samples at `1c9df7c`.
- **Finding (consider):** Section `Applications` states everything section `Postgres sample for Ktor Server` of `postgres/README.md` states.
- **Why it was wrong:** The overlap is one sentence: the root README gives each sample a one-line summary and links its README, whose introduction repeats that summary before its own steps. An index entry summarizing the page it links is the point of an index, and each sample's README must stand alone, since each sample is a separate Gradle project.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.

<!-- example: zustand 1c2936eb2a43ef52b3f93a4b2acc2ae3cf709012c6e6988a67f56ba425679fb6 -->
### Zustand: middleware reference pages

- **Where:** [`docs/reference/middlewares/combine.md:40`](https://github.com/pmndrs/zustand/blob/b57db4f86ef179285da216eeb291266da82c361c/docs/reference/middlewares/combine.md#L40) in pmndrs/zustand at `b57db4f`.
- **Finding (consider):** Section `Parameters` states everything section `Parameters` of `docs/reference/middlewares/immer.md` states. The same text recurs in 1 more section.
- **Why it was wrong:** `combine.md`, `immer.md` and `subscribe-with-selector.md` follow the same API reference template, and each documents its own function's parameters. A shared partial would leave each API's page incomplete.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0.
