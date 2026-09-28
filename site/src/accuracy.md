# Accuracy

How often JevGate's findings are right, from findings labeled by reading the code on projects JevGate was never tuned on. These numbers decide what fails the check by default: a rule's reviews or considers fail it once at least 80% of them were right on those projects, over at least 20 labels (the `mature` level in [configuration](configuration.md#what-fails-the-check-by-default)).

## By rule and level

{{#include reference/_precision.md}}

Below 20 labels a cell gives the counts without a percentage: a few more labels could move such a share by many points. This is the table this release of JevGate uses; `jevgate rules` prints its unseen shares, and each finding in the reports says how often its rule and level were right. Each rule's page gives what it looks at and findings it got wrong.

## How it is measured

- **The corpus.** Open-source projects of many kinds, from web frameworks and command-line tools to intentionally vulnerable applications, plus the maintainer's own applications, each pinned at a commit. JevGate runs every rule on each of them.
- **Labels.** Each review and consider is labeled by reading the code it points at, and the code around it, by the maintainer or by a coding agent following a written labeling guide. Notes are optional by design and are not labeled. A label is kept by the finding's fingerprint (its rule, file and unit), so it carries over to later versions while the finding stands.
- **What counts as right.** Right: the claim is true of the code, and acting on it is an improvement a competent maintainer of that kind of project would accept; for a consider, true and worth a look is enough. Wrong: the claim is false (the value is bound, the copies do different work, the test checks real behavior), or acting on it would be wrong or pointless there (an idiom the framework requires, generated code, a design documented beside it). Debatable: competent maintainers would disagree. The shares count a debatable label as not right; counted as right of right and wrong, leaving debatable labels out, they would be higher.
- **Unseen and tuned projects.** The unseen projects are 11 held out from the start and 14 added later, none used to tune the rules (listed below). The tuned projects are the other labeled projects, without Bend 2 code, which the table leaves out. Only unseen numbers decide what fails the check.
- **Examples.** The wrong findings on the rule pages come from open-source projects used for tuning only: explaining why an unseen project's findings were wrong would be tuning on it.

## Why tuned numbers are higher

Each release changed questions and composition until wrong findings on the tuned projects went away; the [changelog](changelog.md) records each change with its numbers. A change that removes one project's wrong findings need not carry over to code nobody looked at, which is why the tuned column overstates what a new project sees. The tuned projects also hold 8 of the 9 intentionally vulnerable applications, where security findings are right far more often: on the tuned projects, injection reviews were right 76 of 83 times in those applications and 5 of 13 times in the others.

## Limits

- **Precision only.** The labels say how often a reported finding is right, not what JevGate misses.
- **A snapshot.** The table is measured on one release's findings, joined with labels made on earlier ones: 0.25.0's findings with every rule and tests, replayed from the answer cache, with the shared-logic threshold of 0.28 applied. Each release's changelog says what moved.
- **Other rules beside it.** 0.25.0 asked each rule about a function in a request of its own, as a run of this release does when function simplification is the only rule that judges functions, the default. With hardcoded values or a security rule selected, a function's questions share one request, and a function-simplification finding near a threshold can differ: on 28 labeled projects, its reviews were right 50 times in 55 that way and 49 in 56 apart.
- **Not entirely unseen.** A few changes before 0.22 came from findings on these projects: in 0.20.0, flysystem's copies in deprecated code (8 wrong shared-logic findings); in 0.21.0, Online Boutique's Go modules (9 of 10 copies found between them were wrong) and its connections without TLS (7 reviews), the React Native template's i18next escaping, and the follow-ups for hardcoded-value and injection considers, which the fresh projects' first labels pointed to. Since then, changes are fitted on the tuned projects and only checked on these.
- **Whose projects.** 9 of the 25 unseen projects are the maintainer's own applications, and 23 of the 24 unseen labels of agent-context considers come from them.

## Measure your own

When you accept findings with `jevgate baseline`, `jevgate baseline mark intended|later|wrong PATH:LINE` records whether each was right (meant that way, or to fix later) or wrong, and `jevgate baseline stats` gives each rule's rate of wrong findings among those marked: the same measure, on your own code. A wrong finding reported with the [wrong finding template](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml) is how the rules improve.

## The unseen projects

- **Held out (11):** [starlette](https://github.com/encode/starlette), [koa](https://github.com/koajs/koa), [chi](https://github.com/go-chi/chi), [fd](https://github.com/sharkdp/fd), [flysystem](https://github.com/thephpleague/flysystem), [javapoet](https://github.com/square/javapoet), [DVJA](https://github.com/appsecco/dvja), [Symfony demo](https://github.com/symfony/demo), [gorilla/websocket](https://github.com/gorilla/websocket), [tenacity](https://github.com/jd/tenacity) and [mdBook](https://github.com/rust-lang/mdBook).
- **Fresh (14):** [Online Boutique](https://github.com/GoogleCloudPlatform/microservices-demo), [Refined GitHub](https://github.com/refined-github/refined-github), [a React Native template](https://github.com/obytes/react-native-template-obytes), [Uniswap v2 core](https://github.com/Uniswap/v2-core), [jaffle-shop](https://github.com/dbt-labs/jaffle-shop), and 9 of the maintainer's own applications, which are private.
