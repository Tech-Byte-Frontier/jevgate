# Laws (Bend 2)

{{#include ../../reference/_rules.md:tests-laws}}

## When a finding is right

A law is the part of a Bend 2 specification the compiler checks, and its comment the part a person reads. A finding says the comment above a law promises more than, or something other than, what the law states, so a definition could break the promise while every proof passes. It is right when the comment names a property, a case or a condition the law leaves out: bend-json's `remove_sound` checks a one-entry object under "remove deletes key from object". It is wrong when the law states the comment in other words, or when a sibling law states the rest. Of the 27 findings labeled wrong or debatable so far, 15 were laws stating their comment in other words.

The accuracy table leaves Bend 2 projects out, so the rule shows as not measured there. When it shipped in 0.22.0, law findings were right 15 times in 23 on Bend 2 projects never used for tuning.

## Findings it got wrong

Labeled wrong by reading the code, on open-source Bend 2 projects the rules were tuned on.

<!-- example: bend 60f5c8b6e7ab7e32037e743ada682000f9bc96e193d02dc0f8ee678b03a68777 -->
### Bend: `press_reads`

- **Where:** [`demos/app_ray_tracer_3d/LAWS.bend:14`](https://github.com/bendlang/bend/blob/574b6d39a235b539eb19a5c532993a0abb3d11ad/demos/app_ray_tracer_3d/LAWS.bend#L14) in bendlang/bend at `574b6d3`.
- **Finding (consider):** The comment above law `press_reads` promises more than the law states. A definition could break that promise while every proof passes.
- **Why it was wrong:** The law states `Ray.Fly.get(Ray.Fly.set(held, k, v), k) == v` for every list of held keys, slot and value: the comment's "that key reads pressed (or released), for any slot and any held keys" in other words.
- **Since:** not addressed; reported the same way from 0.22.0 through 0.24.1, the latest release run on the Bend 2 projects.

<!-- example: b2-bolt 7b5201d8a928382af8017974038e4834c32256dbb411f37fec026f703194e821 -->
### bolt: `trace_pending_judged`

- **Where:** [`src/rules/LAWS.bend:228`](https://github.com/Emerging-Patterns/bolt/blob/85f175dc80c2d02cb77231d6b010d6c599991df0/src/rules/LAWS.bend#L228) in Emerging-Patterns/bolt at `85f175d`.
- **Finding (review):** The comment above law `trace_pending_judged` promises more than the law states. A definition could break that promise while every proof passes.
- **Why it was wrong:** The law states the comment's main clause exactly: a pending row naming laws is judged as a proved row naming the same laws. The comment's other clauses are stated by sibling laws: `trace_pending_shape` and `trace_proved_shape` right after it, and `trace_counts` further down.
- **Since:** not addressed; reported the same way from 0.22.0 through 0.24.1, the latest release run on the Bend 2 projects.

<!-- example: b2-bolt a1ee5f454f5ed96ca0850381fa314fae0703a074e26f1686302ebfda9771dd71 -->
### bolt: `walk_bfs`

- **Where:** [`src/LAWS.bend:749`](https://github.com/Emerging-Patterns/bolt/blob/85f175dc80c2d02cb77231d6b010d6c599991df0/src/LAWS.bend#L749) in Emerging-Patterns/bolt at `85f175d`.
- **Finding (review):** The comment above law `walk_bfs` promises more than the law states. A definition could break that promise while every proof passes.
- **Why it was wrong:** The law equates the walk's result with `found_in(bfs(…))`, a model written just above it in `LAWS.bend`. Every clause of the comment, the bound on directories read, hidden and `node_modules` directories skipped, nothing past the bound, is a clause of that model, so no walk could break it while the law holds.
- **Since:** not addressed; reported the same way from 0.22.0 through 0.24.1, the latest release run on the Bend 2 projects.
